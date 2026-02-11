from __future__ import annotations

import argparse
import json
import sys
from datetime import timedelta
from pathlib import Path

# Ensure local src is importable when running as a script.
PROJECT_ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(PROJECT_ROOT / "src"))

from collector.config import load_config
from collector.routine import build_routine_candidates, rows_to_sessions
from collector.store import SQLiteStore
from collector.utils.time import parse_ts, utc_now


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Build routine candidates from sessions")
    parser.add_argument(
        "--config", default="configs/config.yaml", help="path to config file"
    )
    parser.add_argument("--start", default="", help="start ts (ISO, optional)")
    parser.add_argument("--end", default="", help="end ts (ISO, optional)")
    parser.add_argument(
        "--days",
        type=float,
        default=7.0,
        help="look back N days for sessions",
    )
    parser.add_argument("--n-min", type=int, default=2, help="min n-gram length")
    parser.add_argument("--n-max", type=int, default=5, help="max n-gram length")
    parser.add_argument(
        "--min-support", type=int, default=2, help="min support threshold"
    )
    parser.add_argument(
        "--max-patterns", type=int, default=100, help="max patterns to store"
    )
    parser.add_argument(
        "--max-evidence", type=int, default=10, help="max evidence session ids"
    )
    parser.add_argument(
        "--use-state",
        action="store_true",
        help="skip if no new sessions since last_routine_ts",
    )
    parser.add_argument("--dry-run", action="store_true", help="do not insert")
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    config = load_config(args.config)
    store = SQLiteStore(
        config.db_path,
        wal_mode=config.wal_mode,
        busy_timeout_ms=config.store.busy_timeout_ms,
    )
    store.connect()
    store.migrate(config.migrations_path)

    start_ts = args.start.strip() or None
    end_ts = args.end.strip() or None
    if args.days and not start_ts:
        start_ts = (utc_now() - timedelta(days=args.days)).isoformat().replace(
            "+00:00", "Z"
        )

    latest_end_ts = store.fetch_latest_session_end_ts()
    if args.use_state and latest_end_ts:
        last_ts = store.get_state("last_routine_ts")
        if last_ts:
            last_parsed = parse_ts(last_ts)
            latest_parsed = parse_ts(latest_end_ts)
            if last_parsed and latest_parsed and latest_parsed <= last_parsed:
                print("routine_candidates_skipped=unchanged")
                store.close()
                return

    rows = store.fetch_sessions(start_ts=start_ts, end_ts=end_ts)
    sessions = rows_to_sessions(rows)
    candidates = build_routine_candidates(
        sessions,
        n_min=args.n_min,
        n_max=args.n_max,
        min_support=args.min_support,
        max_patterns=args.max_patterns,
        max_evidence=args.max_evidence,
    )

    if args.dry_run:
        print(f"routine_candidates_ready={len(candidates)} dry_run=true")
        store.close()
        return

    store.clear_routine_candidates()
    for candidate in candidates:
        store.insert_routine_candidate(
            candidate.pattern_id,
            candidate.pattern_json,
            candidate.support,
            candidate.confidence,
            candidate.last_seen_ts,
            json.dumps(candidate.evidence_session_ids, separators=(",", ":")),
        )

    if args.use_state and latest_end_ts:
        store.set_state("last_routine_ts", latest_end_ts)

    print(f"routine_candidates_inserted={len(candidates)}")
    store.close()


if __name__ == "__main__":
    main()
