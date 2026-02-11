from __future__ import annotations

import argparse
import json
import sqlite3
import sys
from dataclasses import dataclass
from datetime import timedelta
from pathlib import Path
from typing import Any

import yaml

# Ensure local src is importable when running as a script.
PROJECT_ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(PROJECT_ROOT / "src"))

from collector.config import load_config
from collector.utils.time import utc_now


@dataclass
class AppStats:
    app: str
    seconds: float = 0.0
    blocks: int = 0
    last_seen_ts: str = ""


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Recommend allowlist apps based on focus block usage"
    )
    parser.add_argument(
        "--config", default="configs/config.yaml", help="path to config file"
    )
    parser.add_argument("--start", default="", help="start ts (ISO, optional)")
    parser.add_argument("--end", default="", help="end ts (ISO, optional)")
    parser.add_argument(
        "--days", type=float, default=3.0, help="look back N days for usage"
    )
    parser.add_argument(
        "--min-minutes",
        type=float,
        default=10.0,
        help="min focus minutes to recommend",
    )
    parser.add_argument(
        "--min-blocks", type=int, default=3, help="min focus blocks to recommend"
    )
    parser.add_argument(
        "--top-n", type=int, default=30, help="limit recommendations"
    )
    parser.add_argument(
        "--include-existing",
        action="store_true",
        help="include apps already in allowlist/denylist",
    )
    parser.add_argument(
        "--output",
        default="",
        help="optional path to write YAML report",
    )
    parser.add_argument(
        "--apply",
        action="store_true",
        help="append recommendations into privacy_rules.yaml",
    )
    parser.add_argument(
        "--sort",
        choices=["duration", "blocks", "last_seen"],
        default="duration",
        help="sort recommendations by duration, blocks, or last_seen",
    )
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    config = load_config(args.config)
    privacy_rules_path = Path(config.privacy_rules_path)

    start_ts = args.start.strip() or None
    end_ts = args.end.strip() or None
    if args.days and not start_ts:
        start_ts = (utc_now() - timedelta(days=args.days)).isoformat().replace(
            "+00:00", "Z"
        )

    allowlist, denylist = _load_privacy_lists(privacy_rules_path)
    stats = _collect_focus_stats(config.db_path, start_ts, end_ts)
    candidates = _build_candidates(stats, args.min_minutes, args.min_blocks)
    candidates = _filter_candidates(
        candidates, allowlist, denylist, include_existing=args.include_existing
    )
    candidates = _sort_candidates(candidates, args.sort)
    if args.top_n and args.top_n > 0:
        candidates = candidates[: args.top_n]

    report = _build_report(
        candidates,
        allowlist,
        denylist,
        start_ts=start_ts,
        end_ts=end_ts,
        min_minutes=args.min_minutes,
        min_blocks=args.min_blocks,
        top_n=args.top_n,
        sort_key=args.sort,
    )

    if args.output:
        _write_yaml(Path(args.output), report)

    _print_summary(candidates)

    if args.apply:
        added = _apply_allowlist_update(
            privacy_rules_path, allowlist, denylist, candidates
        )
        if added:
            print(f"allowlist_applied={len(added)} backup={privacy_rules_path}.bak")
        else:
            print("allowlist_apply_skipped=none")


def _collect_focus_stats(
    db_path: Path, start_ts: str | None, end_ts: str | None
) -> dict[str, AppStats]:
    if not Path(db_path).exists():
        raise FileNotFoundError(f"db not found: {db_path}")

    query = "SELECT ts, app, payload_json FROM events WHERE event_type = ?"
    params: list[Any] = ["os.app_focus_block"]
    if start_ts:
        query += " AND ts >= ?"
        params.append(start_ts)
    if end_ts:
        query += " AND ts <= ?"
        params.append(end_ts)
    query += " ORDER BY ts ASC"

    stats: dict[str, AppStats] = {}
    conn = sqlite3.connect(str(db_path))
    try:
        for ts, app, payload_json in conn.execute(query, params):
            app_key = _normalize_app(app)
            if not app_key:
                continue
            duration = _extract_duration(payload_json)
            entry = stats.get(app_key)
            if entry is None:
                entry = AppStats(app=app_key)
                stats[app_key] = entry
            entry.seconds += duration
            entry.blocks += 1
            if ts and ts > entry.last_seen_ts:
                entry.last_seen_ts = ts
    finally:
        conn.close()

    return stats


def _build_candidates(
    stats: dict[str, AppStats], min_minutes: float, min_blocks: int
) -> list[AppStats]:
    candidates: list[AppStats] = []
    min_minutes = max(0.0, float(min_minutes))
    min_blocks = max(0, int(min_blocks))
    for entry in stats.values():
        minutes = entry.seconds / 60.0
        if minutes >= min_minutes or entry.blocks >= min_blocks:
            candidates.append(entry)
    return candidates


def _filter_candidates(
    candidates: list[AppStats],
    allowlist: set[str],
    denylist: set[str],
    *,
    include_existing: bool,
) -> list[AppStats]:
    if include_existing:
        return candidates
    filtered = []
    for entry in candidates:
        if entry.app in allowlist or entry.app in denylist:
            continue
        filtered.append(entry)
    return filtered


def _sort_candidates(candidates: list[AppStats], sort_key: str) -> list[AppStats]:
    if sort_key == "blocks":
        return sorted(candidates, key=lambda item: item.blocks, reverse=True)
    if sort_key == "last_seen":
        return sorted(candidates, key=lambda item: item.last_seen_ts, reverse=True)
    return sorted(candidates, key=lambda item: item.seconds, reverse=True)


def _build_report(
    candidates: list[AppStats],
    allowlist: set[str],
    denylist: set[str],
    *,
    start_ts: str | None,
    end_ts: str | None,
    min_minutes: float,
    min_blocks: int,
    top_n: int,
    sort_key: str,
) -> dict[str, Any]:
    output: dict[str, Any] = {
        "generated_at": utc_now().isoformat().replace("+00:00", "Z"),
        "window": {"start_ts": start_ts, "end_ts": end_ts},
        "criteria": {
            "min_minutes": float(min_minutes),
            "min_blocks": int(min_blocks),
            "top_n": int(top_n),
            "sort": sort_key,
        },
        "candidates": [],
        "existing": {
            "allowlist_count": len(allowlist),
            "denylist_count": len(denylist),
        },
    }

    for entry in candidates:
        output["candidates"].append(
            {
                "app": entry.app,
                "focus_minutes": round(entry.seconds / 60.0, 2),
                "focus_seconds": int(entry.seconds),
                "blocks": entry.blocks,
                "last_seen_ts": entry.last_seen_ts or None,
            }
        )
    return output


def _apply_allowlist_update(
    rules_path: Path,
    allowlist: set[str],
    denylist: set[str],
    candidates: list[AppStats],
) -> list[str]:
    additions = [
        entry.app
        for entry in candidates
        if entry.app not in allowlist and entry.app not in denylist
    ]
    if not additions:
        return []

    raw = yaml.safe_load(rules_path.read_text()) or {}
    existing = raw.get("allowlist_apps") or []
    updated = list(existing)
    updated.extend(additions)
    raw["allowlist_apps"] = updated

    backup_path = rules_path.with_suffix(rules_path.suffix + ".bak")
    backup_path.write_text(rules_path.read_text())
    rules_path.write_text(
        yaml.safe_dump(raw, sort_keys=False, allow_unicode=False)
    )
    return additions


def _load_privacy_lists(path: Path) -> tuple[set[str], set[str]]:
    raw = yaml.safe_load(path.read_text()) or {}
    allowlist = _lower_set(raw.get("allowlist_apps"))
    denylist = _lower_set(raw.get("denylist_apps"))
    return allowlist, denylist


def _lower_set(value: Any) -> set[str]:
    if not value:
        return set()
    if isinstance(value, (list, set, tuple)):
        return {str(item).lower() for item in value}
    return {str(value).lower()}


def _normalize_app(app: Any) -> str:
    if app is None:
        return ""
    app_text = str(app).strip().lower()
    if app_text in {"", "unknown", "none"}:
        return ""
    return app_text


def _extract_duration(payload_json: Any) -> float:
    if not payload_json:
        return 0.0
    try:
        payload = json.loads(payload_json)
    except (TypeError, json.JSONDecodeError):
        return 0.0
    duration = payload.get("duration_sec", 0)
    if isinstance(duration, (int, float)):
        return float(duration)
    try:
        return float(duration)
    except (TypeError, ValueError):
        return 0.0


def _write_yaml(path: Path, payload: dict[str, Any]) -> None:
    path.write_text(yaml.safe_dump(payload, sort_keys=False, allow_unicode=False))


def _print_summary(candidates: list[AppStats]) -> None:
    if not candidates:
        print("recommendations=0")
        return
    print(f"recommendations={len(candidates)}")
    for entry in candidates[:10]:
        minutes = entry.seconds / 60.0
        print(
            f"- {entry.app} minutes={minutes:.1f} blocks={entry.blocks} last={entry.last_seen_ts}"
        )


if __name__ == "__main__":
    main()
