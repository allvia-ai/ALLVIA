"""In-memory session registry backed by the core retriever's SessionState."""
from __future__ import annotations

from dataclasses import dataclass, field
from typing import Dict, Optional, Tuple
from uuid import uuid4

from core.search.retriever import SessionState


@dataclass
class SessionRegistry:
    """Stores per-session `SessionState` objects for the API layer."""

    sessions: Dict[str, SessionState] = field(default_factory=dict)

    def get_or_create(self, session_id: Optional[str]) -> Tuple[str, SessionState]:
        if session_id:
            state = self.sessions.setdefault(session_id, SessionState())
            return session_id, state

        new_id = uuid4().hex
        state = SessionState()
        self.sessions[new_id] = state
        return new_id, state

    def reset(self, session_id: Optional[str]) -> Tuple[str, SessionState]:
        new_state = SessionState()
        target_id = session_id or uuid4().hex
        self.sessions[target_id] = new_state
        return target_id, new_state


registry = SessionRegistry()
