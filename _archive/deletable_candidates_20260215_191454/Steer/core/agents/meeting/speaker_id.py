"""Speaker identification utilities for meeting pipeline."""
from __future__ import annotations

import json
import logging
import os
from dataclasses import dataclass
from pathlib import Path
from typing import Dict, Iterable, List, Optional

LOGGER = logging.getLogger(__name__)


def _optional_import_librosa():
    try:  # pragma: no cover - optional dependency
        import librosa  # type: ignore

        return librosa
    except Exception as exc:  # pragma: no cover - best-effort diagnostics
        LOGGER.debug("librosa unavailable for speaker ID: %s", exc)
        return None


def _optional_import_numpy():
    try:  # pragma: no cover - optional dependency
        import numpy as np  # type: ignore

        return np
    except Exception as exc:  # pragma: no cover - best-effort diagnostics
        LOGGER.debug("numpy unavailable for speaker ID: %s", exc)
        return None


@dataclass
class SpeakerProfile:
    """Represents a single registered speaker reference."""

    name: str
    audio_path: Path
    embedding: Optional[object] = None


class SpeakerIdentifier:
    """Naive speaker identification backed by reference voice profiles.

    The implementation intentionally keeps dependencies optional. When the
    required libraries (``librosa`` and ``numpy``) are present, the module will
    compute MFCC-based embeddings for the registered reference audio clips and
    compare them against each meeting segment. Otherwise, it gracefully
    degrades and simply returns the original speaker labels.
    """

    PROFILE_FILENAME = "profiles.json"

    def __init__(
        self,
        profiles: Iterable[SpeakerProfile],
        *,
        similarity_threshold: float = 0.75,
    ) -> None:
        self._librosa = _optional_import_librosa()
        self._np = _optional_import_numpy()
        self._enabled = self._librosa is not None and self._np is not None
        self._profiles: List[SpeakerProfile] = list(profiles)
        self._similarity_threshold = similarity_threshold

        if not self._profiles:
            LOGGER.debug("No speaker profiles provided; speaker ID disabled")
            self._enabled = False

        if self._enabled:
            self._prepare_embeddings()

    # ------------------------------------------------------------------
    # Factory helpers
    # ------------------------------------------------------------------
    @classmethod
    def from_env(cls) -> Optional["SpeakerIdentifier"]:
        """Create speaker identifier from environment configuration."""

        base = os.getenv("MEETING_SPEAKER_PROFILE_DIR")
        if not base:
            return None

        directory = Path(base)
        if not directory.exists():
            LOGGER.warning("speaker profile directory not found: %s", directory)
            return None

        profiles_path = directory / cls.PROFILE_FILENAME
        if not profiles_path.exists():
            LOGGER.warning("speaker profile manifest missing: %s", profiles_path)
            return None

        try:
            manifest = json.loads(profiles_path.read_text(encoding="utf-8"))
        except Exception as exc:  # pragma: no cover - manifest validation
            LOGGER.error("failed to load speaker profile manifest: %s", exc)
            return None

        profiles: List[SpeakerProfile] = []
        entries = manifest if isinstance(manifest, list) else manifest.get("profiles", [])
        for entry in entries:
            name = str(entry.get("name") or "").strip()
            audio_rel = entry.get("audio")
            if not name or not audio_rel:
                continue
            audio_path = (directory / audio_rel).expanduser().resolve()
            if not audio_path.exists():
                LOGGER.warning("speaker profile audio missing: %s", audio_path)
                continue
            profiles.append(SpeakerProfile(name=name, audio_path=audio_path))

        if not profiles:
            LOGGER.warning("speaker profile manifest contains no valid entries")
            return None

        threshold = float(manifest.get("similarity_threshold", 0.75)) if isinstance(manifest, dict) else 0.75
        return cls(profiles, similarity_threshold=threshold)

    # ------------------------------------------------------------------
    # Public API
    # ------------------------------------------------------------------
    def label_segments(
        self,
        audio_path: Path,
        segments: List[dict],
    ) -> List[dict]:
        """Annotate segments with speaker names when possible."""

        if not self._enabled:
            return segments

        if not audio_path.exists():
            LOGGER.warning("audio path missing for speaker identification: %s", audio_path)
            return segments

        try:
            signal, sr = self._librosa.load(str(audio_path), sr=None)  # type: ignore[misc]
        except Exception as exc:  # pragma: no cover - best-effort diagnostics
            LOGGER.warning("failed to load audio for speaker ID: %s", exc)
            return segments

        speaker_votes: Dict[str, Dict[str, int]] = {}

        for segment in segments:
            speaker_id = str(segment.get("speaker") or "")
            start = float(segment.get("start") or 0.0)
            end = float(segment.get("end") or start)
            if end <= start:
                continue

            clip = self._extract_clip(signal, sr, start, end)
            if clip is None:
                continue

            embedding = self._compute_embedding(clip, sr)
            if embedding is None:
                continue

            best_profile, similarity = self._best_match(embedding)
            if best_profile is None or similarity < self._similarity_threshold:
                continue

            votes = speaker_votes.setdefault(speaker_id, {})
            votes[best_profile.name] = votes.get(best_profile.name, 0) + 1

        resolved_names: Dict[str, str] = {}
        for speaker_id, tally in speaker_votes.items():
            winner = max(tally.items(), key=lambda item: item[1])[0]
            resolved_names[speaker_id] = winner

        if not resolved_names:
            return segments

        annotated: List[dict] = []
        for segment in segments:
            speaker_id = str(segment.get("speaker") or "")
            name = resolved_names.get(speaker_id)
            if name:
                segment = dict(segment)
                segment.setdefault("speaker_id", speaker_id)
                segment["speaker"] = name
                segment["speaker_name"] = name
            annotated.append(segment)

        return annotated

    # ------------------------------------------------------------------
    # Internal helpers
    # ------------------------------------------------------------------
    def _prepare_embeddings(self) -> None:
        for profile in self._profiles:
            try:
                signal, sr = self._librosa.load(str(profile.audio_path), sr=None)  # type: ignore[misc]
            except Exception as exc:  # pragma: no cover - optional dependency
                LOGGER.warning("failed to load profile audio %s: %s", profile.audio_path, exc)
                continue
            profile.embedding = self._compute_embedding(signal, sr)
            if profile.embedding is None:
                LOGGER.warning("failed to compute embedding for %s", profile.audio_path)

        self._profiles = [profile for profile in self._profiles if profile.embedding is not None]
        if not self._profiles:
            LOGGER.warning("no usable speaker profiles after embedding computation")
            self._enabled = False

    def _extract_clip(self, signal, sr: int, start: float, end: float):
        if sr <= 0:
            return None
        start_idx = max(int(start * sr), 0)
        end_idx = min(int(end * sr), len(signal))
        if end_idx - start_idx < sr // 10:  # require at least 100ms
            return None
        return signal[start_idx:end_idx]

    def _compute_embedding(self, signal, sr: int):
        if self._np is None:
            return None
        try:
            mfcc = self._librosa.feature.mfcc(y=signal, sr=sr, n_mfcc=20)  # type: ignore[misc]
            if mfcc.size == 0:
                return None
            return self._np.mean(mfcc, axis=1)
        except Exception as exc:  # pragma: no cover - optional dependency
            LOGGER.debug("MFCC embedding failed: %s", exc)
            return None

    def _best_match(self, embedding):
        if self._np is None:
            return None, 0.0
        embedding = self._np.asarray(embedding)
        best_profile = None
        best_similarity = 0.0
        for profile in self._profiles:
            ref = self._np.asarray(profile.embedding)
            if ref.size == 0 or embedding.size == 0:
                continue
            similarity = self._cosine_similarity(embedding, ref)
            if similarity > best_similarity:
                best_similarity = similarity
                best_profile = profile
        return best_profile, float(best_similarity)

    def _cosine_similarity(self, a, b) -> float:
        if self._np is None:
            return 0.0
        a = self._np.asarray(a)
        b = self._np.asarray(b)
        denom = float(self._np.linalg.norm(a) * self._np.linalg.norm(b))
        if denom == 0.0:
            return 0.0
        return float(self._np.dot(a, b) / denom)


def load_speaker_identifier() -> Optional[SpeakerIdentifier]:
    """Helper that hides optional construction failures."""

    try:
        return SpeakerIdentifier.from_env()
    except Exception as exc:  # pragma: no cover - defensive guard
        LOGGER.warning("failed to initialise speaker identifier: %s", exc)
        return None

