import numpy as np
import pytest

from core.search.retriever import QueryResultCache, SemanticQueryCache


@pytest.mark.full
def test_query_result_cache_eviction() -> None:
    cache = QueryResultCache(max_entries=2)
    cache.set("a", [1])
    cache.set("b", [2])
    assert cache.get("a") == [1]
    cache.set("c", [3])
    assert cache.get("b") is None  # least recently used evicted
    assert cache.get("a") == [1]
    assert cache.get("c") == [3]


@pytest.mark.full
def test_query_result_cache_returns_copy() -> None:
    cache = QueryResultCache(max_entries=1)
    data = [{"x": 1}]
    cache.set("k", data)
    cached = cache.get("k")
    assert cached == data
    cached.append({"y": 2})
    # Ensure internal value not mutated
    assert cache.get("k") == data


@pytest.mark.full
def test_semantic_query_cache_matches_similar_vectors() -> None:
    cache = SemanticQueryCache(max_entries=2, threshold=0.95)
    cache.store(np.array([1.0, 0.0], dtype=np.float32), [{"id": 1}])
    result = cache.match(np.array([0.99, 0.01], dtype=np.float32))
    assert result == [{"id": 1}]


@pytest.mark.full
def test_semantic_query_cache_returns_none_for_low_similarity() -> None:
    cache = SemanticQueryCache(max_entries=2, threshold=0.99)
    cache.store(np.array([1.0, 0.0], dtype=np.float32), [{"id": 1}])
    assert cache.match(np.array([0.8, 0.6], dtype=np.float32)) is None
