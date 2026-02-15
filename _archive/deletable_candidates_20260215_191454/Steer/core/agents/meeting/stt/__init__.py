"""Speech-to-text backend factory for the meeting pipeline."""
from __future__ import annotations

import logging
from dataclasses import dataclass
from pathlib import Path
from typing import Optional, Protocol

LOGGER = logging.getLogger(__name__)


@dataclass
class TranscriptionPayload:
    """Container for raw transcription results."""

    text: str
    segments: Optional[list[dict]] = None
    duration_seconds: Optional[float] = None
    language: Optional[str] = None


class STTBackend(Protocol):
    """Protocol every speech-to-text backend implementation must follow."""

    name: str

    def transcribe(
        self,
        audio_path: Path,
        *,
        language: Optional[str] = None,
        diarize: bool = False,
        speaker_count: Optional[int] = None,
    ) -> TranscriptionPayload:
        """Return transcription payload for the provided audio file."""
        ...


def create_stt_backend(name: str | None, **kwargs) -> Optional[STTBackend]:
    """Instantiate the configured STT backend.

    Parameters
    ----------
    name:
        Identifier for the backend implementation. Supported values:
        - "placeholder"/"none": no STT, rely on sidecar transcripts.
        - "whisper": use faster-whisper WhisperModel.

    kwargs:
        Forwarded to the backend constructor.
    """

    if not name:
        return None

    normalized = name.lower().strip()
    if normalized in {"placeholder", "none", "noop"}:
        return None

    if normalized in {"whisper", "faster-whisper"}:
        try:
            from .whisper_service import WhisperSTTBackend
        except ImportError as exc:  # pragma: no cover - optional dependency
            LOGGER.error("faster-whisper is not installed: %s", exc)
            return None
        return WhisperSTTBackend(**kwargs)

    if normalized in {"wav2vec2", "wav2vec"}:
        try:
            from .wav2vec2_service import Wav2Vec2STTBackend
        except ImportError as exc:  # pragma: no cover - optional dependency
            LOGGER.error("transformers wav2vec2 backend unavailable: %s", exc)
            return None
        return Wav2Vec2STTBackend(**kwargs)

    LOGGER.warning("Unknown STT backend '%s'; falling back to placeholder", name)
    return None
