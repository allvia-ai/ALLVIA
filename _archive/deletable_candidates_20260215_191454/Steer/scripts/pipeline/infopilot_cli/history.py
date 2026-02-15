from __future__ import annotations

import json
from pathlib import Path
from typing import Dict, Iterable, List

HISTORY_PATH = Path.home() / ".infopilot" / "agent_history.json"
MAX_AGENT_HISTORY = 5


def load_agent_history() -> Dict[str, List[str]]:
    try:
        payload = json.loads(HISTORY_PATH.read_text(encoding="utf-8"))
        if not isinstance(payload, dict):
            return {"meeting_audio": [], "photo_roots": []}
        meeting = [str(item) for item in payload.get("meeting_audio", []) if isinstance(item, str)]
        photo = [str(item) for item in payload.get("photo_roots", []) if isinstance(item, str)]
        return {
            "meeting_audio": meeting[:MAX_AGENT_HISTORY],
            "photo_roots": photo[:MAX_AGENT_HISTORY],
        }
    except Exception:
        return {"meeting_audio": [], "photo_roots": []}


def save_agent_history(history: Dict[str, List[str]]) -> None:
    try:
        HISTORY_PATH.parent.mkdir(parents=True, exist_ok=True)
        HISTORY_PATH.write_text(json.dumps(history, ensure_ascii=False, indent=2), encoding="utf-8")
    except Exception:
        pass


def remember_agent_history(kind: str, values: Iterable[str]) -> None:
    if kind not in {"meeting_audio", "photo_roots"}:
        return
    history = load_agent_history()
    original = history.get(kind, [])
    merged: List[str] = []
    for value in values:
        normalised = str(Path(value).expanduser())
        if normalised and normalised not in merged:
            merged.append(normalised)
    for existing in original:
        if existing not in merged:
            merged.append(existing)
    history[kind] = merged[:MAX_AGENT_HISTORY]
    save_agent_history(history)


__all__ = [
    "HISTORY_PATH",
    "MAX_AGENT_HISTORY",
    "load_agent_history",
    "save_agent_history",
    "remember_agent_history",
]

