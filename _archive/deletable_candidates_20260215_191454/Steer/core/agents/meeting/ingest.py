"""Helpers to ingest new meeting files from smart folders."""
from __future__ import annotations

from pathlib import Path
from typing import Iterable, Optional

from core.agents.meeting.models import MeetingJobConfig
from core.agents.meeting.pipeline import MeetingPipeline


def ingest_file(
    audio_path: Path,
    output_root: Path,
    *,
    pipeline: Optional[MeetingPipeline] = None,
) -> Path:
    audio_path = audio_path.resolve()
    output_dir = output_root / audio_path.stem
    job = MeetingJobConfig(audio_path=audio_path, output_dir=output_dir)
    pipeline = pipeline or MeetingPipeline()
    pipeline.run(job)
    return output_dir


def ingest_folder(
    input_dir: Path,
    output_root: Path,
    *,
    pattern: str = "*.wav",
    recursive: bool = False,
    pipeline: Optional[MeetingPipeline] = None,
) -> Iterable[Path]:
    input_dir = input_dir.resolve()
    output_root = output_root.resolve()
    files = input_dir.rglob(pattern) if recursive else input_dir.glob(pattern)
    pipeline = pipeline or MeetingPipeline()
    for audio_path in files:
        if audio_path.is_file():
            yield ingest_file(audio_path, output_root, pipeline=pipeline)
