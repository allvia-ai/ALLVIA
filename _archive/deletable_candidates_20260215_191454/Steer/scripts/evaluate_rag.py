"""Simple evaluation loop for Retriever accuracy using labeled queries."""
from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Iterable, List, Sequence

from core.search.retriever import Retriever


def load_cases(path: Path) -> List[dict]:
    if not path.exists():
        raise FileNotFoundError(f"evaluation cases not found: {path}")
    cases: List[dict] = []
    with path.open("r", encoding="utf-8") as fh:
        for line in fh:
            line = line.strip()
            if not line:
                continue
            data = json.loads(line)
            if "query" not in data:
                raise ValueError(f"case missing 'query': {data}")
            expected = data.get("expected") or []
            if isinstance(expected, str):
                expected = [expected]
            data["expected"] = list(expected)
            cases.append(data)
    return cases


def evaluate(retriever: Retriever, cases: Sequence[dict], *, top_k: int) -> dict:
    total = len(cases)
    if total == 0:
        raise ValueError("no evaluation cases provided")
    top1 = 0
    topk = 0
    detailed: List[dict] = []

    for case in cases:
        query = case["query"]
        expected: Iterable[str] = case.get("expected", [])
        hits = retriever.search(query, top_k=top_k)
        paths = [hit.get("path") for hit in hits]
        match_topk = any(path in expected for path in paths if path)
        match_top1 = bool(paths) and paths[0] in expected
        topk += int(match_topk)
        top1 += int(match_top1)
        detailed.append(
            {
                "query": query,
                "expected": expected,
                "hits": paths,
                "top1": bool(match_top1),
                "topk": bool(match_topk),
            }
        )

    return {
        "total": total,
        "top1_acc": top1 / total,
        "topk_acc": topk / total,
        "details": detailed,
    }


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cases", type=Path, required=True, help="JSONL file with evaluation queries")
    parser.add_argument("--model", type=Path, required=True, help="model path (joblib)")
    parser.add_argument("--corpus", type=Path, required=True, help="corpus parquet path")
    parser.add_argument("--cache", type=Path, required=True, help="index cache directory")
    parser.add_argument("--top-k", type=int, default=5, help="maximum k to evaluate")
    return parser


def main() -> None:
    parser = build_parser()
    args = parser.parse_args()

    cases = load_cases(args.cases)

    retriever = Retriever(
        model_path=args.model,
        corpus_path=args.corpus,
        cache_dir=args.cache,
        auto_refresh=False,
    )
    retriever.ready(rebuild=False, wait=True)

    results = evaluate(retriever, cases, top_k=max(1, args.top_k))
    summary = {
        "total": results["total"],
        "top1_acc": round(results["top1_acc"], 4),
        "topk_acc": round(results["topk_acc"], 4),
    }
    print(json.dumps(summary, ensure_ascii=False, indent=2))


if __name__ == "__main__":  # pragma: no cover - CLI entry point
    main()
