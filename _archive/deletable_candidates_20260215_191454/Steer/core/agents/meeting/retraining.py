"""Utilities for handling the meeting retraining queue."""
from __future__ import annotations

import json
import os
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Callable, Dict, List, Optional


@dataclass
class QueueEntry:
    meeting_id: str
    summary_path: str
    transcript_path: str
    created_at: str
    language: str
    quality: Dict[str, object]


class RetrainingQueueProcessor:
    """Simple queue manager for downstream retraining pipelines."""

    def __init__(self, base_dir: Optional[Path] = None) -> None:
        env_dir = os.getenv("MEETING_ANALYTICS_DIR")
        if base_dir is not None:
            self._base_dir = base_dir
        elif env_dir:
            self._base_dir = Path(env_dir)
        else:
            raise ValueError("analytics directory must be provided or MEETING_ANALYTICS_DIR set")

        self._queue_path = self._base_dir / "training_queue.jsonl"
        self._processed_path = self._base_dir / "training_processed.jsonl"

    def pending(self) -> List[QueueEntry]:
        """Return the current pending queue entries."""

        return [self._to_entry(item) for item in self._load_jsonl(self._queue_path)]

    def claim_next(self) -> Optional[QueueEntry]:
        """Pop the next queue entry and persist the updated queue."""

        entries = self._load_jsonl(self._queue_path)
        if not entries:
            return None

        next_entry = entries.pop(0)
        self._write_jsonl(self._queue_path, entries)

        claimed = self._to_entry(next_entry)
        self._append_jsonl(
            self._processed_path,
            {
                **next_entry,
                "status": "claimed",
                "claimed_at": datetime.now(timezone.utc).isoformat(),
            },
        )
        return claimed

    def mark_processed(self, entry: QueueEntry, *, status: str = "completed") -> None:
        """Append a processed record for the supplied entry."""

        payload = {
            "meeting_id": entry.meeting_id,
            "status": status,
            "completed_at": datetime.now(timezone.utc).isoformat(),
        }
        self._append_jsonl(self._processed_path, payload)

    def make_entry(
        self,
        *,
        meeting_id: str,
        summary_path: str = "",
        transcript_path: str = "",
        created_at: str = "",
        language: str = "",
        quality: Optional[Dict[str, object]] = None,
    ) -> QueueEntry:
        return QueueEntry(
            meeting_id=meeting_id,
            summary_path=summary_path,
            transcript_path=transcript_path,
            created_at=created_at,
            language=language,
            quality=quality or {},
        )

    def enqueue(self, entry: QueueEntry) -> None:
        """Append the provided entry to the pending queue."""

        payload = {
            "meeting_id": entry.meeting_id,
            "summary_path": entry.summary_path,
            "transcript_path": entry.transcript_path,
            "created_at": entry.created_at,
            "language": entry.language,
            "quality": entry.quality,
        }
        self._append_jsonl(self._queue_path, payload)

    # ------------------------------------------------------------------
    # Helpers
    # ------------------------------------------------------------------
    def _load_jsonl(self, path: Path) -> List[Dict[str, object]]:
        if not path.exists():
            return []
        results: List[Dict[str, object]] = []
        for line in path.read_text(encoding="utf-8").splitlines():
            if not line.strip():
                continue
            try:
                results.append(json.loads(line))
            except json.JSONDecodeError:
                continue
        return results

    def _write_jsonl(self, path: Path, entries: List[Dict[str, object]]) -> None:
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(
            "\n".join(json.dumps(entry, ensure_ascii=False) for entry in entries),
            encoding="utf-8",
        )

    def _append_jsonl(self, path: Path, entry: Dict[str, object]) -> None:
        path.parent.mkdir(parents=True, exist_ok=True)
        with path.open("a", encoding="utf-8") as handle:
            handle.write(json.dumps(entry, ensure_ascii=False) + "\n")

    def _to_entry(self, data: Dict[str, object]) -> QueueEntry:
        return QueueEntry(
            meeting_id=str(data.get("meeting_id")),
            summary_path=str(data.get("summary_path")),
            transcript_path=str(data.get("transcript_path")),
            created_at=str(data.get("created_at")),
            language=str(data.get("language", "")),
            quality=data.get("quality", {}),
        )


def process_next(
    handler: Callable[[QueueEntry], str],
    *,
    base_dir: Optional[Path] = None,
) -> bool:
    """Claim the next queue entry, pass it to handler, and mark status.

    The handler must return a status string such as ``"completed"`` or
    ``"failed"`` to indicate the result of the retraining step.
    """

    processor = RetrainingQueueProcessor(base_dir)
    entry = processor.claim_next()
    if entry is None:
        return False

    try:
        status = handler(entry)
    except Exception:
        processor.mark_processed(entry, status="failed")
        raise

    processor.mark_processed(entry, status=status or "completed")
    return True
