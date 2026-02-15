"""Evaluate retrieval quality across different model configurations.

Usage example:

    PYTHONPATH=. python -m scripts.benchmarks.evaluate_retrieval \
      --config configs/eval_retrieval.json \
      --dataset data/eval/sample_queries.json \
      --top-k 5 \
      --output results/eval_run.json

The configuration file must contain a JSON object with an "evaluations" array.
Each element should minimally specify `name`, `model_path`, `corpus_path`,
and `cache_dir`. Optional fields are forwarded to LNPChat (e.g. llm_backend,
llm_model, min_similarity).
"""

from __future__ import annotations

import argparse
import json
import math
import statistics
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Dict, Iterable, List, Tuple

from core.conversation.lnp_chat import LNPChat


@dataclass
class EvaluationSpec:
    name: str
    model_path: Path
    corpus_path: Path
    cache_dir: Path
    llm_backend: str = ""
    llm_model: str = ""
    llm_host: str = ""
    min_similarity: float = 0.0
    top_k: int = 8
    include_refs: bool = False

    @classmethod
    def from_dict(cls, payload: Dict[str, Any]) -> "EvaluationSpec":
        name = payload.get("name")
        if not name:
            raise ValueError("evaluation spec missing 'name'")
        try:
            return cls(
                name=name,
                model_path=Path(payload["model_path"]),
                corpus_path=Path(payload["corpus_path"]),
                cache_dir=Path(payload.get("cache_dir", "data/index_cache")),
                llm_backend=payload.get("llm_backend", ""),
                llm_model=payload.get("llm_model", ""),
                llm_host=payload.get("llm_host", ""),
                min_similarity=float(payload.get("min_similarity", 0.0)),
                top_k=int(payload.get("top_k", 8)),
                include_refs=bool(payload.get("include_references", False)),
            )
        except KeyError as exc:
            raise ValueError(f"evaluation spec missing required key: {exc}") from exc


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Evaluate retrieval quality")
    parser.add_argument("--config", required=True, help="Path to evaluation config JSON")
    parser.add_argument("--dataset", required=True, help="Path to queries dataset JSON")
    parser.add_argument("--top-k", type=int, default=8, help="Evaluation cutoff (K) for metrics")
    parser.add_argument("--output", help="Optional path to save aggregated metrics JSON")
    return parser.parse_args()


def load_config(path: Path) -> List[EvaluationSpec]:
    payload = json.loads(path.read_text(encoding="utf-8"))
    evaluations = payload.get("evaluations")
    if not isinstance(evaluations, list) or not evaluations:
        raise ValueError("config file must contain non-empty 'evaluations' array")
    return [EvaluationSpec.from_dict(entry) for entry in evaluations]


def load_dataset(path: Path) -> List[Dict[str, Any]]:
    entries = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(entries, list) or not entries:
        raise ValueError("dataset must be a non-empty JSON array")
    for entry in entries:
        if "query" not in entry or "relevant" not in entry:
            raise ValueError("each dataset entry must contain 'query' and 'relevant'")
    return entries


def normalize_path(value: str) -> str:
    return str(Path(value).expanduser()).lower()


def evaluate_hits(hits: Iterable[str], relevant: Iterable[str], k: int) -> Dict[str, float]:
    hits_list = [normalize_path(h) for h in hits][:k]
    relevant_set = {normalize_path(r) for r in relevant if r}
    if not relevant_set:
        return {"precision": 0.0, "recall": 0.0, "ndcg": 0.0}

    match_flags = [1 if h in relevant_set else 0 for h in hits_list]
    tp = sum(match_flags)
    precision = tp / max(1, len(hits_list))
    recall = tp / len(relevant_set)

    # nDCG@k for binary relevance
    dcg = sum(rel / math.log2(idx + 2) for idx, rel in enumerate(match_flags))
    ideal_len = min(len(relevant_set), k)
    idcg = sum(1 / math.log2(idx + 2) for idx in range(ideal_len))
    ndcg = dcg / idcg if idcg > 0 else 0.0

    return {"precision": precision, "recall": recall, "ndcg": ndcg}


def run_single_evaluation(spec: EvaluationSpec, dataset: List[Dict[str, Any]], default_k: int) -> Dict[str, float]:
    start_time = time.time()
    chat = LNPChat(
        model_path=spec.model_path,
        corpus_path=spec.corpus_path,
        cache_dir=spec.cache_dir,
        llm_backend=spec.llm_backend,
        llm_model=spec.llm_model,
        llm_host=spec.llm_host,
        min_similarity=spec.min_similarity,
        rerank=False,
    )
    chat.ready()
    elapsed_init = time.time() - start_time

    metrics_per_query: List[Dict[str, float]] = []
    k = max(1, spec.top_k or default_k)
    for entry in dataset:
        query = entry["query"]
        relevant = entry["relevant"]
        result = chat.ask(query, topk=k)
        hits = [str(hit.get("path") or "") for hit in result.get("hits", [])]
        metrics = evaluate_hits(hits, relevant, k)
        metrics_per_query.append(metrics)

    # Aggregate
    precision_values = [m["precision"] for m in metrics_per_query]
    recall_values = [m["recall"] for m in metrics_per_query]
    ndcg_values = [m["ndcg"] for m in metrics_per_query]

    summary = {
        "name": spec.name,
        "count": len(dataset),
        "init_seconds": round(elapsed_init, 2),
        "k": k,
        "precision@k": round(statistics.mean(precision_values), 4),
        "recall@k": round(statistics.mean(recall_values), 4),
        "ndcg@k": round(statistics.mean(ndcg_values), 4),
    }
    return summary


def main() -> None:
    args = parse_args()
    config_path = Path(args.config)
    dataset_path = Path(args.dataset)

    evaluations = load_config(config_path)
    dataset = load_dataset(dataset_path)
    cutoff = max(1, args.top_k)

    results: List[Dict[str, Any]] = []
    for spec in evaluations:
        print(f"\n=== Evaluation: {spec.name} ===")
        summary = run_single_evaluation(spec, dataset, cutoff)
        k = summary.get("k", cutoff)
        print(
            f"precision@{k}: {summary['precision@k']:.4f} | "
            f"recall@{k}: {summary['recall@k']:.4f} | "
            f"ndcg@{k}: {summary['ndcg@k']:.4f} | "
            f"init {summary['init_seconds']}s"
        )
        results.append(summary)

    if args.output:
        output_path = Path(args.output)
        output_path.parent.mkdir(parents=True, exist_ok=True)
        output_path.write_text(json.dumps({"results": results}, indent=2), encoding="utf-8")
        print(f"\nSaved metrics to {output_path}")


if __name__ == "__main__":
    main()
