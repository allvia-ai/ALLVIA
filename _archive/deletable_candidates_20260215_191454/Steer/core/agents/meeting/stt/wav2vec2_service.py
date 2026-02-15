"""Speech-to-text backend built on top of HuggingFace Wav2Vec2 models."""
from __future__ import annotations

import logging
import math
import os
import re
from pathlib import Path
from typing import Callable, Iterable, List, Optional, Sequence, Tuple

import numpy as np

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


def _configure_transformers_offline() -> None:
    os.environ.setdefault("HF_HUB_OFFLINE", "1")
    os.environ.setdefault("TRANSFORMERS_OFFLINE", "1")
    os.environ.setdefault("HF_HUB_DISABLE_TELEMETRY", "1")


try:  # Optional dependency; validated when the backend is used.
    import soundfile as sf  # type: ignore
except Exception:  # pragma: no cover - optional import guard
    sf = None  # type: ignore[assignment]

try:  # ``scipy`` is installed via scikit-learn, but keep a guard for minimal envs.
    from scipy.signal import resample_poly  # type: ignore
except Exception:  # pragma: no cover - optional import guard
    resample_poly = None  # type: ignore[assignment]

_DEFAULT_MODEL_ID = "kresnik/wav2vec2-large-xlsr-korean"
_SENTENCE_SPLIT_RE = re.compile(r"(?<=[.!?\n。？！])\s+")


class Wav2Vec2STTBackend:
    """Lazy Wav2Vec2 backend that streams audio through ``transformers`` pipeline."""

    name = "wav2vec2"

    def __init__(
        self,
        *,
        model_id: Optional[str] = None,
        device: Optional[str] = None,
        chunk_length_s: Optional[float] = None,
        stride_length_s: Optional[float] = None,
        sampling_rate: Optional[int] = None,
    ) -> None:
        env_model = os.getenv("MEETING_WAV2VEC2_MODEL") or os.getenv("MEETING_STT_MODEL")
        self.model_id = (model_id or env_model or _DEFAULT_MODEL_ID).strip()
        env_device = os.getenv("MEETING_WAV2VEC2_DEVICE") or os.getenv("MEETING_STT_DEVICE")
        self.device = (device or env_device or "").strip()
        env_chunk = os.getenv("MEETING_WAV2VEC2_CHUNK", "")
        env_stride = os.getenv("MEETING_WAV2VEC2_STRIDE", "")
        env_sr = os.getenv("MEETING_WAV2VEC2_SR", "")

        self.chunk_length_s = self._resolve_positive_float(chunk_length_s, env_chunk, default=20.0)
        stride_default = min(max(self.chunk_length_s * 0.25, 2.0), self.chunk_length_s * 0.8)
        self.stride_length_s = self._resolve_positive_float(stride_length_s, env_stride, default=stride_default)
        self.stride_length_s = max(0.5, min(self.stride_length_s, self.chunk_length_s - 1.0))
        self.sampling_rate = int(self._resolve_positive_float(sampling_rate, env_sr, default=16000))

        self._pipeline: Optional[Callable[..., dict]] = None

    # ------------------------------------------------------------------
    # Public API
    # ------------------------------------------------------------------
    def transcribe(
        self,
        audio_path: Path,
        *,
        language: Optional[str] = None,
        diarize: bool = False,
        speaker_count: Optional[int] = None,
    ) -> TranscriptionPayload:
        if diarize:  # pragma: no cover - warning path
            LOGGER.warning("wav2vec2 backend does not support diarisation; proceeding without speaker splits")

        audio, samplerate = self._load_audio(audio_path)
        duration = len(audio) / float(samplerate)
        pipe = self._ensure_pipeline()

        try:
            result = pipe(audio, sampling_rate=samplerate, return_timestamps="word")
        except (TypeError, ValueError):
            try:
                result = pipe(audio, sampling_rate=samplerate, return_timestamps=True)
            except (TypeError, ValueError):
                result = pipe(audio, sampling_rate=samplerate)

        if not isinstance(result, dict):
            raise RuntimeError("wav2vec2 pipeline returned unexpected payload")

        text = (result.get("text") or "").strip()
        chunks = result.get("chunks") or []
        segments, fallback_text = self._build_segments(chunks, text, duration)
        if not text and fallback_text:
            text = fallback_text

        detected_language = language or os.getenv("MEETING_WAV2VEC2_LANGUAGE")

        return TranscriptionPayload(
            text=text,
            segments=segments,
            duration_seconds=duration,
            language=detected_language or language,
        )

    # ------------------------------------------------------------------
    # Internal helpers
    # ------------------------------------------------------------------
    def _ensure_pipeline(self):
        if self._pipeline is not None:
            return self._pipeline
        if not self.model_id:
            raise RuntimeError("wav2vec2 model identifier is not configured")
        if sf is None:
            raise RuntimeError("soundfile is required for wav2vec2 STT backend")
        if not _remote_models_allowed():
            _configure_transformers_offline()
        try:
            from transformers import AutoModelForCTC, AutoProcessor, pipeline as hf_pipeline  # type: ignore
        except Exception as exc:  # pragma: no cover - import guard
            raise RuntimeError("transformers is required for wav2vec2 STT backend") from exc

        try:
            processor = AutoProcessor.from_pretrained(self.model_id)
            model = AutoModelForCTC.from_pretrained(self.model_id)
        except Exception as exc:  # pragma: no cover - offline cache guard
            if not _remote_models_allowed():
                raise RuntimeError(
                    "wav2vec2 모델이 로컬 캐시에 없어 STT를 진행할 수 없습니다. "
                    "해결: (1) transcript(.txt/.md)를 입력으로 사용하거나, "
                    "(2) 모델을 로컬에 미리 캐시한 뒤 다시 실행하세요. "
                    "(원격 다운로드를 허용하려면 MEETING_ALLOW_REMOTE_MODELS=1)"
                ) from exc
            raise
        tokenizer = getattr(processor, "tokenizer", None) or processor
        feature_extractor = getattr(processor, "feature_extractor", None) or processor
        feat_sr = getattr(feature_extractor, "sampling_rate", None)
        if isinstance(feat_sr, (int, float)) and feat_sr > 0:
            self.sampling_rate = int(feat_sr)

        device_index = self._resolve_device_index()
        stride = max(0.5, min(self.stride_length_s, self.chunk_length_s - 0.5))
        kwargs = {
            "chunk_length_s": self.chunk_length_s,
            "stride_length_s": (stride, stride),
            "device": device_index,
        }
        LOGGER.info(
            "Loading wav2vec2 STT model: id=%s chunk=%.1fs stride=%.1fs device=%s",
            self.model_id,
            self.chunk_length_s,
            stride,
            "cpu" if device_index < 0 else f"cuda:{device_index}",
        )
        pipe = hf_pipeline(
            "automatic-speech-recognition",
            model=model,
            tokenizer=tokenizer,
            feature_extractor=feature_extractor,
            **kwargs,
        )
        self._pipeline = pipe
        return pipe

    def _load_audio(self, audio_path: Path) -> Tuple[np.ndarray, int]:
        if sf is None:  # pragma: no cover - guard
            raise RuntimeError("soundfile is required for wav2vec2 STT backend")
        data, samplerate = sf.read(str(audio_path))
        if data.size == 0:
            raise ValueError(f"audio file is empty: {audio_path}")
        if data.ndim > 1:
            data = np.mean(data, axis=1)
        data = data.astype(np.float32, copy=False)
        if samplerate != self.sampling_rate:
            data = self._resample_audio(data, samplerate, self.sampling_rate)
            samplerate = self.sampling_rate
        return data, samplerate

    def _resample_audio(self, audio: np.ndarray, original_sr: int, target_sr: int) -> np.ndarray:
        if original_sr == target_sr:
            return audio
        if resample_poly is not None:
            factor = math.gcd(original_sr, target_sr)
            up = target_sr // factor
            down = original_sr // factor
            return resample_poly(audio, up, down).astype(np.float32, copy=False)
        if len(audio) == 0:
            return audio
        duration = len(audio) / float(original_sr)
        target_len = max(1, int(round(duration * target_sr)))
        x_old = np.linspace(0.0, 1.0, len(audio), endpoint=False)
        x_new = np.linspace(0.0, 1.0, target_len, endpoint=False)
        return np.interp(x_new, x_old, audio).astype(np.float32, copy=False)

    def _build_segments(
        self,
        chunks: Sequence[dict],
        text: str,
        duration: float,
    ) -> Tuple[List[dict], str]:
        segments: List[dict] = []
        chunk_texts: List[str] = []
        for chunk in chunks:
            chunk_text = (chunk.get("text") or "").strip()
            timestamp = chunk.get("timestamp") or chunk.get("timestamps")
            if not chunk_text:
                continue
            start, end = self._parse_timestamp(timestamp)
            segments.append(
                {
                    "start": start,
                    "end": end,
                    "speaker": "speaker_1",
                    "text": chunk_text,
                }
            )
            chunk_texts.append(chunk_text)

        if segments:
            return segments, " ".join(chunk_texts).strip()

        fallback_segments = self._segments_from_text(text, duration)
        fallback_text = text.strip() if text else " ".join(entry["text"] for entry in fallback_segments)
        return fallback_segments, fallback_text

    def _segments_from_text(self, text: str, duration: float) -> List[dict]:
        cleaned = [frag.strip() for frag in _SENTENCE_SPLIT_RE.split(text or "") if frag.strip()]
        if not cleaned and text:
            cleaned = [text.strip()]
        if not cleaned:
            return []
        window = duration / max(len(cleaned), 1)
        cursor = 0.0
        segments: List[dict] = []
        for fragment in cleaned:
            start = round(cursor, 2)
            cursor += window
            end = round(min(cursor, duration), 2)
            segments.append(
                {
                    "start": start,
                    "end": end if end > start else start,
                    "speaker": "speaker_1",
                    "text": fragment,
                }
            )
        if segments:
            segments[-1]["end"] = round(duration, 2)
        return segments

    @staticmethod
    def _parse_timestamp(raw: object) -> Tuple[float, float]:
        if isinstance(raw, (list, tuple)) and len(raw) == 2:
            try:
                start = round(float(raw[0]), 2)
                end = round(float(raw[1]), 2)
                if end < start:
                    end = start
                return start, end
            except (TypeError, ValueError):
                pass
        return 0.0, 0.0

    def _resolve_device_index(self) -> int:
        label = (self.device or "").strip().lower()
        try:
            import torch  # type: ignore
        except Exception:  # pragma: no cover - optional import guard
            return -1
        if not label:
            return 0 if torch.cuda.is_available() else -1
        if label == "cpu":
            return -1
        if label.startswith("cuda"):
            if not torch.cuda.is_available():
                LOGGER.warning("CUDA device requested but no GPU detected; falling back to CPU")
                return -1
            parts = label.split(":", 1)
            if len(parts) == 2:
                try:
                    index = int(parts[1])
                except ValueError:
                    index = 0
            else:
                index = 0
            if index >= torch.cuda.device_count():
                LOGGER.warning("Requested GPU index %s unavailable; using GPU 0", index)
                index = 0
            return index
        return -1

    @staticmethod
    def _resolve_positive_float(value: Optional[float], env_value: str, *, default: float) -> float:
        candidates: Iterable[object] = (value, env_value)
        for candidate in candidates:
            if candidate in {None, ""}:
                continue
            try:
                numeric = float(candidate)
            except (TypeError, ValueError):
                continue
            if numeric > 0:
                return numeric
        return default


__all__ = ["Wav2Vec2STTBackend"]
