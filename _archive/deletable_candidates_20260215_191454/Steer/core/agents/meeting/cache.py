"""Cache helpers for meeting pipeline artifacts."""
from __future__ import annotations

import json
from pathlib import Path
from typing import Dict, Optional

from .models import MeetingJobConfig, MeetingSummary


def audio_fingerprint(audio_path: Path) -> Dict[str, int]:
    try:
        stat = audio_path.stat()
    except FileNotFoundError:
        return {}
    return {
        "size": stat.st_size,
        "mtime_ns": getattr(stat, "st_mtime_ns", int(stat.st_mtime * 1_000_000_000)),
    }


def matches_fingerprint(audio_path: Path, fingerprint: Dict[str, int]) -> bool:
    if not fingerprint:
        return False
    current = audio_fingerprint(audio_path)
    if not current:
        return False
    return (
        current.get("size") == fingerprint.get("size")
        and current.get("mtime_ns") == fingerprint.get("mtime_ns")
    )


def load_cached_summary(
    job: MeetingJobConfig,
    *,
    stt_backend: str,
    summary_backend: str,
    cache_enabled: bool,
) -> Optional[MeetingSummary]:
    if not cache_enabled:
        return None

    summary_path = job.output_dir / "summary.json"
    segments_path = job.output_dir / "segments.json"
    metadata_path = job.output_dir / "metadata.json"
    transcript_path = job.output_dir / "transcript.txt"

    required = [summary_path, segments_path, metadata_path, transcript_path]
    if not all(path.exists() for path in required):
        return None

    try:
        metadata = json.loads(metadata_path.read_text(encoding="utf-8"))
    except Exception:
        return None

    cache_info = metadata.get("cache") or {}
    if cache_info.get("version") != 1:
        return None
    if cache_info.get("stt_backend") != stt_backend:
        return None
    if cache_info.get("summary_backend") != summary_backend:
        return None

    options = cache_info.get("options", {})
    if bool(options.get("diarize")) != bool(job.diarize):
        return None
    if options.get("speaker_count") != job.speaker_count:
        return None

    if not matches_fingerprint(job.audio_path, cache_info.get("audio_fingerprint", {})):
        return None

    try:
        summary_payload = json.loads(summary_path.read_text(encoding="utf-8"))
    except Exception:
        return None

    summary_section = summary_payload.get("summary", {}) or {}
    structured_summary_payload = summary_payload.get("structured_summary")

    def _extract_text_list(entries: object) -> list[str]:
        items: list[str] = []
        if isinstance(entries, list):
            for entry in entries:
                if isinstance(entry, str):
                    candidate = entry.strip()
                    if candidate:
                        items.append(candidate)
                elif isinstance(entry, dict):
                    text = str(entry.get("text") or "").strip()
                    if text:
                        items.append(text)
        elif isinstance(entries, dict):
            text = str(entries.get("text") or "").strip()
            if text:
                items.append(text)
        return items

    highlights_entries = summary_section.get("highlights", [])
    action_entries = summary_section.get("action_items", [])
    decision_entries = summary_section.get("decisions", [])

    if not highlights_entries and isinstance(structured_summary_payload, dict):
        highlights_entries = structured_summary_payload.get("highlights", [])
    if not action_entries and isinstance(structured_summary_payload, dict):
        action_entries = structured_summary_payload.get("action_items", [])
    if not decision_entries and isinstance(structured_summary_payload, dict):
        decision_entries = structured_summary_payload.get("decisions", [])

    highlights = _extract_text_list(highlights_entries)
    action_items = _extract_text_list(action_entries)
    decisions = _extract_text_list(decision_entries)
    raw_summary = summary_section.get("raw_summary") or summary_payload.get("raw_summary", "")

    structured_summary: Dict[str, object] = {}
    if isinstance(structured_summary_payload, dict):
        structured_summary.update(structured_summary_payload)
    structured_summary.setdefault("highlights", summary_section.get("highlights", []))
    structured_summary.setdefault("action_items", summary_section.get("action_items", []))
    structured_summary.setdefault("decisions", summary_section.get("decisions", []))

    alerts_payload = summary_payload.get("alerts") or (metadata.get("alerts") if isinstance(metadata, dict) else None)
    if alerts_payload and "alerts" not in structured_summary:
        structured_summary["alerts"] = alerts_payload
    supervisor_payload = summary_payload.get("supervisor")
    if not supervisor_payload and isinstance(metadata, dict):
        supervisor_payload = (metadata.get("supervisor") or {}).get("decision")
    if supervisor_payload and "supervisor_decision" not in structured_summary:
        structured_summary["supervisor_decision"] = supervisor_payload

    attachments = summary_payload.get("attachments") or {}
    if not isinstance(attachments, dict):
        attachments = {}
    context_prompt = summary_payload.get("context_prompt")

    return MeetingSummary(
        id=job.meeting_id or "cache_restored",
        highlights=highlights,
        action_items=action_items,
        decisions=decisions,
        raw_summary=raw_summary,
        transcript_path=transcript_path,
        structured_summary=structured_summary,
        context=context_prompt,
        attachments=attachments,
    )


__all__ = ["audio_fingerprint", "matches_fingerprint", "load_cached_summary"]
