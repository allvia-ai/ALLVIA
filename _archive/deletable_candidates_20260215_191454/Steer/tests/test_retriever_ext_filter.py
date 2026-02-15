import sys
import time
from pathlib import Path
from types import SimpleNamespace
import types

from core.search import retriever

if "numpy" not in sys.modules:
    np_stub = types.ModuleType("numpy")
    np_stub.ndarray = list
    np_stub.float32 = float

    def _unsupported(*_args, **_kwargs):  # pragma: no cover - 테스트 환경 안전장치
        raise NotImplementedError("numpy 기능은 테스트 더블로 대체됩니다.")

    np_stub.array = _unsupported
    np_stub.stack = _unsupported
    np_stub.linalg = types.SimpleNamespace(norm=_unsupported)
    np_stub.argpartition = _unsupported
    np_stub.argsort = _unsupported
    np_stub.save = _unsupported
    np_stub.load = _unsupported
    np_stub.dot = _unsupported
    sys.modules["numpy"] = np_stub

try:
    import pytest
except ModuleNotFoundError:  # pytest 미설치 환경 대비
    class _Mark:
        def smoke(self, func):
            return func

        def parametrize(self, _argnames, argvalues):
            def decorator(func):
                def wrapper():
                    for params in argvalues:
                        if not isinstance(params, tuple):
                            params = (params,)
                        func(*params)
                return wrapper
            return decorator

    pytest = types.SimpleNamespace(mark=_Mark())

from core.search.retriever import (
    Retriever,
    SessionState,
    VectorIndex,
    _metadata_text,
    _similarity_to_percent,
    _pick_rerank_device,
    _prioritize_ext_hits,
)


def _make_stub_retriever() -> Retriever:
    retr = Retriever.__new__(Retriever)  # type: ignore
    retr.model_path = Path("dummy_model.joblib")
    retr.corpus_path = Path("dummy_corpus.csv")
    retr.cache_dir = Path("dummy_cache")
    retr.min_similarity = 0.0
    def _encode_query(query: str):
        retr._last_query = query
        return query

    retr.encoder = SimpleNamespace(
        encode_query=_encode_query,
        encode_docs=lambda docs: docs,
    )

    class SimpleIndex:
        def __init__(self):
            self.exts = [
                ".xlsx",
                ".pdf",
                ".pdf",
                ".xlsm",
                ".docx",
                ".pptx",
                ".doc",
                ".hwp",
            ]
            now = time.time()
            year = 365 * 24 * 3600
            self._hits = [
                {"path": "xlsx_doc", "ext": ".xlsx", "similarity": 0.99, "preview": "", "mtime": now - (0.1 * year), "size": 5_000},
                {"path": "pdf_doc_one", "ext": ".pdf", "similarity": 0.95, "preview": "", "mtime": now - (0.2 * year), "size": 20_000, "owner": "alice"},
                {"path": "pdf_doc_two", "ext": ".pdf", "similarity": 0.94, "preview": "", "mtime": now - (1.1 * year), "size": 18_000, "owner": "bob"},
                {"path": "xlsm_budget", "ext": ".xlsm", "similarity": 0.93, "preview": "", "mtime": now - (2.0 * year), "size": 35_000, "owner": "finance"},
                {"path": "docx_doc", "ext": ".docx", "similarity": 0.92, "preview": "", "mtime": now - (3.1 * year), "size": 12_000, "owner": "alice"},
                {"path": "pptx_deck", "ext": ".pptx", "similarity": 0.91, "preview": "", "mtime": now - (4.5 * year), "size": 8_000, "owner": "design"},
                {"path": "doc_report", "ext": ".doc", "similarity": 0.90, "preview": "", "mtime": now - (5.0 * year), "size": 16_000, "owner": "legal"},
                {"path": "hwp_contract", "ext": ".hwp", "similarity": 0.89, "preview": "", "mtime": now - (6.0 * year), "size": 14_000, "owner": "legal"},
            ]

        def search(self, _qvec, top_k: int, oversample: int = 1, **_ignored):
            fetch = max(1, min(len(self._hits), top_k * max(1, oversample)))
            return self._hits[:fetch]

    simple_index = SimpleIndex()

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

    retr.index_manager = StubIndexManager(simple_index)
    retr.index = simple_index  # 편의상 노출 (기존 테스트 호환)
    retr.search_wait_timeout = 0.0
    retr._result_cache = None
    return retr


@pytest.mark.smoke
def test_pdf_extension_query_prioritises_pdf_hits():
    retr = _make_stub_retriever()
    hits = retr.search("확장자가 pdf인 파일 어떤것있어", top_k=3)
    assert len(hits) == 3
    assert [h["ext"] for h in hits[:2]] == [".pdf", ".pdf"]
    assert hits[0]["path"] == "pdf_doc_one"
    breakdown = hits[0].get("score_breakdown", {})
    assert "vector" in breakdown
    assert "extension_bonus" in breakdown
    assert any("확장자" in reason for reason in hits[0].get("match_reasons", []))


@pytest.mark.smoke
def test_query_without_extension_keeps_original_order():
    retr = _make_stub_retriever()
    hits = retr.search("파일 어떤것있어", top_k=3)
    assert len(hits) == 3
    assert hits[0]["ext"] == ".xlsx"


@pytest.mark.smoke
def test_semantic_query_expansion_hits_expected_extensions():
    retr = _make_stub_retriever()
    hits = retr.search("계약서 보여줘", top_k=2)
    expanded_query = getattr(retr, "_last_query", "")
    assert "contract" in expanded_query or "agreement" in expanded_query
    assert hits[0]["ext"] in {".pdf", ".hwp"}
    assert "matched_synonyms" in hits[0]


@pytest.mark.full
@pytest.mark.parametrize(
    "query, expected_exts",
    [
        ("파워 포인트 자료 있어?", {".ppt", ".pptx"}),
        ("ms word 보고서 찾고싶어", {".doc", ".docx"}),
        ("spreadsheet 공유해줘", {".xls", ".xlsx", ".xlsm", ".xlsb", ".xltx"}),
        ("계약서 파일 보여줘", {".pdf", ".hwp"}),
        ("예산 자료 어디 있어?", {".xlsx", ".xls", ".xlsm"}),
    ],
)
def test_extension_synonym_queries_prioritise_expected_type(query, expected_exts):
    retr = _make_stub_retriever()
    hits = retr.search(query, top_k=2)
    assert hits, "검색 결과가 비어있습니다."
    assert hits[0]["ext"] in expected_exts


@pytest.mark.smoke
def test_metadata_text_includes_extension_context_keywords():
    metadata = _metadata_text("팀/분기_계획.pptx", ".pptx", "driveA")
    assert "pptx" in metadata
    assert "파워포인트" in metadata
    assert "presentation" in metadata


@pytest.mark.smoke
def test_filename_overlap_boosts_results_without_explicit_extension():
    retr = _make_stub_retriever()
    retr.index._hits = [
        {"path": "random_notes.txt", "ext": ".txt", "similarity": 0.99, "preview": ""},
        {"path": "연간 계획.docx", "ext": ".docx", "similarity": 0.88, "preview": ""},
        {"path": "yearly_budget.xlsx", "ext": ".xlsx", "similarity": 0.85, "preview": ""},
    ]
    retr.index.exts = [hit["ext"] for hit in retr.index._hits]

    hits = retr.search("연간 계획 자료", top_k=2)
    assert any(hit["path"] == "연간 계획.docx" for hit in hits)
    assert hits[0]["score"] >= hits[1]["score"]
    doc_hit = next(hit for hit in hits if hit["path"] == "연간 계획.docx")
    assert "질문 키워드" in " ".join(doc_hit.get("match_reasons", []))


@pytest.mark.smoke
def test_metadata_filters_recognise_relative_year():
    retr = _make_stub_retriever()
    now = time.time()
    # Make only one document fall within 3 years ago window
    retr.index._hits[4]["mtime"] = now - (3 * 365 * 24 * 3600)
    retr.index._hits[5]["mtime"] = now - (6 * 365 * 24 * 3600)
    hits = retr.search("3년 전 작성한 자료", top_k=3)
    assert hits and hits[0]["path"] == "docx_doc"


@pytest.mark.smoke
def test_session_extension_preference_increases_bonus():
    retr = _make_stub_retriever()
    session = SessionState()
    session.preferred_exts[".docx"] = 1.0
    hits = retr.search("파일 어떤것있어", top_k=5, session=session)
    docx_hit = next(hit for hit in hits if hit["ext"] == ".docx")
    assert docx_hit["score_breakdown"].get("session_ext", 0.0) > 0
    assert any("세션 선호 확장자" in reason for reason in docx_hit.get("match_reasons", []))


@pytest.mark.smoke
def test_session_owner_preference_increases_bonus():
    retr = _make_stub_retriever()
    session = SessionState()
    session.owner_prior["alice"] = 1.0
    hits = retr.search("파일 어떤것있어", top_k=5, session=session)
    owner_hit = next(hit for hit in hits if hit.get("owner") == "alice")
    assert owner_hit["score_breakdown"].get("session_owner", 0.0) > 0
    assert any("세션 선호 작성자" in reason for reason in owner_hit.get("match_reasons", []))


@pytest.mark.full
def test_refresh_if_cache_changed_handles_missing_signature():
    retr = _make_stub_retriever()
    if hasattr(retr, "_cache_signature"):
        delattr(retr, "_cache_signature")
    retr._refresh_if_cache_changed()
    assert hasattr(retr, "_cache_signature")


@pytest.mark.full
def test_prioritize_ext_hits_pushes_desired_extensions_forward():
    hits = [
        {"path": "doc_high", "ext": ".docx", "score": 0.99},
        {"path": "pdf_one", "ext": ".pdf", "score": 0.95},
        {"path": "pdf_two", "ext": ".pdf", "score": 0.94},
        {"path": "pdf_three", "ext": ".pdf", "score": 0.93},
        {"path": "pdf_four", "ext": ".pdf", "score": 0.92},
    ]
    prioritized = _prioritize_ext_hits(hits, desired_exts={".pdf"}, top_k=5)
    assert all(hit["ext"] == ".pdf" for hit in prioritized[:4])
    assert any(hit["ext"] == ".docx" for hit in prioritized)


@pytest.mark.full
def test_similarity_to_percent_clamps_range():
    assert _similarity_to_percent(0.873) == "87.3%"
    assert _similarity_to_percent(1.4) == "100.0%"
    assert _similarity_to_percent(-0.2) == "0.0%"
    assert _similarity_to_percent("oops") == "-"


@pytest.mark.full
def test_pick_rerank_device_prefers_explicit_value():
    assert _pick_rerank_device("cuda:1") == "cuda:1"


@pytest.mark.full
def test_pick_rerank_device_uses_cuda_when_available():
    original_torch = retriever.torch

    class _CudaTorch:
        class cuda:  # type: ignore
            @staticmethod
            def is_available() -> bool:
                return True

    retriever.torch = _CudaTorch()
    try:
        assert _pick_rerank_device(None) == "cuda"
    finally:
        retriever.torch = original_torch


@pytest.mark.full
def test_pick_rerank_device_falls_back_to_cpu():
    original_torch = retriever.torch

    class _CpuTorch:
        class cuda:  # type: ignore
            @staticmethod
            def is_available() -> bool:
                return False

    retriever.torch = _CpuTorch()
    try:
        assert _pick_rerank_device(None) == "cpu"
    finally:
        retriever.torch = original_torch


@pytest.mark.full
def test_vector_index_ann_search_roundtrip():
    if retriever.faiss is None:
        pytest.skip("FAISS not available")
    import numpy as np

    index = VectorIndex()
    embeddings = np.eye(10, dtype=np.float32)
    paths = [f"doc_{i}" for i in range(10)]
    exts = [".txt"] * 10
    previews = ["preview"] * 10
    index.configure_ann(threshold=5, m=16, ef_construction=40)
    index.build(
        embeddings,
        paths,
        exts,
        previews,
        sizes=[0] * 10,
        mtimes=[0.0] * 10,
        ctimes=[0.0] * 10,
        owners=["tester"] * 10,
    )
    index.configure_ann(ef_search=12)
    query = embeddings[0]
    hits = index.search(query, top_k=3)
    assert hits
    assert hits[0]["path"] == "doc_0"
