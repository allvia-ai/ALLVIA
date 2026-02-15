# session.py - Extracted from retriever.py (GOD CLASS refactoring)
"""Session state management for personalized search."""

from __future__ import annotations

from collections import deque
from dataclasses import dataclass, field
from typing import Deque, Dict, List, Optional, Set, Tuple

_SESSION_HISTORY_LIMIT = 50
_SESSION_CHAT_HISTORY_LIMIT = 20
_SESSION_PREF_DECAY = 0.85
_SESSION_CLICK_WEIGHT = 0.35
_SESSION_PIN_WEIGHT = 0.6
_SESSION_LIKE_WEIGHT = 0.45
_SESSION_DISLIKE_WEIGHT = -0.5


def _clamp(value: float, lower: float, upper: float) -> float:
    return max(lower, min(upper, value))


def _normalize_ext(ext: Optional[str]) -> str:
    if not ext:
        return ""
    ext_value = str(ext).strip().lower()
    if not ext_value:
        return ""
    if not ext_value.startswith("."):
        ext_value = f".{ext_value}"
    return ext_value


def _normalize_owner(owner: Optional[str]) -> str:
    return (owner or "").strip().lower()


@dataclass
class SessionState:
    recent_queries: Deque[str] = field(default_factory=lambda: deque(maxlen=_SESSION_HISTORY_LIMIT))
    clicked_doc_ids: Set[int] = field(default_factory=set)
    preferred_exts: Dict[str, float] = field(default_factory=dict)
    owner_prior: Dict[str, float] = field(default_factory=dict)
    chat_history: Deque[Tuple[str, str]] = field(default_factory=lambda: deque(maxlen=_SESSION_CHAT_HISTORY_LIMIT))

    def add_query(self, query: str) -> None:
        if not query:
            return
        self.recent_queries.append(query)

    def record_user_message(self, message: str) -> None:
        self._append_chat_turn("user", message)

    def record_assistant_message(self, message: str) -> None:
        self._append_chat_turn("assistant", message)

    def get_chat_history(self) -> List[Tuple[str, str]]:
        return list(self.chat_history)

    def record_click(
        self,
        *,
        doc_id: Optional[int] = None,
        ext: Optional[str] = None,
        owner: Optional[str] = None,
    ) -> None:
        if doc_id is not None:
            self.clicked_doc_ids.add(int(doc_id))
        self._apply_preference(ext=ext, owner=owner, delta=_SESSION_CLICK_WEIGHT)

    def record_pin(
        self,
        *,
        doc_id: Optional[int] = None,
        ext: Optional[str] = None,
        owner: Optional[str] = None,
    ) -> None:
        if doc_id is not None:
            self.clicked_doc_ids.add(int(doc_id))
        self._apply_preference(ext=ext, owner=owner, delta=_SESSION_PIN_WEIGHT)

    def record_like(self, *, ext: Optional[str] = None, owner: Optional[str] = None) -> None:
        self._apply_preference(ext=ext, owner=owner, delta=_SESSION_LIKE_WEIGHT)

    def record_dislike(self, *, ext: Optional[str] = None, owner: Optional[str] = None) -> None:
        self._apply_preference(ext=ext, owner=owner, delta=_SESSION_DISLIKE_WEIGHT)

    def _apply_preference(self, *, ext: Optional[str], owner: Optional[str], delta: float) -> None:
        if ext:
            normalized_ext = _normalize_ext(ext)
            if normalized_ext:
                self._update_pref(self.preferred_exts, normalized_ext, delta)
        if owner:
            normalized_owner = _normalize_owner(owner)
            if normalized_owner:
                self._update_pref(self.owner_prior, normalized_owner, delta)

    def _update_pref(self, store: Dict[str, float], key: str, delta: float) -> None:
        current = store.get(key, 0.0) * _SESSION_PREF_DECAY
        updated = _clamp(current + delta, -1.0, 1.0)
        if abs(updated) < 1e-4:
            store.pop(key, None)
        else:
            store[key] = updated

    def _append_chat_turn(self, role: str, message: str) -> None:
        text = (message or "").strip()
        if not text:
            return
        normalized_role = role if role in {"user", "assistant"} else "assistant"
        self.chat_history.append((normalized_role, text))
