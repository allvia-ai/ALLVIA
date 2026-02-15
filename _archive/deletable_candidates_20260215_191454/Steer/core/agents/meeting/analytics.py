"""Analytics helpers for meeting insights and retraining signals."""
from __future__ import annotations

import json
import os
from collections import Counter
from dataclasses import dataclass
from pathlib import Path
from typing import Dict, Iterable, List, Optional

from .models import MeetingJobConfig, MeetingSummary, MeetingTranscriptionResult


@dataclass
class SpeakerAggregate:
    """Lightweight container describing per-speaker statistics."""

    name: str
    duration: float
    segment_count: int
    word_count: int


class MeetingAnalyticsRecorder:
    """Persist analytics artefacts for dashboards and retraining."""

    def __init__(self, base_dir: Optional[Path] = None) -> None:
        env_dir = os.getenv("MEETING_ANALYTICS_DIR")
        if base_dir is not None:
            self._base_dir = base_dir
        elif env_dir:
            self._base_dir = Path(env_dir)
        else:
            self._base_dir = None

    # ------------------------------------------------------------------
    # Public API
    # ------------------------------------------------------------------
    def record(
        self,
        job: MeetingJobConfig,
        transcription: MeetingTranscriptionResult,
        summary: MeetingSummary,
        quality_metrics: Dict[str, float | int | str],
    ) -> None:
        analytics_dir = self._resolve_dir(job)
        analytics_dir.mkdir(parents=True, exist_ok=True)

        entry = self._build_entry(job, transcription, summary, quality_metrics)
        meeting_path = analytics_dir / f"{entry['meeting_id']}.json"
        meeting_path.write_text(json.dumps(entry, ensure_ascii=False, indent=2), encoding="utf-8")

        self._update_index(analytics_dir, entry, meeting_path)
        self._update_dashboard(analytics_dir)
        self._enqueue_training(analytics_dir, entry, job, summary)

    # ------------------------------------------------------------------
    # Helpers
    # ------------------------------------------------------------------
    def _resolve_dir(self, job: MeetingJobConfig) -> Path:
        if self._base_dir is not None:
            return self._base_dir
        return job.output_dir / "analytics"

    def _build_entry(
        self,
        job: MeetingJobConfig,
        transcription: MeetingTranscriptionResult,
        summary: MeetingSummary,
        quality_metrics: Dict[str, float | int | str],
    ) -> Dict[str, object]:
        meeting_id = job.audio_path.stem or "meeting"
        speaker_stats = self._speaker_aggregates(transcription.segments)
        total_words = sum(item.word_count for item in speaker_stats)
        action_entries = summary.structured_summary.get("action_items", [])
        decision_entries = summary.structured_summary.get("decisions", [])

        return {
            "meeting_id": meeting_id,
            "created_at": job.created_at.isoformat(),
            "duration_seconds": transcription.duration_seconds,
            "language": transcription.language,
            "speaker_stats": [
                {
                    "name": speaker.name,
                    "duration_seconds": round(speaker.duration, 2),
                    "segment_count": speaker.segment_count,
                    "word_count": speaker.word_count,
                }
                for speaker in speaker_stats
            ],
            "counts": {
                "highlights": len(summary.highlights),
                "action_items": len(action_entries),
                "decisions": len(decision_entries),
            },
            "quality": quality_metrics,
            "total_words": total_words,
            "attachments": summary.attachments,
            "context_present": bool(summary.context),
        }

    def _speaker_aggregates(self, segments: Iterable[dict]) -> List[SpeakerAggregate]:
        aggregates: Dict[str, SpeakerAggregate] = {}
        for segment in segments:
            name = str(segment.get("speaker_name") or segment.get("speaker") or "speaker_1")
            start = float(segment.get("start") or 0.0)
            end = float(segment.get("end") or start)
            duration = max(end - start, 0.0)
            text = str(segment.get("text") or "")
            words = len(text.split()) if text else 0

            aggregate = aggregates.get(name)
            if aggregate is None:
                aggregates[name] = SpeakerAggregate(name=name, duration=duration, segment_count=1, word_count=words)
            else:
                aggregate.duration += duration
                aggregate.segment_count += 1
                aggregate.word_count += words
        return list(aggregates.values())

    def _update_index(self, analytics_dir: Path, entry: Dict[str, object], meeting_path: Path) -> None:
        index_path = analytics_dir / "analytics_index.jsonl"
        records: List[Dict[str, object]] = []
        if index_path.exists():
            for line in index_path.read_text(encoding="utf-8").splitlines():
                try:
                    record = json.loads(line)
                except json.JSONDecodeError:
                    continue
                if record.get("meeting_id") != entry["meeting_id"]:
                    records.append(record)

        records.append(
            {
                "meeting_id": entry["meeting_id"],
                "path": meeting_path.name,
                "created_at": entry["created_at"],
                "language": entry["language"],
            }
        )

        index_path.write_text(
            "\n".join(json.dumps(record, ensure_ascii=False) for record in records),
            encoding="utf-8",
        )

    def _update_dashboard(self, analytics_dir: Path) -> None:
        entries: List[Dict[str, object]] = []
        for item in analytics_dir.glob("*.json"):
            if item.name == "dashboard.json":
                continue
            try:
                entries.append(json.loads(item.read_text(encoding="utf-8")))
            except json.JSONDecodeError:
                continue

        dashboard = self._aggregate_dashboard(entries)
        dashboard_path = analytics_dir / "dashboard.json"
        dashboard_path.write_text(json.dumps(dashboard, ensure_ascii=False, indent=2), encoding="utf-8")

    def _aggregate_dashboard(self, entries: Iterable[Dict[str, object]]) -> Dict[str, object]:
        total_meetings = 0
        total_duration = 0.0
        total_actions = 0
        total_decisions = 0
        languages: Counter[str] = Counter()
        speaker_load: Counter[str] = Counter()

        for entry in entries:
            total_meetings += 1
            total_duration += float(entry.get("duration_seconds", 0.0))
            counts = entry.get("counts", {})
            total_actions += int(counts.get("action_items", 0))
            total_decisions += int(counts.get("decisions", 0))
            languages[str(entry.get("language"))] += 1

            for speaker in entry.get("speaker_stats", []):
                name = str(speaker.get("name") or "speaker")
                duration = float(speaker.get("duration_seconds", 0.0))
                speaker_load[name] += duration

        avg_duration = (total_duration / total_meetings) if total_meetings else 0.0
        avg_actions = (total_actions / total_meetings) if total_meetings else 0.0
        avg_decisions = (total_decisions / total_meetings) if total_meetings else 0.0

        top_speakers = [
            {"name": name, "talk_time": round(duration, 2)}
            for name, duration in speaker_load.most_common(5)
        ]

        language_distribution = {key: value for key, value in languages.items()}

        return {
            "total_meetings": total_meetings,
            "average_duration_seconds": round(avg_duration, 2),
            "average_action_items": round(avg_actions, 2),
            "average_decisions": round(avg_decisions, 2),
            "language_distribution": language_distribution,
            "top_speakers": top_speakers,
        }

    def _enqueue_training(
        self,
        analytics_dir: Path,
        entry: Dict[str, object],
        job: MeetingJobConfig,
        summary: MeetingSummary,
    ) -> None:
        queue_path = analytics_dir / "training_queue.jsonl"
        existing_ids = set()
        if queue_path.exists():
            for line in queue_path.read_text(encoding="utf-8").splitlines():
                try:
                    data = json.loads(line)
                except json.JSONDecodeError:
                    continue
                existing_ids.add(data.get("meeting_id"))

        if entry["meeting_id"] in existing_ids:
            return

        payload = {
            "meeting_id": entry["meeting_id"],
            "summary_path": str(job.output_dir / "summary.json"),
            "transcript_path": str(summary.transcript_path),
            "created_at": entry["created_at"],
            "language": entry["language"],
            "quality": entry["quality"],
        }
        with queue_path.open("a", encoding="utf-8") as handle:
            handle.write(json.dumps(payload, ensure_ascii=False) + "\n")


def load_dashboard(base_dir: Optional[Path] = None) -> Dict[str, object]:
    """Load analytics dashboard summary from the given directory or env."""

    env_dir = os.getenv("MEETING_ANALYTICS_DIR")
    if base_dir is None:
        if env_dir:
            base_dir = Path(env_dir)
        else:
            raise ValueError("analytics directory must be provided or MEETING_ANALYTICS_DIR set")

    dashboard_path = Path(base_dir) / "dashboard.json"
    if not dashboard_path.exists():
        raise FileNotFoundError(f"dashboard not found: {dashboard_path}")
    return json.loads(dashboard_path.read_text(encoding="utf-8"))


def format_dashboard(dashboard: Dict[str, object]) -> str:
    """Return a human-readable dashboard summary string."""

    total_meetings = dashboard.get("total_meetings", 0)
    avg_duration = dashboard.get("average_duration_seconds", 0)
    avg_actions = dashboard.get("average_action_items", 0)
    avg_decisions = dashboard.get("average_decisions", 0)
    language_dist = dashboard.get("language_distribution", {})
    top_speakers = dashboard.get("top_speakers", [])

    lines = [
        "회의 분석 대시보드",
        "----------------",
        f"총 회의 수: {total_meetings}",
        f"평균 회의 길이(초): {avg_duration}",
        f"평균 액션 아이템 수: {avg_actions}",
        f"평균 결정 수: {avg_decisions}",
        "언어 분포:",
    ]

    if language_dist:
        for language, count in language_dist.items():
            lines.append(f"  - {language}: {count}")
    else:
        lines.append("  - (데이터 없음)")

    lines.append("상위 발화자:")
    if top_speakers:
        for speaker in top_speakers:
            lines.append(
                f"  - {speaker.get('name', 'speaker')}: {speaker.get('talk_time', 0)}초"
            )
    else:
        lines.append("  - (데이터 없음)")

    return "\n".join(lines)
