from __future__ import annotations

import argparse
import json
import sqlite3
import sys
from datetime import timedelta
from pathlib import Path
from typing import Any, Iterable

# Ensure local src is importable when running as a script.
PROJECT_ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(PROJECT_ROOT / "src"))

from collector.config import load_config
from collector.utils.time import parse_ts, utc_now


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Show focus block titles from stored events (on-demand)"
    )
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
        "--limit", type=int, default=50, help="max rows to print"
    )
    parser.add_argument(
        "--order",
        choices=["asc", "desc"],
        default="desc",
        help="sort by ts",
    )
    parser.add_argument(
        "--app",
        default="",
        help="comma-separated app names to include (case-insensitive)",
    )
    parser.add_argument(
        "--contains",
        default="",
        help="filter titles containing substring (case-insensitive)",
    )
    parser.add_argument(
        "--local-time",
        action="store_true",
        help="format timestamps in local time",
    )
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    config = load_config(args.config)

    start_ts = args.start.strip() or None
    end_ts = args.end.strip() or None
    if args.since_hours and not start_ts:
        start_ts = (utc_now() - timedelta(hours=args.since_hours)).isoformat().replace(
            "+00:00", "Z"
        )

    app_filter = _parse_apps(args.app)
    title_filter = args.contains.strip().lower()

    query = "SELECT ts, app, payload_json FROM events WHERE event_type = ?"
    params: list[Any] = ["os.app_focus_block"]
    if start_ts:
        query += " AND ts >= ?"
        params.append(start_ts)
    if end_ts:
        query += " AND ts <= ?"
        params.append(end_ts)
    query += " ORDER BY ts " + ("ASC" if args.order == "asc" else "DESC")

    printed = 0
    conn = sqlite3.connect(str(config.db_path))
    try:
        for ts, app, payload_json in conn.execute(query, params):
            app_name = str(app or "").strip()
            if app_filter and app_name.lower() not in app_filter:
                continue
            title = _extract_title(payload_json)
            if not title:
                continue
            if title_filter and title_filter not in title.lower():
                continue
            ts_text = _format_ts(ts, local=args.local_time)
            print(f"{ts_text} {app_name} {title}")
            printed += 1
            if printed >= max(1, int(args.limit)):
                break
    finally:
        conn.close()

    if printed == 0:
        print("no matching titles found")


def _extract_title(payload_json: Any) -> str:
    if not payload_json:
        return ""
    try:
        payload = json.loads(payload_json)
    except (TypeError, json.JSONDecodeError):
        return ""
    title = payload.get("window_title")
    if isinstance(title, str):
        return title.strip()
    return ""


def _parse_apps(value: str) -> set[str]:
    if not value:
        return set()
    parts = [item.strip().lower() for item in value.split(",")]
    return {item for item in parts if item}


def _format_ts(value: Any, *, local: bool) -> str:
    if not local:
        return str(value or "")
    parsed = parse_ts(value)
    if not parsed:
        return str(value or "")
    return parsed.astimezone().strftime("%Y-%m-%d %H:%M:%S")


if __name__ == "__main__":
    main()
