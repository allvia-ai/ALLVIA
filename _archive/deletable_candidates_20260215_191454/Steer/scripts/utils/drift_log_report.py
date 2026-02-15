"""Summarize drift JSONL log entries with policy/cache metadata."""
from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Dict, List


def _tail_jsonl(path: Path, limit: int) -> List[Dict[str, object]]:
    if not path.exists():
        return []
    try:
        lines = path.read_text(encoding="utf-8").splitlines()
    except OSError:
        return []
    entries: List[Dict[str, object]] = []
    for raw in lines[-limit:]:
        raw = raw.strip()
        if not raw:
            continue
        try:
            payload = json.loads(raw)
        except json.JSONDecodeError:
            continue
        entries.append(payload)
    return entries


def main() -> None:
    parser = argparse.ArgumentParser(description="Summarize drift log")
    parser.add_argument("--log-path", default="artifacts/logs/drift_log.jsonl", help="Drift JSONL path")
    parser.add_argument("--limit", type=int, default=20, help="Entries to read")
    args = parser.parse_args()

    log_path = Path(args.log_path)
    entries = _tail_jsonl(log_path, args.limit)
    if not entries:
        print("로그가 없습니다.")
        return

    print(f"최근 {len(entries)}개 drift 로그 요약 ({log_path}):")
    for entry in entries:
        policy = entry.get("policy_id") or entry.get("policy_source")
        cache_action = entry.get("cache_action")
        ratio = entry.get("hash_drift_ratio")
        semantic = entry.get("semantic_shift")
        recs = entry.get("recommendations") or []
        print(
            f"- drift ratio={ratio} semantic={semantic} "
            f"policy={policy} cache_action={cache_action} recs={recs}"
        )


if __name__ == "__main__":
    main()
