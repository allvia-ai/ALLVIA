"""Dataclasses describing meeting agent inputs/outputs."""
from __future__ import annotations

from dataclasses import dataclass, field
from datetime import datetime, timezone
from pathlib import Path
from typing import Dict, List, Optional


@dataclass
class MeetingJobConfig:
    audio_path: Path
    output_dir: Path
    language: str = "ko"
    diarize: bool = False
    speaker_count: Optional[int] = None
    policy_tag: Optional[str] = None
    context_dirs: List[Path] = field(default_factory=list)
    enable_resume: bool = False
    created_at: datetime = field(default_factory=lambda: datetime.now(timezone.utc))
    meeting_id: Optional[str] = None


@dataclass
class MeetingTranscriptionResult:
    text: str
    segments: List[dict]
    duration_seconds: float
    language: str


@dataclass
class MeetingSummary:
    id: str
    highlights: List[str]
    action_items: List[str]
    decisions: List[str]
    raw_summary: str
    transcript_path: Path
    structured_summary: dict
    masked_transcript: Optional[str] = None
    action_items_structured: List[Dict[str, str]] = field(default_factory=list)
    context: Optional[str] = None
    attachments: Dict[str, List[dict] | dict | str] = field(default_factory=dict)


@dataclass
class StreamingSummarySnapshot:
    summary_text: str
    highlights: List[str]
    action_items: List[str]
    decisions: List[str]
    elapsed_seconds: float
    language: str
