from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

# Ensure local src is importable when running as a script.
PROJECT_ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(PROJECT_ROOT / "src"))

from collector.config import load_config
from collector.handoff import build_handoff_with_size_guard
from collector.store import SQLiteStore
from collector.utils.time import utc_now


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Build handoff package and enqueue")
    parser.add_argument(
        "--config", default="configs/config.yaml", help="path to config file"
    )
    parser.add_argument(
        "--max-size-kb", type=int, default=50, help="max payload size in KB"
    )
    parser.add_argument(
        "--sessions", type=int, default=3, help="recent sessions to include"
    )
    parser.add_argument(
        "--routines", type=int, default=10, help="routine candidates to include"
    )
    parser.add_argument(
        "--resources", type=int, default=10, help="max resources per session"
    )
    parser.add_argument(
        "--evidence", type=int, default=5, help="evidence session ids per candidate"
    )
    parser.add_argument(
        "--redaction-scan",
        type=int,
        default=200,
        help="recent events to scan for redaction summary",
    )
    parser.add_argument("--dry-run", action="store_true", help="do not enqueue")
    parser.add_argument(
        "--skip-unchanged",
        action="store_true",
        help="skip if last_event_ts matches pending payload",
    )
    parser.add_argument(
        "--keep-latest-pending",
        action="store_true",
        help="delete existing pending payload before insert",
    )
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

    payload = build_handoff_with_size_guard(
        store,
        str(config.privacy_rules_path),
        max_size_bytes=args.max_size_kb * 1024,
        recent_sessions=args.sessions,
        recent_routines=args.routines,
        max_resources=args.resources,
        max_evidence=args.evidence,
        redaction_scan_limit=args.redaction_scan,
    )

    payload_json = json.dumps(payload.payload, separators=(",", ":"))
    last_event_ts = payload.payload.get("device_context", {}).get("last_event_ts")

    if args.skip_unchanged:
        latest = store.fetch_latest_handoff(status="pending")
        if latest:
            _, previous_json = latest
            previous = _safe_json(previous_json)
            prev_ts = previous.get("device_context", {}).get("last_event_ts")
            if prev_ts and prev_ts == last_event_ts:
                print("handoff_skipped=unchanged")
                store.close()
                return

    if args.dry_run:
        print(f"handoff_ready size_bytes={payload.size_bytes}")
        store.close()
        return

    if args.keep_latest_pending:
        store.clear_pending_handoff()

    store.enqueue_handoff(
        package_id=payload.payload["package_id"],
        created_at=payload.payload["created_at"],
        status="pending",
        payload_json=payload_json,
        payload_size=payload.size_bytes,
        expires_at=None,
        error=None,
    )

    print(f"handoff_enqueued size_bytes={payload.size_bytes}")
    store.close()


def _safe_json(value: str) -> dict:
    try:
        return json.loads(value)
    except (TypeError, ValueError, json.JSONDecodeError):
        return {}


if __name__ == "__main__":
    main()
