"""Workflow checkpointing utilities for the meeting pipeline."""
from __future__ import annotations

import json
from dataclasses import dataclass, field
from pathlib import Path
from typing import Dict, Optional

from .models import MeetingSummary, MeetingTranscriptionResult


@dataclass
class StageState:
    status: str = "pending"
    payload: Optional[str] = None


@dataclass
class WorkflowState:
    stages: Dict[str, StageState] = field(default_factory=dict)


class MeetingWorkflowEngine:
    """Persist and reload intermediate pipeline artefacts."""

    STAGES = ("transcription", "summary", "persistence")

    def __init__(self, output_dir: Path, *, enable_resume: bool = False) -> None:
        self._output_dir = output_dir
        self._state_path = output_dir / "workflow_state.json"
        self._checkpoint_dir = output_dir / "checkpoints"
        self._checkpoint_dir.mkdir(parents=True, exist_ok=True)
        self._state = self._load_state()
        self._enable_resume = enable_resume

    # ------------------------------------------------------------------
    # State persistence helpers
    # ------------------------------------------------------------------
    def _load_state(self) -> WorkflowState:
        if not self._state_path.exists():
            return WorkflowState(stages={stage: StageState() for stage in self.STAGES})
        try:
            payload = json.loads(self._state_path.read_text(encoding="utf-8"))
        except Exception:
            payload = {}
        stages = {}
        for stage in self.STAGES:
            info = payload.get(stage, {}) if isinstance(payload, dict) else {}
            stages[stage] = StageState(
                status=str(info.get("status", "pending")),
                payload=info.get("payload"),
            )
        return WorkflowState(stages=stages)

    def _write_state(self) -> None:
        serialised = {name: {"status": state.status, "payload": state.payload} for name, state in self._state.stages.items()}
        self._state_path.write_text(json.dumps(serialised, ensure_ascii=False, indent=2), encoding="utf-8")

    # ------------------------------------------------------------------
    # Stage control helpers
    # ------------------------------------------------------------------
    def should_run(self, stage: str) -> bool:
        state = self._state.stages.get(stage)
        if not state:
            return True
        if state.status != "completed":
            return True
        return not self._enable_resume

    def mark_completed(self, stage: str) -> None:
        self._state.stages.setdefault(stage, StageState()).status = "completed"
        self._write_state()

    def store_transcription(self, transcription: MeetingTranscriptionResult) -> Path:
        path = self._checkpoint_dir / "transcription.json"
        payload = {
            "text": transcription.text,
            "segments": transcription.segments,
            "duration_seconds": transcription.duration_seconds,
            "language": transcription.language,
        }
        path.write_text(json.dumps(payload, ensure_ascii=False, indent=2), encoding="utf-8")
        self._state.stages.setdefault("transcription", StageState()).payload = path.name
        self._write_state()
        return path

    def load_transcription(self) -> Optional[MeetingTranscriptionResult]:
        state = self._state.stages.get("transcription")
        if not state or not state.payload:
            return None
        path = self._checkpoint_dir / state.payload
        if not path.exists():
            return None
        try:
            data = json.loads(path.read_text(encoding="utf-8"))
        except Exception:
            return None
        return MeetingTranscriptionResult(
            text=data.get("text", ""),
            segments=data.get("segments", []),
            duration_seconds=float(data.get("duration_seconds", 0.0)),
            language=data.get("language", "ko"),
        )

    def store_summary(self, summary: MeetingSummary) -> Path:
        path = self._checkpoint_dir / "summary.json"
        payload = {
            "highlights": summary.highlights,
            "action_items": summary.action_items,
            "decisions": summary.decisions,
            "raw_summary": summary.raw_summary,
            "structured_summary": summary.structured_summary,
            "context": summary.context,
            "attachments": summary.attachments,
        }
        path.write_text(json.dumps(payload, ensure_ascii=False, indent=2), encoding="utf-8")
        self._state.stages.setdefault("summary", StageState()).payload = path.name
        self._write_state()
        return path

    def load_summary(self) -> Optional[MeetingSummary]:
        state = self._state.stages.get("summary")
        if not state or not state.payload:
            return None
        path = self._checkpoint_dir / state.payload
        if not path.exists():
            return None
        try:
            data = json.loads(path.read_text(encoding="utf-8"))
        except Exception:
            return None
        return MeetingSummary(
            highlights=list(data.get("highlights", [])),
            action_items=list(data.get("action_items", [])),
            decisions=list(data.get("decisions", [])),
            raw_summary=data.get("raw_summary", ""),
            transcript_path=Path(""),
            structured_summary=data.get("structured_summary") or {},
            context=data.get("context"),
            attachments=data.get("attachments") or {},
        )
