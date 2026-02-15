"""Lightweight offline evaluation metrics for embeddings."""

from __future__ import annotations

import math
from typing import Dict, Optional

import numpy as np

try:
    from sklearn.neighbors import NearestNeighbors
except Exception:  # pragma: no cover - optional
    NearestNeighbors = None  # type: ignore[assignment]


def evaluate_embeddings(
    embeddings: np.ndarray,
    topics: Optional[np.ndarray],
    *,
    topk: int = 5,
) -> Dict[str, float]:
    """Treat documents sharing the same topic as relevant and compute proxy P@K/nDCG."""

    if embeddings is None or embeddings.size == 0 or topics is None:
        return {"p_at_k": 0.0, "ndcg": 0.0}
    unique_topics = np.unique(topics)
    if len(unique_topics) <= 1 or NearestNeighbors is None:
        return {"p_at_k": 0.0, "ndcg": 0.0}

    k = min(topk + 1, max(2, embeddings.shape[0]))
    nbrs = NearestNeighbors(n_neighbors=k, metric="cosine")
    nbrs.fit(embeddings)
    distances, indices = nbrs.kneighbors(embeddings, return_distance=True)

    total_precision = 0.0
    total_ndcg = 0.0
    denom = embeddings.shape[0]
    ideal_dcg = _ideal_dcg(topk)

    for i in range(denom):
        label = topics[i]
        hits = 0
        dcg = 0.0
        rank = 0
        for idx in indices[i]:
            if idx == i:
                continue
            if rank >= topk:
                break
            relevance = 1.0 if topics[idx] == label else 0.0
            hits += relevance
            if relevance:
                dcg += 1.0 / math.log2(rank + 2)
            rank += 1
        total_precision += (hits / float(topk)) if topk else 0.0
        total_ndcg += (dcg / ideal_dcg) if ideal_dcg else 0.0

    return {
        "p_at_k": round(total_precision / max(1, denom), 4),
        "ndcg": round(total_ndcg / max(1, denom), 4),
    }


def _ideal_dcg(k: int) -> float:
    return sum(1.0 / math.log2(i + 2) for i in range(min(k, 1024)))
