from __future__ import annotations

from pathlib import Path
from types import SimpleNamespace
from typing import List

import numpy as np
import pytest
from fastapi.testclient import TestClient

from core.search.retriever import (
    EarlyStopConfig,
    Retriever,
    SessionState,
    VectorIndex,
)
from core.api.app_factory import create_app
from core.api import session as session_registry


@pytest.mark.smoke
def test_session_state_recent_queries_bounded():
    state = SessionState()
    limit = 50
    for idx in range(limit * 2):
        state.add_query(f"q{idx}")
    assert len(state.recent_queries) == limit
    assert state.recent_queries[0] == f"q{limit}"
    assert state.recent_queries[-1] == f"q{limit * 2 - 1}"


@pytest.mark.smoke
def test_early_stop_state_triggers_after_patience():
    config = EarlyStopConfig(score_threshold=0.5, window_size=2, patience=2)
    state = config.create_state(batch_size=4)
    assert not state.observe([0.8, 0.7])
    assert not state.observe([0.3, 0.2])
    assert state.patience_hits == 1
    assert state.observe([0.2, 0.1]) is True


def _synthetic_index(doc_count: int = 256, dim: int = 64) -> VectorIndex:
    rng = np.random.default_rng(123)
    embeddings = rng.normal(size=(doc_count, dim)).astype(np.float32)
    paths = [f"/tmp/doc_{idx}.txt" for idx in range(doc_count)]
    exts = [".txt"] * doc_count
    previews = [""] * doc_count
    owners = ["owner"] * doc_count
    index = VectorIndex()
    index.build(embeddings, paths, exts, previews, owners=owners)
    index.configure_ann(threshold=16, ef_search=128, ef_construction=200, m=32)
    return index


@pytest.mark.full
def test_ann_search_matches_exact_top_k():
    index = _synthetic_index()
    rng = np.random.default_rng(999)
    query = rng.normal(size=index.dimension).astype(np.float32)
    exact = index.search(query, top_k=5, use_ann=False)
    ann = index.search(query, top_k=5, use_ann=True)
    exact_ids = [hit["doc_id"] for hit in exact]
    ann_ids = [hit["doc_id"] for hit in ann]
    assert ann_ids == exact_ids


class _AnnSpyIndex:
    def __init__(self) -> None:
        self.exts = [".txt"] * 10
        self.doc_ids = list(range(10))
        now_hits = []
        for doc_id in self.doc_ids:
            now_hits.append(
                {
                    "doc_id": doc_id,
                    "path": f"doc_{doc_id}.txt",
                    "ext": ".txt",
                    "preview": "",
                    "vector_similarity": 0.9 - (doc_id * 0.01),
                    "lexical_score": 0.0,
                    "score": 0.9 - (doc_id * 0.01),
                }
            )
        self._hits = now_hits
        self.calls: List[bool | None] = []

    def configure_ann(self, **_kwargs) -> None:  # pragma: no cover - side effects not used
        return None

    def search(self, _qvec, top_k: int, oversample: int = 1, **kwargs):
        self.calls.append(kwargs.get("use_ann"))
        fetch = max(1, min(len(self._hits), top_k * max(1, oversample)))
        return self._hits[:fetch]


@pytest.mark.smoke
def test_retriever_passes_ann_flag():
    retr = Retriever.__new__(Retriever)  # type: ignore[misc]
    retr.model_path = Path("dummy_model.joblib")
    retr.corpus_path = Path("dummy_corpus.csv")
    retr.cache_dir = Path("dummy_cache")
    retr.min_similarity = 0.0

    spy_index = _AnnSpyIndex()

    class StubIndexManager:
        def __init__(self, index):
            self._index = index

        def ensure_loaded(self):
            return self._index

        def schedule_rebuild(self, priority: bool = False):
            return None

        def wait_until_ready(self, timeout=None):
            return True

        def get_index(self, wait: bool = False, timeout=None):
            return self._index

        def shutdown(self):  # pragma: no cover - cleanup hook
            return None

    retr.index_manager = StubIndexManager(spy_index)
    retr.search_wait_timeout = 0.0
    retr.encoder = SimpleNamespace(
        encode_query=lambda text: np.ones(32, dtype=np.float32),
    )

    session = SessionState()
    retr.search("query", top_k=3, session=session, use_ann=True)
    retr.search("query", top_k=3, session=session, use_ann=False)
    assert spy_index.calls[-2] is True
    assert spy_index.calls[-1] is False


@pytest.mark.smoke
def test_fastapi_feedback_returns_session_summary():
    session_registry.registry.sessions.clear()

    class StubRetriever:
        def __init__(self) -> None:
            self.last_session = None

        def search(self, query: str, top_k: int = 5, *, session: SessionState | None = None, **_kwargs):
            if session is not None:
                session.add_query(query)
                session.record_like(ext=".pdf", owner="alice")
            return [
                {
                    "doc_id": 1,
                    "path": "doc.pdf",
                    "ext": ".pdf",
                    "vector_similarity": 0.9,
                    "lexical_score": 0.2,
                    "combined_score": 0.85,
                    "match_reasons": [],
                }
            ]

        def ready(self, rebuild: bool = False, *, wait: bool = True) -> bool:  # pragma: no cover - unused
            return True

    settings = SimpleNamespace(STARTUP_LOAD=False)
    retriever = StubRetriever()
    app = create_app(settings=settings, retriever_provider=lambda: retriever)
    app.state.retriever = retriever
    client = TestClient(app)

    session_id = "test-session"
    resp = client.post("/api/search", json={"query": "hello", "top_k": 3, "session_id": session_id})
    assert resp.status_code == 200
    payload = resp.json()
    assert payload["session"]["recent_queries"] == ["hello"]
    assert payload["session"]["preferred_exts"][0] == ".pdf"

    feedback = client.post(
        "/api/feedback",
        json={
            "session_id": session_id,
            "doc_id": 1,
            "ext": ".pdf",
            "owner": "alice",
            "action": "like",
        },
    )
    assert feedback.status_code == 200
    summary = feedback.json()["session"]
    assert ".pdf" in summary["preferred_exts"]
    assert "alice" in summary["owner_prior"]
