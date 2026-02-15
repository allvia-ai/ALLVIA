"""CLI utility to summarise desktop agent audit logs."""
from __future__ import annotations

import argparse
import json
from collections import Counter, defaultdict
from pathlib import Path
from typing import Dict

from scripts.audit_log import DEFAULT_AUDIT_PATH


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Summarise desktop agent audit events")
    parser.add_argument(
        "--log",
        default=str(DEFAULT_AUDIT_PATH),
        help="JSONL audit log path (default: artifacts/logs/audit/desktop_agents.jsonl)",
    )
    parser.add_argument("--json", action="store_true", help="Print raw JSON summary")
    return parser


def summarise(path: Path) -> Dict[str, Dict[str, int]]:
    summary: Dict[str, Dict[str, int]] = defaultdict(lambda: defaultdict(int))
    if not path.exists():
        return summary
    with path.open("r", encoding="utf-8") as handle:
        for line in handle:
            line = line.strip()
            if not line:
                continue
            try:
                payload = json.loads(line)
            except json.JSONDecodeError:
                continue
            agent = str(payload.get("agent") or "unknown")
            status = str(payload.get("status") or "unknown")
            summary[agent][status] += 1
    return summary


def main() -> None:
    parser = build_parser()
    args = parser.parse_args()
    log_path = Path(args.log).expanduser()
    summary = summarise(log_path)
    if args.json:
        print(json.dumps(summary, ensure_ascii=False, indent=2))
        return
    if not summary:
        print(f"No audit events found ({log_path})")
        return
    print(f"Audit summary for {log_path}")
    for agent, counts in sorted(summary.items()):
        total = sum(counts.values())
        breakdown = ", ".join(f"{status}={count}" for status, count in sorted(counts.items()))
        print(f"- {agent}: {total} events ({breakdown})")


if __name__ == "__main__":
    main()
