# reranker.py - Extracted from retriever.py (GOD CLASS refactoring)
"""Cross-encoder reranking utilities for retrieval results."""

from __future__ import annotations

import logging
import os
import time
from collections import deque
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Deque, Dict, Iterable, List, Optional, Set

from core.config.paths import MODELS_DIR

from .scoring import (
    _compose_rerank_document,
    _compute_extension_bonus,
    _compute_owner_bonus,
    _minmax_scale,
    _normalize_ext,
)
from .session import SessionState

try:
    import numpy as np
except Exception:  # pragma: no cover - defensive fallback for minimal envs
    class _NumpyStub:
        ndarray = list
        float32 = float
        int64 = int
        _is_stub = True

        def __getattr__(self, name: str) -> Any:
            raise ModuleNotFoundError(
                "numpy 모듈이 필요합니다. pip install numpy 로 설치 후 다시 시도해 주세요."
            )

    np = _NumpyStub()  # type: ignore[assignment]

try:
    from sentence_transformers import CrossEncoder
except Exception:
    CrossEncoder = None

logger = logging.getLogger(__name__)
if not logger.handlers:
    logger.addHandler(logging.NullHandler())


@dataclass
class EarlyStopConfig:
    score_threshold: float = 0.05
    window_size: int = 0
    patience: int = 2

    def create_state(self, batch_size: int) -> "EarlyStopState":
        window = self.window_size or batch_size
        return EarlyStopState(
            threshold=max(0.0, float(self.score_threshold)),
            window=max(1, int(window)),
            patience=max(1, int(self.patience)),
        )


@dataclass
class EarlyStopState:
    threshold: float
    window: int
    patience: int
    scores: Deque[float] = field(default_factory=deque)
    patience_hits: int = 0
    last_average: float = 0.0

    def observe(self, new_scores: Iterable[float]) -> bool:
        for score in new_scores:
            self.scores.append(float(score))
            if len(self.scores) > self.window:
                self.scores.popleft()
        if len(self.scores) < self.window:
            self.patience_hits = 0
            self.last_average = 0.0
            return False
        self.last_average = sum(self.scores) / len(self.scores)
        if self.last_average < self.threshold:
            self.patience_hits += 1
            if self.patience_hits >= self.patience:
                return True
        else:
            self.patience_hits = 0
        return False


class CrossEncoderReranker:
    def __init__(
        self,
        model_name: str,
        *,
        device: Optional[str] = None,
        batch_size: int = 16,
        early_stop: Optional[EarlyStopConfig] = None,
    ) -> None:
        if CrossEncoder is None:
            raise RuntimeError("sentence-transformers의 CrossEncoder를 사용할 수 없습니다.")
        allow_remote = (os.getenv("INFOPILOT_ALLOW_REMOTE_MODELS") or "").strip().lower() in {
            "1",
            "true",
            "yes",
            "on",
        }
        if not allow_remote:
            candidate_path = Path(model_name).expanduser()
            if not candidate_path.exists():
                cache_root = MODELS_DIR / "sentence_transformers" / f"models--{model_name.replace('/', '--')}"
                if not cache_root.exists():
                    raise RuntimeError(
                        f"reranker model is not available locally ({model_name}). "
                        "Disable rerank (--no-rerank) or download the model into models/sentence_transformers "
                        "(set INFOPILOT_ALLOW_REMOTE_MODELS=1 to allow remote downloads)."
                    )
        self.model_name = model_name
        self.device = device or None
        self.batch_size = max(1, int(batch_size) if batch_size else 1)
        self.early_stop_config = early_stop or EarlyStopConfig(window_size=self.batch_size)
        if self.early_stop_config.window_size <= 0:
            self.early_stop_config.window_size = self.batch_size

        load_kwargs: Dict[str, Any] = {}
        if self.device:
            load_kwargs["device"] = self.device

        t0 = time.time()
        try:
            self.model = CrossEncoder(model_name, **load_kwargs)
        except RuntimeError as exc:
            message = str(exc).lower()
            if "meta tensor" in message or "to_empty" in message:
                logger.warning(
                    "CrossEncoder load failed on device=%s; retrying on CPU. %s",
                    load_kwargs.get("device", "auto"),
                    exc,
                )
                load_kwargs["device"] = "cpu"
                self.device = "cpu"
                self.model = CrossEncoder(model_name, **load_kwargs)
            else:
                raise
        dt = time.time() - t0
        device_label = self.device or getattr(self.model, "device", "cpu")
        logger.info("reranker loaded: model=%s device=%s dt=%.1fs", model_name, device_label, dt)

    def rerank(
        self,
        query: str,
        hits: List[Dict[str, Any]],
        *,
        desired_exts: Optional[Set[str]] = None,
        session: Optional[SessionState] = None,
    ) -> List[Dict[str, Any]]:
        if not hits:
            return []

        pairs: List[List[str]] = []
        prepared_hits: List[Dict[str, Any]] = []
        for hit in hits:
            doc_text = _compose_rerank_document(hit)
            pairs.append([query, doc_text])
            prepared_hits.append(dict(hit))

        collected: List[float] = []
        try:
            for batch_scores in self._predict_iter(pairs):
                collected.extend(batch_scores)
        except Exception as exc:  # pragma: no cover - defensive path
            logger.warning("rerank inference failed, fallback to previous scores: %s", exc)
            return hits

        if not collected:
            return hits

        ext_preferences = desired_exts or set()
        rerank_raw = [float(s) for s in collected[: len(prepared_hits)]]
        vector_components = [float(h.get("vector_similarity", 0.0)) for h in prepared_hits]
        lexical_components = [float(h.get("lexical_score", 0.0)) for h in prepared_hits]

        rerank_scaled = _minmax_scale(rerank_raw)
        vector_scaled = _minmax_scale(vector_components)
        lexical_scaled = _minmax_scale(lexical_components)

        alpha, beta, gamma = 0.60, 0.25, 0.15
        combined_hits: List[Dict[str, Any]] = []

        for idx, hit in enumerate(prepared_hits):
            rerank_score = rerank_raw[idx] if idx < len(rerank_raw) else 0.0
            rerank_component = rerank_scaled[idx] if idx < len(rerank_scaled) else 0.0
            vector_component = vector_scaled[idx] if idx < len(vector_scaled) else 0.0
            lexical_component = lexical_scaled[idx] if idx < len(lexical_scaled) else 0.0

            ext = _normalize_ext(hit.get("ext"))
            total_ext_bonus, desired_ext_bonus, session_ext_bonus = _compute_extension_bonus(
                ext,
                ext_preferences,
                session,
            )
            owner_bonus = _compute_owner_bonus(hit.get("owner"), session)
            temporal_factor = float(hit.get("temporal_weight", 1.0))

            combined = (
                (alpha * rerank_component)
                + (beta * vector_component)
                + (gamma * lexical_component)
                + total_ext_bonus
                + owner_bonus
            )
            combined *= temporal_factor

            negative_penalty = float(hit.get("negative_penalty", 0.0) or 0.0)
            if negative_penalty > 0.0:
                combined = max(0.0, combined - negative_penalty)

            original_vector = float(hit.get("vector_similarity", hit.get("similarity", 0.0)))
            hit["vector_similarity"] = original_vector
            hit.setdefault("pre_rerank_score", float(hit.get("score", 0.0)))
            hit["rerank_score"] = float(rerank_score)
            hit["combined_score"] = float(combined)
            hit["score"] = float(combined)
            hit["similarity"] = original_vector
            hit["desired_extension_bonus"] = float(desired_ext_bonus)
            hit["session_ext_bonus"] = float(session_ext_bonus)
            hit["session_owner_bonus"] = float(owner_bonus)
            hit["temporal_weight"] = temporal_factor
            if total_ext_bonus:
                hit["rerank_ext_bonus"] = total_ext_bonus
            match_reasons = hit.get("match_reasons")
            if match_reasons is not None and session_ext_bonus:
                label = "세션 선호 확장자 가중치" if session_ext_bonus > 0 else "세션 비선호 확장자 페널티"
                if label not in match_reasons:
                    match_reasons.append(label)
            if match_reasons is not None and owner_bonus:
                owner_label = "세션 선호 작성자 가중치" if owner_bonus > 0 else "세션 비선호 작성자 페널티"
                if owner_label not in match_reasons:
                    match_reasons.append(owner_label)
            combined_hits.append(hit)

        combined_hits.sort(key=lambda item: item.get("score", 0.0), reverse=True)
        return combined_hits

    def _predict_iter(self, pairs: List[List[str]]) -> Iterable[np.ndarray]:
        total = len(pairs)
        if total <= self.batch_size:
            yield self.model.predict(
                pairs,
                batch_size=self.batch_size,
                convert_to_numpy=True,
                show_progress_bar=False,
            )
            return

        start = 0
        stop_state = self.early_stop_config.create_state(self.batch_size)
        while start < total:
            end = min(total, start + self.batch_size)
            batch = pairs[start:end]
            batch_scores = self.model.predict(
                batch,
                batch_size=self.batch_size,
                convert_to_numpy=True,
                show_progress_bar=False,
            )
            yield batch_scores
            if stop_state.observe(batch_scores):
                logger.info(
                    "reranker early stop: avg=%.4f window=%d processed=%d/%d",
                    stop_state.last_average,
                    stop_state.window,
                    end,
                    total,
                )
                break
            start = end

