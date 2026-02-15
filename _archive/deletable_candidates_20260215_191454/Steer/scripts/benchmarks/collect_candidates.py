"""Collect top-k candidate documents for a list of queries.

The script helps with manual labelling: it runs the current retrieval
pipeline and dumps the top matches to JSON so that you can mark which
ones are truly relevant.

Usage:

    PYTHONPATH=. python -m scripts.benchmarks.collect_candidates \
      --model data/topic_model.joblib \
      --corpus data/corpus.parquet \
      --cache data/index_cache \
      --queries data/eval/queries_to_label.json \
      --out data/eval/candidate_dump.json \
      --top-k 10
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any, Dict, List

from core.conversation.lnp_chat import LNPChat


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Collect candidate documents for labelling")
    parser.add_argument("--model", required=True, help="Path to trained topic model (.joblib)")
    parser.add_argument("--corpus", required=True, help="Path to corpus parquet file")
    parser.add_argument("--cache", required=True, help="Path to cache directory")
    parser.add_argument("--queries", required=True, help="JSON array of query strings")
    parser.add_argument("--out", required=True, help="Output JSON path")
    parser.add_argument("--top-k", type=int, default=15, help="Number of candidates per query")
    return parser.parse_args()


def load_queries(path: Path) -> List[str]:
    payload = json.loads(path.read_text(encoding="utf-8"))
    if isinstance(payload, dict):
        payload = payload.get("queries")
    if not isinstance(payload, list):
        raise ValueError("queries file must be a list of strings or an object with 'queries'")
    queries = [str(q).strip() for q in payload if q]
    if not queries:
        raise ValueError("queries list is empty")
    return queries


def main() -> None:
    args = parse_args()
    model = Path(args.model)
    corpus = Path(args.corpus)
    cache = Path(args.cache)
    queries_path = Path(args.queries)
    out_path = Path(args.out)
    top_k = max(1, int(args.top_k))

    queries = load_queries(queries_path)

    chat = LNPChat(
        model_path=model,
        corpus_path=corpus,
        cache_dir=cache,
        llm_backend="",
        rerank=False,
    )
    chat.ready()

    dump: List[Dict[str, Any]] = []
    for query in queries:
        result = chat.ask(query, topk=top_k)
        hits = []
        for hit in result.get("hits", [])[:top_k]:
            hits.append(
                {
                    "path": hit.get("path"),
                    "similarity": hit.get("similarity"),
                    "preview": (hit.get("preview") or "")[:200],
                }
            )
        dump.append({"query": query, "candidates": hits})

    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(json.dumps(dump, ensure_ascii=False, indent=2), encoding="utf-8")
    print(f"Saved candidates for {len(dump)} queries → {out_path}")


if __name__ == "__main__":
    main()
