"""Entry point for executing the knowledge agent (InfoPilot) in desktop UI.

The meeting desktop UI invokes this script with JSON-safe output requirements,
so the script captures stdout from downstream components and returns a single
JSON object describing success or failure.
"""
from __future__ import annotations

import argparse
import contextlib
import io
import json
import sys
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Dict, Iterable, List, Optional, TYPE_CHECKING

REPO_ROOT = Path(__file__).resolve().parents[1]
if str(REPO_ROOT) not in sys.path:
    sys.path.insert(0, str(REPO_ROOT))

from scripts.audit_log import record_event

if TYPE_CHECKING:  # pragma: no cover
    from core.conversation.lnp_chat import LNPChat
    from core.data_pipeline.policies.engine import PolicyEngine

KNOWLEDGE_AGENT = "knowledge_search"
DEFAULT_MODEL = Path("data/topic_model.joblib")
DEFAULT_CORPUS = Path("data/corpus.parquet")
DEFAULT_CACHE = Path("data/cache")


@dataclass
class FolderInfo:
    label: str
    path: Optional[Path]
    scope: str
    policy_path: Optional[Path]


def _parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Run knowledge agent query")
    parser.add_argument("--query", required=True, help="User query text")
    parser.add_argument("--folder-path", default="", help="Smart folder absolute path")
    parser.add_argument("--folder-label", default="", help="Display label for folder")
    parser.add_argument("--folder-scope", default="auto", help="Scope value from UI")
    parser.add_argument("--policy-path", default="", help="Optional smart folder policy path")
    parser.add_argument("--model", default=str(DEFAULT_MODEL), help="Topic model path")
    parser.add_argument("--corpus", default=str(DEFAULT_CORPUS), help="Corpus parquet path")
    parser.add_argument("--cache", default=str(DEFAULT_CACHE), help="Vector cache directory")
    parser.add_argument("--topk", type=int, default=5, help="Maximum number of hits to return")
    parser.add_argument("--translate", action="store_true", help="Enable query translation")
    parser.add_argument("--no-translate", dest="translate", action="store_false", help="Disable query translation")
    parser.set_defaults(translate=False)
    return parser.parse_args()


def _resolve_path(value: str) -> Optional[Path]:
    value = (value or "").strip()
    if not value:
        return None
    path = Path(value).expanduser()
    try:
        return path.resolve()
    except OSError:
        return path


def _load_policy_engine(policy_path: Optional[Path]) -> PolicyEngine:
    try:
        from core.data_pipeline.policies.engine import PolicyEngine
    except ModuleNotFoundError as exc:
        raise RuntimeError(
            "Smart folder policy support requires extra dependencies. "
            "Run inside the project env (e.g. `conda run -n ai-summary python3 ...`) "
            "or install missing packages (e.g. `pip install jsonschema`)."
        ) from exc

    if not policy_path:
        return PolicyEngine.empty()
    try:
        return PolicyEngine.from_file(policy_path)
    except Exception as exc:  # noqa: BLE001 - propagate to stderr later
        raise RuntimeError(f"정책 파일을 불러오지 못했습니다: {exc}") from exc


def _policy_scope(folder_scope: str, has_policy: bool) -> str:
    normalized = (folder_scope or "auto").strip().lower()
    if normalized in {"global"}:
        return "global"
    if normalized in {"policy", "local", "folder", "scoped"}:
        return "policy"
    if has_policy:
        return "policy"
    return "auto"


def _filter_hits_by_folder(hits: Iterable[Dict[str, Any]], folder_path: Optional[Path]) -> List[Dict[str, Any]]:
    if not folder_path:
        return [dict(hit) for hit in hits]
    allowed_root = folder_path
    filtered: List[Dict[str, Any]] = []
    for hit in hits:
        raw_path = hit.get("path")
        if not raw_path:
            continue
        try:
            resolved = Path(str(raw_path)).expanduser().resolve()
        except Exception:
            continue
        try:
            if resolved == allowed_root or resolved.is_relative_to(allowed_root):
                filtered.append(dict(hit))
        except AttributeError:  # Python <3.9 fallback (should not happen here)
            resolved_parts = resolved.parts
            allowed_parts = allowed_root.parts
            if len(resolved_parts) >= len(allowed_parts) and resolved_parts[: len(allowed_parts)] == allowed_parts:
                filtered.append(dict(hit))
    return filtered


def _normalize_items(hits: List[Dict[str, Any]]) -> List[Dict[str, Any]]:
    items: List[Dict[str, Any]] = []
    for hit in hits:
        path_value = str(hit.get("path") or "").strip()
        title = hit.get("title")
        if not title:
            title = Path(path_value).name if path_value else "검색 결과"
        snippet = hit.get("preview") or hit.get("snippet") or ""
        score = (
            hit.get("similarity")
            or hit.get("vector_similarity")
            or hit.get("rerank_score")
            or hit.get("score")
        )
        items.append(
            {
                "title": title,
                "snippet": snippet,
                "path": path_value,
                "score": score,
                "ext": hit.get("ext"),
            }
        )
    return items


def _emit(payload: Dict[str, Any], *, exit_code: int = 0) -> None:
    sys.stdout.write(json.dumps(payload, ensure_ascii=False))
    sys.stdout.write("\n")
    sys.stdout.flush()
    raise SystemExit(exit_code)


def main() -> None:
    args = _parse_args()

    folder = FolderInfo(
        label=args.folder_label or "선택된 폴더",
        path=_resolve_path(args.folder_path),
        scope=args.folder_scope or "auto",
        policy_path=_resolve_path(args.policy_path),
    )

    model_path = _resolve_path(args.model) or DEFAULT_MODEL.resolve()
    corpus_path = _resolve_path(args.corpus) or DEFAULT_CORPUS.resolve()
    cache_dir = _resolve_path(args.cache) or DEFAULT_CACHE.resolve()

    for required_path in (model_path, corpus_path):
        if not required_path.exists():
            record_event(
                agent="knowledge",
                event="resource_check",
                status="error",
                details={"resource": str(required_path)},
            )
            _emit(
                {
                    "ok": False,
                    "error": f"필수 자원({required_path})을 찾을 수 없습니다.",
                    "data": None,
                },
                exit_code=2,
            )

    try:
        policy_engine = _load_policy_engine(folder.policy_path)
    except Exception as exc:  # noqa: BLE001
        record_event(
            agent="knowledge",
            event="policy_load",
            status="error",
            details={
                "reason": str(exc),
                "policy_path": str(folder.policy_path) if folder.policy_path else None,
            },
        )
        _emit({"ok": False, "error": str(exc), "data": None}, exit_code=3)

    scope_value = _policy_scope(folder.scope, policy_engine.has_policies if policy_engine else False)

    try:
        from core.conversation.lnp_chat import LNPChat
    except ModuleNotFoundError as exc:
        _emit(
            {
                "ok": False,
                "error": (
                    "Knowledge agent requires project dependencies. "
                    "Run inside the project env (e.g. `conda run -n ai-summary python3 ...`). "
                    f"(detail: {exc})"
                ),
                "data": None,
            },
            exit_code=5,
        )

    chat = LNPChat(
        model_path=model_path,
        corpus_path=corpus_path,
        cache_dir=cache_dir,
        topk=args.topk,
        translate=args.translate,
        policy_engine=policy_engine,
        policy_scope=scope_value,
        policy_agent=KNOWLEDGE_AGENT,
    )

    captured = io.StringIO()
    start = time.perf_counter()
    try:
        with contextlib.redirect_stdout(captured):
            chat.ready(rebuild=False)
            response = chat.ask(args.query, topk=args.topk)
    except Exception as exc:  # noqa: BLE001
        logs = captured.getvalue()
        if logs:
            print(logs, file=sys.stderr)
        record_event(
            agent="knowledge",
            event="run",
            status="error",
            details={
                "reason": str(exc),
                "query": args.query,
                "folder_path": str(folder.path) if folder.path else None,
            },
        )
        _emit({"ok": False, "error": f"지식 검색 실패: {exc}", "data": None}, exit_code=4)
    elapsed = time.perf_counter() - start

    logs = captured.getvalue()
    if logs:
        print(logs, file=sys.stderr)

    hits = response.get("hits") or []
    filtered_hits = _filter_hits_by_folder(hits, folder.path)
    items = _normalize_items(filtered_hits)

    payload = {
        "ok": True,
        "data": {
            "query": args.query,
            "folder": {
                "label": folder.label,
                "path": str(folder.path) if folder.path else "",
            },
            "items": items,
            "suggestions": response.get("suggestions") or [],
            "answer": response.get("answer"),
            "stats": {
                "total_hits": len(hits),
                "filtered_hits": len(filtered_hits),
                "elapsed_seconds": elapsed,
                "policy_scope": scope_value,
                "policy_applied": bool(chat._policy_effective),  # type: ignore[attr-defined]
            },
        },
    }

    record_event(
        agent="knowledge",
        event="run",
        status="success",
        details={
            "query": args.query,
            "folder_path": str(folder.path) if folder.path else None,
            "total_hits": len(hits),
            "filtered_hits": len(filtered_hits),
            "elapsed_seconds": elapsed,
            "policy_scope": scope_value,
        },
    )
    _emit(payload, exit_code=0)


if __name__ == "__main__":
    main()
