# core/agents/meeting/segmentation.py
"""
Audio segmentation and transcript processing utilities for the meeting agent.
"""
from __future__ import annotations

import logging
import re
from pathlib import Path
from typing import Dict, List, Optional, Sequence, Iterable

# Constants moved from pipeline.py if needed, or imported?
# pipeline.py uses SENTENCE_BOUNDARY, AVERAGE_SPEECH_WPM. 
# We should probably define them here or import them.
SENTENCE_BOUNDARY = re.compile(r"([.!?])\s+")
AVERAGE_SPEECH_WPM = 150.0

LOGGER = logging.getLogger(__name__)


def estimate_duration(audio_path: Path, transcript: str) -> float:
    """Estimate audio duration from file or text length."""
    try:
        import soundfile as sf
        with sf.SoundFile(audio_path) as audio:
            return len(audio) / audio.samplerate
    except Exception:
        LOGGER.debug("soundfile not available for %s; estimating duration from text", audio_path)

    return estimate_text_duration(transcript)


def estimate_text_duration(transcript: str) -> float:
    """Rough estimate based on word count."""
    words = max(len((transcript or "").split()), 1)
    minutes = words / AVERAGE_SPEECH_WPM
    return round(minutes * 60, 2)


def segment_transcript(
    transcript: str,
    duration_seconds: float,
    speaker_count: Optional[int] = None,
) -> List[dict]:
    """Break a raw transcript block into heuristic segments with estimated timestamps."""
    sentences = [s.strip() for s in SENTENCE_BOUNDARY.split(transcript) if s.strip()]
    if not sentences:
        sentences = [transcript.strip() or "(empty transcript)"]

    segment_count = len(sentences)
    slice_duration = duration_seconds / segment_count if segment_count else 0.0
    segments: List[dict] = []
    cursor = 0.0
    
    # Speaker cycling logic
    cycle_limit = speaker_count or 1
    
    for index, sentence in enumerate(sentences):
        start = round(cursor, 2)
        if index == segment_count - 1:
            end = duration_seconds
        else:
            end = round(cursor + slice_duration, 2)
        cursor = end
        
        segments.append(
            {
                "start": start,
                "end": max(end, start),
                "speaker": f"speaker_{(index % cycle_limit) + 1}",
                "text": sentence,
            }
        )
    return segments


def safe_time(value: Optional[float], default: float) -> float:
    try:
        return float(value) if value is not None else float(default)
    except (TypeError, ValueError):
        return float(default)


def normalise_segments(
    segments: Optional[Sequence[dict]],
    speaker_count: Optional[int] = None,
    speaker_identifier = None,
    audio_path: Optional[Path] = None,
) -> List[dict]:
    """Sort, merge, and normalize segments, optionally applying speaker ID."""
    if not segments:
        return []

    speaker_alias: Dict[str, str] = {}
    next_alias = 1
    normalised: List[dict] = []

    sorted_segments = sorted(
        segments,
        key=lambda item: (
            safe_time(item.get("start"), 0.0),
            safe_time(item.get("end"), 0.0),
        ),
    )

    fallback_cycle = speaker_count or 1

    for segment in sorted_segments:
        text = str(segment.get("text") or "").strip()
        if not text:
            continue

        start = round(safe_time(segment.get("start"), 0.0), 2)
        end = round(safe_time(segment.get("end"), start), 2)
        if end < start:
            end = start

        raw_speaker = str(segment.get("speaker") or "").strip()
        if raw_speaker:
            speaker_label = speaker_alias.get(raw_speaker)
            if speaker_label is None:
                speaker_label = f"speaker_{next_alias}"
                speaker_alias[raw_speaker] = speaker_label
                next_alias += 1
        else:
            cycle = fallback_cycle if fallback_cycle > 0 else max(len(speaker_alias), 1)
            index = (len(normalised) % cycle) + 1 if cycle else 1
            speaker_label = f"speaker_{index}"

        if normalised and normalised[-1]["speaker"] == speaker_label:
            # Merge with previous if same speaker
            normalised[-1]["text"] = f"{normalised[-1]['text']} {text}".strip()
            normalised[-1]["end"] = round(max(normalised[-1]["end"], end), 2)
        else:
            normalised.append(
                {
                    "start": start,
                    "end": end,
                    "speaker": speaker_label,
                    "text": text,
                }
            )

    # Apply external Speaker ID if available
    if speaker_identifier is not None and audio_path is not None:
        try:
            return speaker_identifier.label_segments(audio_path, normalised)
        except Exception as exc:
            LOGGER.warning("speaker identification failed: %s", exc)
            return normalised
            
    return normalised
