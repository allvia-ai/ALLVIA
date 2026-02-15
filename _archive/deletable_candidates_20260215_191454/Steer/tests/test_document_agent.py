from __future__ import annotations

from types import SimpleNamespace

import pytest

from core.agents import AgentRequest
from core.agents.document.agent import DocumentAgent
from core.agents.supervisor import SupervisorDecision


class StubSupervisor:
    def __init__(self, decision: SupervisorDecision) -> None:
        self.decision = decision
        self.called = False

    @staticmethod
    def is_enabled() -> bool:
        return True

    def decide(self, *args, **kwargs) -> SupervisorDecision:  # type: ignore[override]
        self.called = True
        return self.decision


@pytest.mark.smoke
def test_document_agent_supervisor_escalates_manual_review():
    agent = DocumentAgent.__new__(DocumentAgent)  # type: ignore[call-arg]
    agent.name = "document_search"
    agent._chat = SimpleNamespace(
        ask=lambda query: {
            "answer": "",
            "suggestions": [],
            "hits": [],
            "llm_summary": None,
        }
    )
    supervisor = StubSupervisor(SupervisorDecision(action="escalate", reason="empty answer"))
    agent._supervisor = supervisor  # type: ignore[attr-defined]
    agent._supervisor_mode = "auto"

    response = agent.run(AgentRequest(query="테스트", context=None))  # type: ignore[arg-type]

    assert supervisor.called
    assert response.metadata.get("requires_manual_review") is True
    assert response.metadata.get("supervisor_reason") == "empty answer"
