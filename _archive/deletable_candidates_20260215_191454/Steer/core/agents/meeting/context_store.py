"""Context store scaffolding for meeting RAG support."""
from __future__ import annotations

import json
import os
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable, List, Optional

from core.utils import get_logger

LOGGER = get_logger("meeting.context_store")


@dataclass
class StoredDocument:
    meeting_id: str
    text: str
    source: str


class MeetingContextStore:
    """Minimal JSONL-backed storage for meeting context and artefacts."""

    def __init__(self, base_dir: Optional[Path], enabled: bool) -> None:
        self._base_dir = base_dir
        self._enabled = enabled and base_dir is not None
        if self._enabled:
            self._base_dir.mkdir(parents=True, exist_ok=True)

    @classmethod
    def from_env(cls) -> "MeetingContextStore":
        enabled = os.getenv("MEETING_RAG_ENABLED", "0").strip().lower() in {"1", "true", "yes"}
        directory = os.getenv("MEETING_RAG_STORE")
        base_dir = Path(directory).expanduser() if directory else None
        return cls(base_dir, enabled)

    def is_enabled(self) -> bool:
        return self._enabled

    def record_documents(self, meeting_id: str, documents: Iterable[dict]) -> None:
        if not self._enabled or not documents:
            return
        path = self._base_dir / f"{meeting_id}.jsonl"
        lines: List[str] = []
        for doc in documents:
            text = (doc.get("preview") or doc.get("text") or "").strip()
            if not text:
                continue
            payload = {
                "meeting_id": meeting_id,
                "text": text,
                "source": doc.get("name") or doc.get("target_name") or "context",
            }
            lines.append(json.dumps(payload, ensure_ascii=False))

        if not lines:
            return

        with path.open("a", encoding="utf-8") as handle:
            handle.write("\n".join(lines) + "\n")
        LOGGER.debug("stored %s context snippets for %s", len(lines), meeting_id)

    def record_meeting_artifacts(
        self,
        meeting_id: str,
        transcription_text: str,
        summary_text: str,
        analytics: Optional[dict] = None,
    ) -> None:
        if not self._enabled:
            return
        path = self._base_dir / f"{meeting_id}.jsonl"
        entries: List[str] = []
        if transcription_text:
            entries.append(
                json.dumps(
                    {
                        "meeting_id": meeting_id,
                        "type": "transcript",
                        "source": "transcript",
                        "text": transcription_text[:2000],
                    },
                    ensure_ascii=False,
                )
            )
        if summary_text:
            entries.append(
                json.dumps(
                    {
                        "meeting_id": meeting_id,
                        "type": "summary",
                        "source": "summary",
                        "text": summary_text,
                    },
                    ensure_ascii=False,
                )
            )
        if analytics:
            digest = json.dumps(analytics, ensure_ascii=False)
            entries.append(
                json.dumps(
                    {
                        "meeting_id": meeting_id,
                        "type": "analytics",
                        "source": "analytics",
                        "text": digest[:2000],
                    },
                    ensure_ascii=False,
                )
            )
        if not entries:
            return
        with path.open("a", encoding="utf-8") as handle:
            handle.write("\n".join(entries) + "\n")

    def retrieve_prompt(self, meeting_id: str, top_k: int = 3) -> Optional[str]:
        if not self._enabled:
            return None
        path = self._base_dir / f"{meeting_id}.jsonl"
        if not path.exists():
            return None
        try:
            lines = path.read_text(encoding="utf-8").splitlines()
        except Exception as exc:  # pragma: no cover - defensive
            LOGGER.warning("failed to read context store for %s: %s", meeting_id, exc)
            return None

        snippets = []
        for line in reversed(lines):
            if len(snippets) >= top_k:
                break
            try:
                payload = json.loads(line)
            except json.JSONDecodeError:
                continue
            text = payload.get("text")
            source = payload.get("source")
            if text:
                snippets.append(f"[{source}] {text}")

        if not snippets:
            return None
        return "\n".join(snippets)
