from __future__ import annotations

import itertools
from typing import Any, Dict, Iterable, List

from core.data_pipeline.pipeline import DEFAULT_EMBED_MODEL, DEFAULT_N_COMPONENTS, TrainConfig


def build_train_config(args) -> TrainConfig:
    return TrainConfig(
        max_features=args.max_features,
        n_components=args.n_components,
        n_clusters=args.n_clusters,
        ngram_range=(1, 2),
        min_df=args.min_df,
        max_df=args.max_df,
        use_sentence_transformer=getattr(args, "use_embedding", True),
        embedding_model=getattr(args, "embedding_model", DEFAULT_EMBED_MODEL),
        embedding_batch_size=getattr(args, "embedding_batch_size", 32),
        async_embeddings=getattr(args, "async_embed", True),
        embedding_concurrency=max(1, int(getattr(args, "embedding_concurrency", 1))),
        embedding_dtype=getattr(args, "embedding_dtype", "auto"),
        embedding_chunk_size=max(0, int(getattr(args, "embedding_chunk_size", 0) or 0)),
        embedding_chunk_start=max(0, int(getattr(args, "embedding_chunk_start", 0) or 0)),
        embedding_chunk_end=int(getattr(args, "embedding_chunk_end", -1) or -1),
        embedding_subprocess_fallback=bool(getattr(args, "embedding_subprocess_fallback", True)),
    )


def maybe_limit_rows(rows: Iterable[Dict[str, Any]], limit: int) -> List[Dict[str, Any]]:
    iterator = iter(rows)
    if limit and limit > 0:
        limited = list(itertools.islice(iterator, limit))
        if next(iterator, None) is not None:
            print(f"⚡ 테스트 모드: 상위 {limit}개 파일만 사용합니다.")
        return limited
    return list(iterator)


def default_train_config() -> TrainConfig:
    return TrainConfig(
        max_features=50000,
        n_components=DEFAULT_N_COMPONENTS,
        n_clusters=25,
        ngram_range=(1, 2),
        min_df=2,
        max_df=0.85,
        use_sentence_transformer=True,
        embedding_model=DEFAULT_EMBED_MODEL,
        embedding_batch_size=32,
        embedding_chunk_size=0,
        embedding_chunk_start=0,
        embedding_chunk_end=-1,
        embedding_subprocess_fallback=True,
    )


__all__ = ["build_train_config", "default_train_config", "maybe_limit_rows"]

