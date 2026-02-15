"""Gather meeting training queue entries into a consolidated queue."""

from __future__ import annotations

import argparse
import json
from pathlib import Path


def _load_jsonl(path: Path) -> list[dict]:
    if not path.exists():
        return []
    items: list[dict] = []
    for line in path.read_text(encoding="utf-8").splitlines():
        if not line.strip():
            continue
        try:
            items.append(json.loads(line))
        except json.JSONDecodeError:
            continue
    return items


def _write_jsonl(path: Path, entries: list[dict]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(
        "\n".join(json.dumps(entry, ensure_ascii=False) for entry in entries),
        encoding="utf-8",
    )


def consolidate(*, source_root: Path, output_dir: Path, overwrite: bool = False) -> Path:
    """Collect per-meeting queues under source_root into output_dir."""

    queues: list[dict] = []

    for path in source_root.rglob("training_queue.jsonl"):
        entries = _load_jsonl(path)
        if not entries:
            continue
        queues.extend(entries)

    output_dir.mkdir(parents=True, exist_ok=True)
    target_path = output_dir / "training_queue.jsonl"

    if target_path.exists() and not overwrite:
        existing = _load_jsonl(target_path)
        queues = existing + queues

    _write_jsonl(target_path, queues)
    return target_path


def _build_argparser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source-root", type=Path, required=True, help="Root directory containing meeting outputs")
    parser.add_argument("--output-dir", type=Path, required=True, help="Directory where consolidated queue will be stored")
    parser.add_argument("--overwrite", action="store_true", help="Replace existing output queue instead of appending")
    return parser


def main() -> None:
    args = _build_argparser().parse_args()
    target = consolidate(source_root=args.source_root, output_dir=args.output_dir, overwrite=args.overwrite)
    print(f"Consolidated queue written to {target}")


if __name__ == "__main__":  # pragma: no cover - CLI entry
    main()

