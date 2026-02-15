"""Streaming helper for incremental meeting transcription and summaries."""
from __future__ import annotations

from typing import Dict, List, Optional, Tuple, TYPE_CHECKING

from .models import (
    MeetingJobConfig,
    MeetingSummary,
    MeetingTranscriptionResult,
    StreamingSummarySnapshot,
)

if TYPE_CHECKING:  # Avoid circular import at runtime
    from .pipeline import MeetingPipeline


class StreamingMeetingSession:
    """Stateful helper that supports streaming meeting transcription snapshots."""

    def __init__(
        self,
        pipeline: "MeetingPipeline",
        job: MeetingJobConfig,
        *,
        update_interval: float,
    ) -> None:
        self._pipeline = pipeline
        self._job = job
        self._update_interval = max(update_interval, 0.0)
        self._segments: List[dict] = []
        self._text_chunks: List[str] = []
        self._elapsed = 0.0
        self._since_snapshot = 0.0
        self._speaker_alias: Dict[str, str] = {}
        self._next_alias = 1
        self._final_summary: Optional[MeetingSummary] = None
        self._finalised = False

    def ingest(
        self,
        text: str,
        *,
        speaker: Optional[str] = None,
        start: Optional[float] = None,
        end: Optional[float] = None,
    ) -> Optional[StreamingSummarySnapshot]:
        if self._finalised:
            raise RuntimeError("streaming session already finalised")

        cleaned = (text or "").strip()
        if not cleaned:
            return None

        start_time, end_time = self._resolve_window(cleaned, start, end)
        speaker_label = self._normalise_speaker(speaker)

        segment = {
            "start": start_time,
            "end": end_time,
            "speaker": speaker_label,
            "text": cleaned,
        }
        self._segments.append(segment)
        self._text_chunks.append(cleaned)

        self._elapsed = max(self._elapsed, end_time)
        segment_duration = max(end_time - start_time, 0.0)
        self._since_snapshot += segment_duration

        if self._update_interval == 0 or self._since_snapshot >= self._update_interval:
            self._since_snapshot = 0.0
            return self.snapshot()
        return None

    def snapshot(self) -> StreamingSummarySnapshot:
        language = self._detect_language()
        highlights = self._pipeline._extract_highlights(self._segments, language)
        action_entries = self._pipeline._extract_action_items(self._segments, language)
        decision_entries = self._pipeline._extract_decisions(self._segments, language)
        summary_text = self._pipeline._build_summary_text(highlights, action_entries, decision_entries)

        return StreamingSummarySnapshot(
            summary_text=summary_text,
            highlights=[entry.get("text", "") for entry in highlights],
            action_items=[entry.get("text", "") for entry in action_entries],
            decisions=[entry.get("text", "") for entry in decision_entries],
            elapsed_seconds=self._elapsed,
            language=language,
        )

    def finalize(self) -> MeetingSummary:
        if self._final_summary is not None:
            return self._final_summary

        transcription = self._build_transcription_result()
        context_bundle = self._pipeline._collect_context_bundle(self._job)
        summary = self._pipeline._summarise(self._job, transcription, context_bundle)
        if self._pipeline._mask_pii_enabled:
            self._pipeline._mask_sensitive_content(transcription=transcription, summary=summary)
        self._pipeline._persist(self._job, transcription, summary, review_info=None, metrics=None, alerts=None, supervisor_info=None)

        self._final_summary = summary
        self._finalised = True
        return summary

    # ------------------------------------------------------------------
    # Helpers
    # ------------------------------------------------------------------

    def _resolve_window(
        self,
        text: str,
        start: Optional[float],
        end: Optional[float],
    ) -> Tuple[float, float]:
        if start is None:
            start_time = self._elapsed
        else:
            start_time = max(float(start), 0.0)

        if end is not None:
            end_time = max(float(end), start_time)
        else:
            duration = self._pipeline._estimate_text_duration(text)
            end_time = max(start_time + duration, start_time)

        return (round(start_time, 2), round(end_time, 2))

    def _normalise_speaker(self, speaker: Optional[str]) -> str:
        if not speaker:
            alias = self._speaker_alias.get("__default__")
            if alias:
                return alias
            alias = "speaker_1"
            self._speaker_alias["__default__"] = alias
            self._next_alias = max(self._next_alias, 2)
            return alias

        key = speaker.strip().lower()
        if key in self._speaker_alias:
            return self._speaker_alias[key]

        alias = f"speaker_{self._next_alias}"
        self._next_alias += 1
        self._speaker_alias[key] = alias
        return alias

    def _detect_language(self) -> str:
        text = " ".join(self._text_chunks)
        return self._pipeline._detect_language(text, self._job.language)

    def _build_transcription_result(self) -> MeetingTranscriptionResult:
        text = " ".join(self._text_chunks).strip()
        language = self._pipeline._detect_language(text, self._job.language)
        normalised_segments = self._pipeline._normalise_segments(self._segments, self._job)
        duration = self._elapsed if self._elapsed > 0 else self._pipeline._estimate_text_duration(text)
        return MeetingTranscriptionResult(
            text=text,
            segments=normalised_segments,
            duration_seconds=duration,
            language=language,
        )


__all__ = ["StreamingMeetingSession"]
