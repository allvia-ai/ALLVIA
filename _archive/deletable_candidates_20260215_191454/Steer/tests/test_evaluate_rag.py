from pathlib import Path

import pytest

from scripts.evaluate_rag import evaluate


class DummyRetriever:
    def __init__(self, mapping):
        self.mapping = mapping

    def search(self, query, top_k=5):
        return self.mapping.get(query, [])[:top_k]


@pytest.mark.full
def test_evaluate_reports_accuracy():
    retriever = DummyRetriever(
        {
            "hello": [{"path": "a"}],
            "world": [{"path": "b"}, {"path": "c"}],
        }
    )
    cases = [
        {"query": "hello", "expected": ["a"]},
        {"query": "world", "expected": ["c"]},
        {"query": "missing", "expected": ["x"]},
    ]
    result = evaluate(retriever, cases, top_k=2)
    assert result["total"] == 3
    assert result["top1_acc"] == pytest.approx(1 / 3)
    assert result["topk_acc"] == pytest.approx(2 / 3)
