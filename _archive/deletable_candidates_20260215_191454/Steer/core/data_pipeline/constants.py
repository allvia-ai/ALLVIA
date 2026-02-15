"""Shared constants/utilities for `core.data_pipeline`."""

from __future__ import annotations

import importlib
import os
import platform
import re
from pathlib import Path
from typing import Optional

from core.config.paths import MODELS_DIR
from core.data_pipeline.cache_manager import ChunkCache, SQLiteChunkCache

try:  # pragma: no cover - optional dependency
    import pandas as pd
except Exception:  # pragma: no cover - optional dependency
    pd = None  # type: ignore[assignment]


PARQUET_ENGINE: Optional[str] = None
if pd is not None:
    for candidate in ("fastparquet", "pyarrow"):
        try:
            importlib.import_module(candidate)
            PARQUET_ENGINE = candidate
            break
        except ImportError:
            continue


TOKEN_PATTERN = r"(?u)(?:[가-힣]{1,}|[A-Za-z0-9]{2,})"

# 고정된 SVD 차원 수. Index/모델 불일치를 막기 위해 한곳에서 정의한다.
DEFAULT_N_COMPONENTS = 128
MODEL_TEXT_COLUMN = "text_model"
_META_SPLIT_RE = re.compile(r"[^0-9A-Za-z가-힣]+")


def _default_embed_model() -> str:
    env_model = os.getenv("DEFAULT_EMBED_MODEL")
    if env_model:
        return env_model
    if platform.system() == "Darwin":
        # Prefer the bundled multilingual-e5-small copy on macOS for stability
        return "models--intfloat--multilingual-e5-small"
    return "BAAI/bge-m3"


DEFAULT_EMBED_MODEL = _default_embed_model()
MODEL_TYPE_SENTENCE_TRANSFORMER = "sentence-transformer"


def _normalize_hf_model_id(value: str) -> str:
    """Convert cache-style ids like `models--org--repo` into `org/repo`."""
    raw = (value or "").strip()
    if not raw:
        return raw
    if raw.startswith("models--") and "/" not in raw:
        parts = raw.split("--")
        if len(parts) >= 3:
            org = parts[1].strip()
            repo = "--".join(parts[2:]).strip()
            if org and repo:
                return f"{org}/{repo}"
    return raw


def _resolve_sentence_transformer_location(model_name: str) -> str:
    """Prefer local model snapshots under `models/sentence_transformers/` when available."""
    base_dir = MODELS_DIR / "sentence_transformers"
    if not base_dir.exists():
        return model_name

    direct = base_dir / model_name
    if direct.exists():
        snapshots = direct / "snapshots"
        if snapshots.exists():
            candidates = sorted(
                snapshots.iterdir(),
                key=lambda item: item.stat().st_mtime,
                reverse=True,
            )
            for candidate in candidates:
                if any(
                    (candidate / marker).exists()
                    for marker in (
                        "config.json",
                        "modules.json",
                        "config_sentence_transformers.json",
                    )
                ):
                    return str(candidate)
        if any(
            (direct / marker).exists()
            for marker in ("config.json", "modules.json", "config_sentence_transformers.json")
        ):
            return str(direct)

    cache_root = base_dir / f"models--{model_name.replace('/', '--')}"
    if cache_root.exists():
        snapshots = cache_root / "snapshots"
        if snapshots.exists():
            candidates = sorted(
                snapshots.iterdir(),
                key=lambda item: item.stat().st_mtime,
                reverse=True,
            )
            for candidate in candidates:
                if any(
                    (candidate / marker).exists()
                    for marker in (
                        "config.json",
                        "modules.json",
                        "config_sentence_transformers.json",
                    )
                ):
                    return str(candidate)
        if any(
            (cache_root / marker).exists()
            for marker in ("config.json", "modules.json", "config_sentence_transformers.json")
        ):
            return str(cache_root)

    return model_name


DEFAULT_CHUNK_MIN_TOKENS = 200
DEFAULT_CHUNK_MAX_TOKENS = 500

EMBED_DTYPE_ENV = "INFOPILOT_EMBED_DTYPE"
_VALID_EMBED_DTYPES = {"auto", "fp16", "fp32"}
CACHE_BACKEND_ENV = "INFOPILOT_CACHE_BACKEND"
_VALID_CACHE_BACKENDS = {"json", "sqlite"}


def _sanitize_embed_dtype(value: Optional[str]) -> Optional[str]:
    if value is None:
        return None
    normalized = str(value).strip().lower()
    return normalized if normalized in _VALID_EMBED_DTYPES else None


def _sanitize_cache_backend(value: Optional[str]) -> str:
    if not value:
        return "json"
    normalized = str(value).strip().lower()
    return normalized if normalized in _VALID_CACHE_BACKENDS else "json"


def _create_chunk_cache(path: Path) -> ChunkCache:
    backend = _sanitize_cache_backend(os.getenv(CACHE_BACKEND_ENV))
    actual_path = path
    if backend == "sqlite":
        if actual_path.suffix.lower() == ".json":
            actual_path = actual_path.with_suffix(".sqlite")
        elif not actual_path.name.endswith(".sqlite"):
            actual_path = actual_path.with_name(actual_path.name + ".sqlite")
    if backend == "sqlite":
        print(f"⚙️ Chunk cache: SQLite backend → {actual_path}", flush=True)
        return SQLiteChunkCache(actual_path)
    if backend != "json":
        print(f"⚠️ 지원하지 않는 캐시 백엔드 '{backend}' → json으로 대체합니다.", flush=True)
    return ChunkCache(path)

