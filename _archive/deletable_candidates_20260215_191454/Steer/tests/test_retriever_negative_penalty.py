import pytest

from core.search import retriever


pytestmark = pytest.mark.full


def test_negative_template_penalty_detects_toc():
    hit = {"preview": "이 문서는 Table of Contents 및 목차로만 구성되어 있습니다."}
    penalty, reasons = retriever._negative_template_penalty(hit)
    assert penalty > 0.0
    assert any("목차" in reason.lower() or "table of contents" in reason.lower() for reason in reasons)


def test_negative_template_penalty_handles_missing_preview():
    hit = {"path": "/docs/표지_샘플.pdf"}
    penalty, reasons = retriever._negative_template_penalty(hit)
    assert penalty > 0.0
    assert reasons
