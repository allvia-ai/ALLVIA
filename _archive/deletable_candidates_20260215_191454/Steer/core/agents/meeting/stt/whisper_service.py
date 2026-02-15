"""Wrapper around faster-whisper to provide meeting STT."""
from __future__ import annotations

import logging
import os
from pathlib import Path
from typing import Optional

from . import STTBackend, TranscriptionPayload

LOGGER = logging.getLogger(__name__)

_REMOTE_ALLOW_ENV = ("MEETING_ALLOW_REMOTE_MODELS", "INFOPILOT_ALLOW_REMOTE_MODELS")


def _remote_models_allowed() -> bool:
    for key in _REMOTE_ALLOW_ENV:
        raw = os.getenv(key)
        if raw is None:
            continue
        raw = raw.strip().lower()
        if raw in {"1", "true", "yes", "on"}:
            return True
        if raw in {"0", "false", "no", "off"}:
            return False
    return False


def _local_model_configured(model_size: str, download_root: Optional[str]) -> bool:
    """Return True if Whisper weights appear to be available locally without downloads."""
    if download_root:
        try:
            root = Path(download_root).expanduser()
            if root.exists() and root.is_dir():
                return True
        except Exception:
            pass
    # faster-whisper accepts a local path instead of a size string.
    try:
        candidate = Path(model_size).expanduser()
        if candidate.exists():
            return True
    except Exception:
        pass
    return False


class WhisperSTTBackend:
    """Lazy-initialised faster-whisper transcription backend."""

    def __init__(
        self,
        *,
        model_size: Optional[str] = None,
        device: Optional[str] = None,
        compute_type: Optional[str] = None,
        download_root: Optional[str] = None,
        beam_size: int = 5,
        vad_filter: bool = True,
    ) -> None:
        self.name = "whisper"
        self.model_size = model_size or os.getenv("MEETING_STT_MODEL", "small")
        self.device = device or os.getenv("MEETING_STT_DEVICE")
        self.compute_type = compute_type or os.getenv("MEETING_STT_COMPUTE", "int8")
        self.download_root = download_root or os.getenv("MEETING_STT_MODEL_DIR")
        self.beam_size = beam_size
        self.vad_filter = vad_filter
        self._model = None

    # ------------------------------------------------------------------
    # Helpers
    # ------------------------------------------------------------------
    def _ensure_model(self):
        if self._model is not None:
            return self._model

        from faster_whisper import WhisperModel  # type: ignore

        if not _remote_models_allowed() and not _local_model_configured(self.model_size, self.download_root):
            raise RuntimeError(
                "faster-whisper 모델이 로컬에 없어 STT를 진행할 수 없습니다. "
                "해결: (1) transcript(.txt/.md)를 입력으로 사용하거나, "
                "(2) MEETING_STT_MODEL_DIR에 로컬 모델 경로를 지정하세요. "
                "(원격 다운로드를 허용하려면 MEETING_ALLOW_REMOTE_MODELS=1)"
            )

        kwargs = {}
        if self.device:
            kwargs["device"] = self.device
        if self.compute_type:
            kwargs["compute_type"] = self.compute_type
        if self.download_root:
            kwargs["download_root"] = self.download_root

        LOGGER.info(
            "Loading faster-whisper model: size=%s device=%s compute=%s",  # noqa: G004
            self.model_size,
            kwargs.get("device", "auto"),
            kwargs.get("compute_type", "auto"),
        )
        self._model = WhisperModel(self.model_size, **kwargs)
        return self._model

    def _default_speaker(self, index: int) -> str:
        # Diarisation may be disabled or unsupported; fall back to speaker index.
        return f"speaker_{index + 1}"

    # ------------------------------------------------------------------
    # STTBackend API
    # ------------------------------------------------------------------
    def transcribe(
        self,
        audio_path: Path,
        *,
        language: Optional[str] = None,
        diarize: bool = False,
        speaker_count: Optional[int] = None,
    ) -> TranscriptionPayload:
        model = self._ensure_model()

        kwargs = {
            "beam_size": self.beam_size,
            "vad_filter": self.vad_filter,
        }
        if language:
            kwargs["language"] = language
        if diarize:
            kwargs["diarize"] = True
            if speaker_count:
                kwargs["speaker_count"] = speaker_count

        segments_iter, info = model.transcribe(str(audio_path), **kwargs)

        segments = []
        text_chunks = []
        speaker_cycle = max(speaker_count or 0, 1)
        for idx, segment in enumerate(segments_iter):
            speaker = getattr(segment, "speaker", None)
            if not speaker:
                speaker = self._default_speaker(idx % speaker_cycle)
            chunk_text = (segment.text or "").strip()
            if chunk_text:
                text_chunks.append(chunk_text)
            segments.append(
                {
                    "start": round(float(getattr(segment, "start", 0.0)), 2),
                    "end": round(float(getattr(segment, "end", 0.0)), 2),
                    "speaker": speaker,
                    "text": chunk_text,
                }
            )

        joined_text = " ".join(text_chunks).strip()
        duration = getattr(info, "duration", None)
        detected_language = getattr(info, "language", None)

        return TranscriptionPayload(
            text=joined_text,
            segments=segments,
            duration_seconds=duration,
            language=detected_language or language,
        )
