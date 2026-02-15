from __future__ import annotations

from core.agents.rag.citation import (
    build_allowed_refs,
    ensure_citations_or_append_sources,
    format_ref_id,
)


def test_format_ref_id_uses_basename_and_chunk() -> None:
    assert format_ref_id("/Users/me/docs/report.md", 12) == "report#12"


def test_ensure_citations_appends_sources_when_missing() -> None:
    hits = [
        {"path": "/tmp/a.md", "chunk_id": 1},
        {"path": "/tmp/b.md", "chunk_id": 2},
    ]
    allowed = build_allowed_refs(hits)
    text = "요약입니다."
    out = ensure_citations_or_append_sources(text, hits, allowed_refs=allowed)
    assert "Sources:" in out
    assert "a.md [a#1]" in out


def test_ensure_citations_warns_on_unknown_ref() -> None:
    hits = [{"path": "/tmp/a.md", "chunk_id": 1}]
    allowed = build_allowed_refs(hits)
    text = "내용입니다. [ref: other#9]"
    out = ensure_citations_or_append_sources(text, hits, allowed_refs=allowed)
    assert "유효하지 않습니다" in out
    assert "Sources:" in out

