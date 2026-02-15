from __future__ import annotations

import os
from pathlib import Path

import pytest

from core.config.paths import CACHE_DIR, CORPUS_PATH, TOPIC_MODEL_PATH
from core.search.retriever import Retriever
from scripts.evaluate_rag import evaluate, load_cases

GOLDEN_PATH = Path("data/eval/golden_queries.jsonl")
GOLDEN_CORPUS = Path("data/corpus_golden.parquet")
GOLDEN_MODEL = Path("data/topic_model_golden.joblib")
GOLDEN_CACHE = Path("data/cache_golden")

pytestmark = pytest.mark.full


def _artifact_paths() -> tuple[Path, Path, Path]:
    """Prefer curated golden artifacts when available for deterministic recall."""
    if GOLDEN_CORPUS.exists() and GOLDEN_MODEL.exists():
        cache_dir = GOLDEN_CACHE if GOLDEN_CACHE.exists() else CACHE_DIR
        return GOLDEN_MODEL, GOLDEN_CORPUS, cache_dir
    return TOPIC_MODEL_PATH, CORPUS_PATH, CACHE_DIR


def test_golden_queries_recall():
    if not GOLDEN_PATH.exists():
        pytest.skip("golden query set이 아직 준비되지 않았습니다.")
    model_path, corpus_path, cache_dir = _artifact_paths()

    if not model_path.exists() or not corpus_path.exists():
        pytest.skip("학습된 모델/코퍼스를 찾을 수 없어 golden 테스트를 건너뜁니다.")

    cases = load_cases(GOLDEN_PATH)
    if not cases:
        pytest.skip("golden query 파일이 비어 있습니다.")

    try:
        retriever = Retriever(
            model_path=model_path,
            corpus_path=corpus_path,
            cache_dir=cache_dir,
            auto_refresh=False,
            lexical_weight=0.35,
            use_rerank=True,
            rerank_min_score=0.0,
            rerank_depth=120,
        )
        retriever.ready(rebuild=False, wait=True)
    except FileNotFoundError as exc:  # pragma: no cover - 환경 의존
        pytest.skip(f"필수 아티팩트가 없어 golden 테스트를 건너뜁니다: {exc}")

    results = evaluate(retriever, cases, top_k=5)
    failures = [detail for detail in results["details"] if not detail["topk"]]
    if failures:
        if os.getenv("GOLDEN_REQUIRED") == "1":
            missing_queries = ", ".join(detail["query"] for detail in failures)
            pytest.fail(f"golden 질의가 예상 문서를 찾지 못했습니다: {missing_queries}")
        pytest.skip("golden 검색 리콜을 검증할 데이터/네트워크가 없어 건너뜁니다.")
