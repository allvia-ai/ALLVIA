"""Persistence helpers for meeting artifacts and exports."""
from __future__ import annotations

import json
import os
from datetime import timedelta
from pathlib import Path
from typing import Dict, Optional

from .models import MeetingJobConfig, MeetingSummary, MeetingTranscriptionResult
from .integrations import sync_action_items


def append_jsonl(path: Path, payload: Dict[str, object]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("a", encoding="utf-8") as handle:
        handle.write(json.dumps(payload, ensure_ascii=False) + "\n")


def record_quality_alerts(job: MeetingJobConfig, alerts: list[str]) -> None:
    if not alerts:
        return
    flag_path = job.output_dir / "summary_alerts.json"
    payload = {
        "meeting_id": job.audio_path.stem,
        "created_at": job.created_at.isoformat(),
        "alerts": alerts,
    }
    flag_path.write_text(json.dumps(payload, ensure_ascii=False, indent=2), encoding="utf-8")


def record_for_search(
    job: MeetingJobConfig,
    transcription: MeetingTranscriptionResult,
    summary: MeetingSummary,
    quality_metrics: Dict[str, float | int | str],
) -> None:
    index_env = os.getenv("MEETING_VECTOR_INDEX")
    if index_env:
        index_path = Path(index_env)
    else:
        index_path = job.output_dir.parent / "meeting_vector_index.jsonl"
    entry = {
        "meeting_id": job.audio_path.stem,
        "created_at": job.created_at.isoformat(),
        "language": transcription.language,
        "summary": summary.raw_summary,
        "highlights": summary.structured_summary.get("highlights", []),
        "action_items": summary.structured_summary.get("action_items", []),
        "decisions": summary.structured_summary.get("decisions", []),
        "quality_metrics": quality_metrics,
        "source": str(job.audio_path),
        "output_dir": str(job.output_dir),
    }

    try:
        index_path.parent.mkdir(parents=True, exist_ok=True)
        with index_path.open("a", encoding="utf-8") as handle:
            handle.write(json.dumps(entry, ensure_ascii=False) + "\n")
    except Exception:
        # best-effort; swallow diagnostics at caller
        raise


def record_analytics(recorder, job: MeetingJobConfig, transcription: MeetingTranscriptionResult, summary: MeetingSummary, quality_metrics: Dict[str, float | int | str]) -> None:
    if recorder is None:
        return
    recorder.record(job, transcription, summary, quality_metrics)


def record_audit(
    audit_logger,
    job: MeetingJobConfig,
    transcription: MeetingTranscriptionResult,
    summary: MeetingSummary,
    summary_payload: Dict[str, object],
    summary_path: Path,
    metadata_path: Path,
    segments_path: Path,
    *,
    summary_backend: str,
    stt_backend: str,
) -> None:
    if audit_logger is None or not audit_logger.is_enabled():
        return
    payload: Dict[str, object] = {
        "event_type": "meeting_pipeline.completed",
        "meeting_id": job.audio_path.stem,
        "audio_path": str(job.audio_path),
        "output_dir": str(job.output_dir),
        "summary_path": str(summary_path),
        "metadata_path": str(metadata_path),
        "segments_path": str(segments_path),
        "summary_backend": summary_backend,
        "stt_backend": stt_backend,
        "language": transcription.language,
        "duration_seconds": transcription.duration_seconds,
        "highlight_count": len(summary.highlights),
        "action_count": len(summary.action_items),
        "decision_count": len(summary.decisions),
        "quality_metrics": summary_payload.get("quality_metrics"),
        "status": "completed",
    }
    if job.policy_tag:
        payload["policy_tag"] = job.policy_tag
    if job.created_at:
        payload["created_at"] = job.created_at.isoformat()
    if summary.context:
        payload["context_snippet"] = summary.context[:500]
    audit_logger.record(payload)


def export_integrations(
    job: MeetingJobConfig,
    transcription: MeetingTranscriptionResult,
    summary: MeetingSummary,
) -> Dict[str, str]:
    attachments: Dict[str, str] = {}

    summary_md_path = job.output_dir / "summary.md"
    summary_md = _build_markdown_summary(job, transcription, summary)
    summary_md_path.write_text(summary_md, encoding="utf-8")
    attachments["summary_md"] = summary_md_path.name

    tasks_path = job.output_dir / "tasks.json"
    tasks_payload = [
        {
            "title": item.get("text", ""),
            "reference": item.get("ref"),
            "status": "pending",
            "source_meeting": job.audio_path.stem,
        }
        for item in summary.structured_summary.get("action_items", [])
        if isinstance(item, dict)
    ]
    tasks_path.write_text(
        json.dumps(tasks_payload, ensure_ascii=False, indent=2),
        encoding="utf-8",
    )
    attachments["tasks"] = tasks_path.name

    action_items_path = job.output_dir / "action_items.json"
    action_entries = [
        item
        for item in summary.structured_summary.get("action_items", [])
        if isinstance(item, dict)
    ]
    action_items_payload = {
        "meeting_id": job.audio_path.stem,
        "generated_at": job.created_at.isoformat(),
        "items": action_entries,
    }
    action_items_path.write_text(
        json.dumps(action_items_payload, ensure_ascii=False, indent=2),
        encoding="utf-8",
    )
    attachments["action_items"] = action_items_path.name

    calendar_path = job.output_dir / "meeting.ics"
    calendar_path.write_text(
        _build_calendar_event(job, transcription, summary),
        encoding="utf-8",
    )
    attachments["calendar"] = calendar_path.name

    integrations_path = job.output_dir / "integrations.json"
    integrations_payload = {
        "meeting_id": job.audio_path.stem,
        "generated_at": job.created_at.isoformat(),
        "tasks_file": tasks_path.name,
        "calendar_file": calendar_path.name,
        "action_items_file": action_items_path.name,
        "action_items": summary.structured_summary.get("action_items", []),
        "decisions": summary.structured_summary.get("decisions", []),
    }
    integrations_path.write_text(
        json.dumps(integrations_payload, ensure_ascii=False, indent=2),
        encoding="utf-8",
    )
    attachments["integrations"] = integrations_path.name

    summary_json = job.output_dir / "summary.json"
    if summary_json.exists():
        try:
            payload = json.loads(summary_json.read_text(encoding="utf-8"))
        except Exception:
            payload = {}
        payload.setdefault("attachments", {}).update(attachments)
        summary_json.write_text(
            json.dumps(payload, ensure_ascii=False, indent=2),
            encoding="utf-8",
        )

    return attachments


def _build_markdown_summary(
    job: MeetingJobConfig,
    transcription: MeetingTranscriptionResult,
    summary: MeetingSummary,
) -> str:
    lines = [
        f"# Meeting Summary — {job.audio_path.stem}",
        "",
        f"- Created: {job.created_at.isoformat()}",
        f"- Source: `{job.audio_path}`",
        f"- Language: `{transcription.language}`",
        "",
        "## Summary",
        summary.raw_summary.strip(),
        "",
    ]

    highlights = summary.structured_summary.get("highlights", []) if isinstance(summary.structured_summary, dict) else []
    if highlights:
        lines.extend(["## Highlights", ""])
        for item in highlights:
            if isinstance(item, dict):
                text = (item.get("text") or "").strip()
                ref = (item.get("ref") or "").strip()
                if ref:
                    lines.append(f"- {text} ({ref})".strip())
                else:
                    lines.append(f"- {text}".strip())
            else:
                lines.append(f"- {str(item).strip()}")
        lines.append("")

    actions = summary.structured_summary.get("action_items", []) if isinstance(summary.structured_summary, dict) else []
    if actions:
        lines.extend(["## Action Items", ""])
        for item in actions:
            if isinstance(item, dict):
                text = (item.get("text") or "").strip()
                ref = (item.get("ref") or "").strip()
                if ref:
                    lines.append(f"- {text} ({ref})".strip())
                else:
                    lines.append(f"- {text}".strip())
            else:
                lines.append(f"- {str(item).strip()}")
        lines.append("")

    decisions = summary.structured_summary.get("decisions", []) if isinstance(summary.structured_summary, dict) else []
    if decisions:
        lines.extend(["## Decisions", ""])
        for item in decisions:
            if isinstance(item, dict):
                text = (item.get("text") or "").strip()
                ref = (item.get("ref") or "").strip()
                if ref:
                    lines.append(f"- {text} ({ref})".strip())
                else:
                    lines.append(f"- {text}".strip())
            else:
                lines.append(f"- {str(item).strip()}")
        lines.append("")

    context_prompt = (summary.context or "").strip()
    if context_prompt:
        preview = context_prompt if len(context_prompt) <= 1500 else context_prompt[:1500] + "…"
        lines.extend(["## Context (Preview)", "", preview, ""])

    return "\n".join(lines).rstrip() + "\n"


def sync_action_items_if_configured(job: MeetingJobConfig, summary: MeetingSummary, integration_config) -> None:
    if not integration_config:
        return
    entries = summary.structured_summary.get("action_items") or []
    if not entries:
        return
    sync_action_items(entries, integration_config, output_dir=job.output_dir)


def _build_calendar_event(
    job: MeetingJobConfig,
    transcription: MeetingTranscriptionResult,
    summary: MeetingSummary,
) -> str:
    start = job.created_at
    duration = transcription.duration_seconds or 0.0
    if duration <= 0:
        duration = 3600.0
    end = start + timedelta(seconds=duration)
    dtstamp = start.strftime("%Y%m%dT%H%M%SZ")
    dtstart = dtstamp
    dtend = end.strftime("%Y%m%dT%H%M%SZ")
    description = summary.raw_summary.replace("\n", "\\n")
    uid = f"{job.audio_path.stem}-{int(start.timestamp())}@infopilot.local"

    lines = [
        "BEGIN:VCALENDAR",
        "VERSION:2.0",
        "PRODID:-//InfoPilot//Meeting Agent//EN",
        "BEGIN:VEVENT",
        f"UID:{uid}",
        f"DTSTAMP:{dtstamp}",
        f"DTSTART:{dtstart}",
        f"DTEND:{dtend}",
        f"SUMMARY:{job.audio_path.stem or 'Meeting'}",
        f"DESCRIPTION:{description}",
        "END:VEVENT",
        "END:VCALENDAR",
    ]
    return "\r\n".join(lines) + "\r\n"


__all__ = [
    "append_jsonl",
    "record_quality_alerts",
    "record_for_search",
    "record_analytics",
    "record_audit",
    "export_integrations",
    "sync_action_items_if_configured",
]
