from __future__ import annotations

from pathlib import Path
from typing import Dict, List

import pytest

from core.conversation.llm_client import LLMClient, LLMClientError
from core.conversation.lnp_chat import LNPChat
from core.conversation.prompting import ToolRouter


class _DummyLLM(LLMClient):
    def __init__(self, response: str = "", *, fail: bool = False) -> None:
        self._response = response
        self._fail = fail
        self.calls: List[Dict[str, str]] = []

    def is_available(self) -> bool:
        return True

    def generate(self, prompt: str, *, system: str | None = None, timeout: float = 30.0) -> str:
        self.calls.append({"prompt": prompt, "system": system or "", "timeout": str(timeout)})
        if self._fail:
            raise LLMClientError("forced failure")
        return self._response


def _stub_hits() -> List[Dict[str, object]]:
    return [
        {"path": "docs/a.pdf", "ext": ".pdf", "preview": "첫 번째 문서 요약 내용입니다."},
        {"path": "docs/b.docx", "ext": ".docx", "preview": "두 번째 문서에는 중요한 지표가 포함되어 있습니다."},
    ]


@pytest.fixture
def chat(tmp_path: Path) -> LNPChat:
    return LNPChat(
        model_path=tmp_path / "model.joblib",
        corpus_path=tmp_path / "corpus.parquet",
        cache_dir=tmp_path / "cache",
    )


@pytest.mark.full
def test_summarize_hits_returns_llm_response(chat: LNPChat) -> None:
    dummy = _DummyLLM(response="요약 결과")
    chat.llm_client = dummy

    summary = chat._summarize_hits("테스트 질문", _stub_hits())

    assert summary is not None
    assert "요약 결과" in summary
    # Sources footer removed - LLM now includes inline citations
    assert dummy.calls, "LLM 호출이 수행되어야 합니다."
    recorded = dummy.calls[0]
    assert "테스트 질문" in recorded["prompt"]
    assert "a.pdf" in recorded["prompt"]


@pytest.mark.full
def test_summarize_hits_swallows_llm_failure(chat: LNPChat) -> None:
    chat.llm_client = _DummyLLM(response="", fail=True)

    summary = chat._summarize_hits("질문", _stub_hits())

    assert summary is None


def test_toolrouter_selects_summary_for_keywords() -> None:
    router = ToolRouter()
    action = router.select_action("/search 회의 내용을 요약해줘", use_translation=False, policy_active=False, llm_available=True)
    assert action == "search_and_summarize"


def test_toolrouter_prefers_search_for_short_queries() -> None:
    router = ToolRouter()
    action = router.select_action("보고서", use_translation=False, policy_active=False, llm_available=True)
    assert action == "dialogue"


def test_toolrouter_respects_llm_unavailable() -> None:
    router = ToolRouter()
    action = router.select_action("/search 이 문서 정리해줘", use_translation=False, policy_active=False, llm_available=False)
    assert action == "search"
