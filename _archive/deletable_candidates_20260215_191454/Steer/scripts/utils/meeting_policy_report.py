"""Generate a brief report of meeting analytics with policy/cache info."""
from __future__ import annotations

import argparse
import json
from pathlib import Path


def main() -> None:
    parser = argparse.ArgumentParser(description="Summarize meeting analytics with policy info")
    parser.add_argument("--analytics-dir", default="data/meetings", help="Analytics directory root")
    parser.add_argument("--limit", type=int, default=10, help="Max entries to show")
    args = parser.parse_args()

    root = Path(args.analytics_dir).expanduser()
    items = sorted(root.glob("**/analytics.json"), key=lambda p: p.stat().st_mtime, reverse=True)
    if not items:
        print("No meeting analytics found.")
        return

    for item in items[: args.limit]:
        try:
            entry = json.loads(item.read_text(encoding="utf-8"))
        except json.JSONDecodeError:
            continue
        policy_tag = entry.get("policy_tag")
        cache_hit = entry.get("cache_hit")
        print(
            f"- {item}: language={entry.get('language')} "
            f"duration={entry.get('duration_seconds')} "
            f"actions={len(entry.get('action_items') or [])} "
            f"policy={policy_tag} cache_hit={cache_hit}"
        )


if __name__ == "__main__":
    main()
