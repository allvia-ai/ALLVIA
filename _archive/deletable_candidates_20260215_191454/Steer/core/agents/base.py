"""Shared agent interfaces for InfoPilot assistants."""
from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any, Dict, Optional, Protocol


@dataclass
class AgentRequest:
    """Container passed to agents when handling a user query."""

    query: str
    context: Dict[str, Any] = field(default_factory=dict)


@dataclass
class AgentResult:
    """Standard response returned by conversational agents."""

    content: str
    metadata: Dict[str, Any] = field(default_factory=dict)
    suggestions: Optional[list[str]] = None


class ConversationalAgent(Protocol):
    """Lightweight protocol implemented by document/meeting/photo agents."""

    name: str
    description: str

    def prepare(self) -> None:
        """Initialise heavy dependencies. Called once during startup."""

    def run(self, request: AgentRequest) -> AgentResult:
        """Execute the agent with the provided request and return a result."""
