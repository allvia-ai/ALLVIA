# query_encoder.py - Extracted from retriever.py (GOD CLASS refactoring)
"""Query/document embedding encoder for hybrid retrieval."""

from __future__ import annotations

import importlib
import logging
import os
import platform
import sys
import types
from pathlib import Path
from typing import Any, List, Optional

from core.config.paths import MODELS_DIR

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
    import joblib
except Exception:
    joblib = None

try:
    from sentence_transformers import SentenceTransformer
except Exception:
    SentenceTransformer = None

logger = logging.getLogger(__name__)
if not logger.handlers:
    logger.addHandler(logging.NullHandler())

MODEL_TEXT_COLUMN = "text_model"
MODEL_TYPE_SENTENCE_TRANSFORMER = "sentence-transformer"

TORCH_META_HINT = """\
SentenceTransformer 초기화 중 torch 메타 텐서 오류가 발생했습니다.
현재 설치된 PyTorch 빌드가 SentenceTransformer와 호환되지 않습니다.

macOS/CPU 환경에서는 아래 명령으로 권장 버전을 설치한 뒤 다시 시도해 주세요.

  pip install --upgrade --no-cache-dir \\
      torch==2.3.0 torchvision==0.18.0 torchaudio==2.3.0 \\
      --index-url https://download.pytorch.org/whl/cpu

설치 후 `python infopilot.py run train` 또는 데스크톱 앱을 다시 실행하면 문제를 해결할 수 있습니다.
"""


def _default_embed_model() -> str:
    env_model = os.getenv("DEFAULT_EMBED_MODEL")
    if env_model:
        return env_model
    if platform.system() == "Darwin":
        return "models--intfloat--multilingual-e5-small"
    return "BAAI/bge-m3"


DEFAULT_EMBED_MODEL = _default_embed_model()


def _resolve_sentence_transformer_location(model_name: str) -> str:
    """Prefer locally cached SentenceTransformer snapshots when available."""
    base_dir = MODELS_DIR / "sentence_transformers"
    if base_dir.exists():
        direct = base_dir / model_name
        if direct.exists():
            snapshots = direct / "snapshots"
            if snapshots.exists():
                candidates = sorted(snapshots.iterdir(), key=lambda item: item.stat().st_mtime, reverse=True)
                for candidate in candidates:
                    if any(
                        (candidate / marker).exists()
                        for marker in ("config.json", "modules.json", "config_sentence_transformers.json")
                    ):
                        return str(candidate)

            if any(
                (direct / marker).exists()
                for marker in ("config.json", "modules.json", "config_sentence_transformers.json")
            ):
                return str(direct)

        cache_dir = base_dir / f"models--{model_name.replace('/', '--')}"
        snapshots = cache_dir / "snapshots"
        if snapshots.exists():
            candidates = sorted(snapshots.iterdir(), key=lambda item: item.stat().st_mtime, reverse=True)
            for candidate in candidates:
                if any(
                    (candidate / marker).exists()
                    for marker in ("config.json", "modules.json", "config_sentence_transformers.json")
                ):
                    return str(candidate)
    return model_name


def _alias_legacy_modules() -> bool:
    candidates = ["pipeline", "step2_module_progress", "step2_pipeline", "Step2_module_progress"]
    for name in candidates:
        try:
            mod = importlib.import_module(name)
            sys.modules["TextCleaner"] = mod
            return True
        except Exception:
            continue
    shim = types.ModuleType("TextCleaner")
    sys.modules["TextCleaner"] = shim
    return False


class QueryEncoder:
    def __init__(self, model_path: Path):
        if joblib is None:
            raise RuntimeError("joblib이 필요합니다. pip install joblib")

        try:
            obj = joblib.load(model_path)
        except ModuleNotFoundError as e:
            if "TextCleaner" in str(e):
                logger.warning("legacy model detected; injecting TextCleaner alias and retrying")
                _alias_legacy_modules()
                obj = joblib.load(model_path)
            else:
                raise

        self.model_type = MODEL_TYPE_SENTENCE_TRANSFORMER
        self.embedding_dim: Optional[int] = None
        self.embedder: Optional[SentenceTransformer] = None
        self.pipeline = None
        self.tfidf = None
        self.svd = None

        if isinstance(obj, dict) and obj.get("model_type") == MODEL_TYPE_SENTENCE_TRANSFORMER:
            if SentenceTransformer is None:
                raise RuntimeError(
                    "sentence-transformers 라이브러리가 필요합니다. pip install sentence-transformers"
                )
            model_name = obj.get("model_name") or DEFAULT_EMBED_MODEL
            resolved_model = _resolve_sentence_transformer_location(model_name)
            if resolved_model != model_name:
                logger.info("Sentence-BERT loaded (local): %s -> %s", model_name, resolved_model)
            else:
                logger.info("Sentence-BERT loaded: %s", model_name)
            try:
                self.embedder = SentenceTransformer(resolved_model)
            except (RuntimeError, NotImplementedError) as exc:
                message = str(exc).lower()
                meta_issue = "meta tensor" in message or "to_empty" in message
                if meta_issue:
                    logger.warning("SentenceTransformer auto-device load failed; falling back to CPU. %s", exc)
                    try:
                        self.embedder = SentenceTransformer(resolved_model, device="cpu")
                    except Exception as inner_exc:
                        raise RuntimeError(TORCH_META_HINT) from inner_exc
                else:
                    raise
            detected_dim = obj.get("embedding_dim")
            if detected_dim:
                try:
                    self.embedding_dim = int(detected_dim)
                except (TypeError, ValueError):
                    self.embedding_dim = self.embedder.get_sentence_embedding_dimension()
            else:
                self.embedding_dim = self.embedder.get_sentence_embedding_dimension()
            self.cluster_model = obj.get("cluster_model")
            self.train_config = obj.get("train_config")
        else:
            self.model_type = "tfidf"
            self.pipeline = obj["pipeline"]
            self.tfidf = self.pipeline.named_steps["tfidf"]
            self.svd = self.pipeline.named_steps["svd"]
            self.embedding_dim = getattr(self.svd, "n_components", None)
            self.cluster_model = None
            self.train_config = obj.get("cfg") if isinstance(obj, dict) else None

    @staticmethod
    def _sanitize_texts(texts: List[Any]) -> List[str]:
        cleaned: List[str] = []
        for raw in texts:
            if raw is None:
                cleaned.append("")
                continue
            cleaned.append(str(raw))
        return cleaned

    def encode_docs(self, texts: List[str]) -> np.ndarray:
        texts = self._sanitize_texts(texts)
        if self.model_type == MODEL_TYPE_SENTENCE_TRANSFORMER and self.embedder is not None:
            batch_size = int(os.getenv("INDEX_EMBED_BATCH", "16") or 16)
            Z = self.embedder.encode(
                texts,
                batch_size=max(1, batch_size),
                show_progress_bar=False,
                convert_to_numpy=True,
                normalize_embeddings=True,
            )
            return np.asarray(Z, dtype=np.float32)

        if self.tfidf is None or self.svd is None:
            raise RuntimeError("TF-IDF 파이프라인이 초기화되지 않았습니다.")
        X = self.tfidf.transform(texts)
        Z = self.svd.transform(X)
        return Z.astype(np.float32, copy=False)

    def encode_query(self, query: str) -> np.ndarray:
        clean_query = self._sanitize_texts([query])
        if self.model_type == MODEL_TYPE_SENTENCE_TRANSFORMER and self.embedder is not None:
            batch_size = int(os.getenv("INDEX_QUERY_BATCH", "4") or 4)
            Zq = self.embedder.encode(
                clean_query,
                batch_size=max(1, batch_size),
                show_progress_bar=False,
                convert_to_numpy=True,
                normalize_embeddings=True,
            )
            return np.asarray(Zq, dtype=np.float32)

        if self.tfidf is None or self.svd is None:
            raise RuntimeError("TF-IDF 파이프라인이 초기화되지 않았습니다.")
        Xq = self.tfidf.transform(clean_query)
        Zq = self.svd.transform(Xq)
        return Zq.astype(np.float32, copy=False)
