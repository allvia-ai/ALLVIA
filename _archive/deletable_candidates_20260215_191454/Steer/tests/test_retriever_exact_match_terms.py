from __future__ import annotations

import pytest

from core.search.retriever import _extract_exact_query_terms, _rerank_hits


@pytest.mark.smoke
def test_extract_exact_query_terms_handles_korean_name() -> None:
    assert _extract_exact_query_terms("가나다 관련문서찾아줘") == ["가나다"]
    assert _extract_exact_query_terms("가나다 관련 문서 찾아줘") == ["가나다"]
    assert _extract_exact_query_terms("  가나다  ") == ["가나다"]
    assert _extract_exact_query_terms("가나다 이력서 찾아줘") == ["가나다"]


@pytest.mark.smoke
def test_rerank_boosts_exact_term_matches() -> None:
    hits = [
        {"path": "/tmp/irrelevant.pdf", "preview": "청년정책 보고서", "vector_similarity": 0.99},
        {"path": "/tmp/cert.pdf", "preview": "교육참가확인증 가나다", "vector_similarity": 0.90},
    ]
    ranked = _rerank_hits(
        "가나다 이력서 찾아줘",
        "가나다 이력서 찾아줘",
        hits,
        desired_exts=set(),
        top_k=5,
        session=None,
    )
    assert ranked[0]["path"] == "/tmp/cert.pdf"
    assert (ranked[0].get("exact_terms_matched") or []) == ["가나다"]
