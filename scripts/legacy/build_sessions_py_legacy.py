from __future__ import annotations

import argparse
from datetime import timedelta
from pathlib import Path

import sys

# Ensure local src is importable when running as a script.
PROJECT_ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(PROJECT_ROOT / "src"))

from collector.config import load_config
from collector.sessionizer import build_session_records, rows_to_events, sessionize
from collector.store import SQLiteStore
from collector.utils.time import parse_ts, utc_now


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Build sessions from stored events")
    parser.add_argument(
        "--config", default="configs/config.yaml", help="path to config file"
    )
    parser.add_argument("--start", default="", help="start ts (ISO, optional)")
    parser.add_argument("--end", default="", help="end ts (ISO, optional)")
    parser.add_argument(
        "--since-hours",
        type=float,
        default=0.0,
        help="start from now minus N hours (optional)",
    )
    parser.add_argument(
        "--gap-minutes",
        type=int,
        default=15,
        help="gap threshold in minutes",
    )
    parser.add_argument(
        "--use-state",
        action="store_true",
        help="resume from last_sessionized_ts in state table",
    )
    parser.add_argument("--dry-run", action="store_true", help="do not insert")
    return parser.parse_args()


def _format_ts(value) -> str:
    return value.isoformat().replace("+00:00", "Z")


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
    if args.since_hours and not start_ts:
        start_ts = (utc_now() - timedelta(hours=args.since_hours)).isoformat().replace(
            "+00:00", "Z"
        )
    if args.use_state and not start_ts and not args.since_hours:
        last_ts = store.get_state("last_sessionized_ts")
        if last_ts:
            parsed = parse_ts(last_ts)
            if parsed is not None:
                start_ts = _format_ts(parsed + timedelta(microseconds=1))

    rows = store.fetch_events(start_ts=start_ts, end_ts=end_ts)
    events = rows_to_events(rows)
    sessions = sessionize(events, gap_seconds=args.gap_minutes * 60)
    records = build_session_records(sessions)

    if args.dry_run:
        print(f"sessions_ready={len(records)} dry_run=true")
        store.close()
        return

    for record in records:
        store.insert_session(
            record.session_id,
            record.start_ts,
            record.end_ts,
            record.duration_sec,
            record.summary_json,
        )

    if args.use_state and records:
        store.set_state("last_sessionized_ts", records[-1].end_ts)

    print(f"sessions_inserted={len(records)}")
    store.close()


if __name__ == "__main__":
    main()
