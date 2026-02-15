"""FastAPI application factory compatible with the historical test suite."""
from __future__ import annotations

import logging
from contextlib import asynccontextmanager
from typing import Callable, Iterable, List, Sequence

from fastapi import Depends, FastAPI, HTTPException, Request

from . import schemas
from .session import registry
from .settings import Settings

logger = logging.getLogger(__name__)


def _build_session_summary(state) -> schemas.SessionSummary:
    recent = list(getattr(state, "recent_queries", []))[-10:]
    preferred_exts = [
        ext
        for ext, weight in sorted(
            getattr(state, "preferred_exts", {}).items(), key=lambda item: item[1], reverse=True
        )
        if weight > 0
    ][:5]
    owner_prior = [
        owner
        for owner, weight in sorted(
            getattr(state, "owner_prior", {}).items(), key=lambda item: item[1], reverse=True
        )
        if weight > 0
    ][:5]
    return schemas.SessionSummary(
        recent_queries=recent,
        preferred_exts=preferred_exts,
        owner_prior=owner_prior,
    )


def _fallback_answer(query: str, hits: Sequence[dict]) -> str:
    if not hits:
        return f"'{query}'에 대한 관련 문서를 찾지 못했습니다."
    top = hits[0]
    path = str(top.get("path") or top.get("doc_id") or "결과")
    return f"'{query}' 검색 결과 상위 문서: {path}"


def _extract_match_reasons(hits: Iterable[dict]) -> List[List[str]]:
    return [list(hit.get("match_reasons", []) or []) for hit in hits]


def create_app(*, settings: Settings, retriever_provider: Callable[[], object]) -> FastAPI:
    """Create a lightweight FastAPI app backed by the existing retriever."""

    @asynccontextmanager
    async def lifespan(app: FastAPI):
        app.state.retriever = None
        if settings.STARTUP_LOAD:
            try:
                retriever = retriever_provider()
                ready = getattr(retriever, "ready", None)
                if callable(ready):
                    try:
                        ready(wait=False)
                    except TypeError:
                        ready()
                app.state.retriever = retriever
                logger.info("retriever initialised during startup")
            except Exception:  # pragma: no cover - defensive initialisation
                logger.exception("failed to initialise retriever during startup")
        yield
        retriever = getattr(app.state, "retriever", None)
        shutdown = getattr(retriever, "shutdown", None)
        if callable(shutdown):
            try:
                shutdown()
            except Exception:  # pragma: no cover - defensive cleanup
                logger.exception("failed to shutdown retriever cleanly")

    app = FastAPI(title="AI Summary Backend", lifespan=lifespan)

    def _ensure_retriever(request: Request):
        retriever = getattr(request.app.state, "retriever", None)
        if retriever is None:
            try:
                retriever = retriever_provider()
                request.app.state.retriever = retriever
            except Exception as exc:  # pragma: no cover - defensive path
                raise HTTPException(status_code=503, detail="Retriever not initialised") from exc
        return retriever

    @app.post("/api/search", response_model=schemas.SearchResponse)
    def search(req: schemas.SearchRequest, retr=Depends(_ensure_retriever)) -> schemas.SearchResponse:
        session_id, session_state = registry.get_or_create(req.session_id)
        recent_before = list(getattr(session_state, "recent_queries", []))
        if hasattr(session_state, "record_user_message"):
            session_state.record_user_message(req.query)
        hits = retr.search(req.query, top_k=req.top_k, session=session_state)
        if (
            hasattr(session_state, "add_query")
            and list(getattr(session_state, "recent_queries", [])) == recent_before
        ):
            session_state.add_query(req.query)
        answer = _fallback_answer(req.query, hits)
        if answer and hasattr(session_state, "record_assistant_message"):
            session_state.record_assistant_message(answer)
        summary = _build_session_summary(session_state)
        history_fn = getattr(session_state, "get_chat_history", None)
        if callable(history_fn):
            history_source = history_fn()
        else:
            history_source = []
        history_payload = [
            schemas.ChatMessage(role=role, text=text)
            for role, text in history_source
        ]
        return schemas.SearchResponse(
            session_id=session_id,
            results=list(hits),
            explain=_extract_match_reasons(hits),
            session=summary,
            answer=answer,
            answer_source="fallback" if answer else "none",
            history=history_payload,
            llm_error=None,
        )

    @app.post("/api/feedback", response_model=schemas.FeedbackResponse)
    def feedback(req: schemas.FeedbackRequest) -> schemas.FeedbackResponse:
        session_id, session_state = registry.get_or_create(req.session_id)
        action = req.action
        if action == "click" and hasattr(session_state, "record_click"):
            session_state.record_click(doc_id=req.doc_id, ext=req.ext, owner=req.owner)
        elif action == "pin" and hasattr(session_state, "record_pin"):
            session_state.record_pin(doc_id=req.doc_id, ext=req.ext, owner=req.owner)
        elif action == "like" and hasattr(session_state, "record_like"):
            session_state.record_like(ext=req.ext, owner=req.owner)
        elif action == "dislike" and hasattr(session_state, "record_dislike"):
            session_state.record_dislike(ext=req.ext, owner=req.owner)
        else:
            logger.warning("unsupported feedback action: %s", action)
        summary = _build_session_summary(session_state)
        return schemas.FeedbackResponse(session_id=session_id, status="ok", session=summary)

    @app.post("/api/session/reset", response_model=schemas.SessionResetResponse)
    def reset(req: schemas.FeedbackRequest) -> schemas.SessionResetResponse:
        session_id, session_state = registry.reset(req.session_id)
        summary = _build_session_summary(session_state)
        return schemas.SessionResetResponse(session_id=session_id, recent_queries=summary.recent_queries)

    @app.post("/api/reindex", response_model=schemas.ReindexResponse)
    def reindex(req: schemas.ReindexRequest, retr=Depends(_ensure_retriever)) -> schemas.ReindexResponse:
        ready = getattr(retr, "ready", None)
        if callable(ready):
            try:
                ready(rebuild=req.force, wait=False)
            except TypeError:
                ready()
        return schemas.ReindexResponse(status="scheduled")

    return app
