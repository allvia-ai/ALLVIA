from __future__ import annotations

import argparse
import json
import time
import urllib.error
import urllib.request
from collections import Counter
from datetime import datetime
from pathlib import Path
from typing import Any, Dict, Iterable, List, Optional, Tuple


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Replay JSONL events into collector")
    parser.add_argument("--file", required=True, help="path to jsonl file")
    parser.add_argument(
        "--endpoint",
        default="http://127.0.0.1:8080/events",
        help="collector /events endpoint",
    )
    parser.add_argument(
        "--speed",
        default="fast",
        help="replay speed: fast, realtime, x10",
    )
    parser.add_argument("--batch", type=int, default=1, help="events per request")
    parser.add_argument("--token", default="", help="shared token (optional)")
    return parser.parse_args()


def _parse_ts(value: Any) -> Optional[datetime]:
    if not value:
        return None
    if isinstance(value, (int, float)):
        return datetime.utcfromtimestamp(value)
    if isinstance(value, str):
        try:
            return datetime.fromisoformat(value.replace("Z", "+00:00"))
        except ValueError:
            return None
    return None


def _speed_factor(speed: str) -> Optional[float]:
    speed = speed.strip().lower()
    if speed == "fast":
        return None
    if speed == "realtime":
        return 1.0
    if speed.startswith("x"):
        try:
            factor = float(speed[1:])
        except ValueError:
            return None
        if factor <= 0:
            return None
        return 1.0 / factor
    return None


def _chunks(events: List[Dict[str, Any]], batch: int) -> Iterable[List[Dict[str, Any]]]:
    if batch <= 1:
        for event in events:
            yield [event]
        return
    for idx in range(0, len(events), batch):
        yield events[idx : idx + batch]


def _send_payload(
    endpoint: str, payload: Any, token: str
) -> Tuple[bool, int, float, str]:
    data = json.dumps(payload, separators=(",", ":")).encode("utf-8")
    headers = {"Content-Type": "application/json"}
    if token:
        headers["X-Collector-Token"] = token
    request = urllib.request.Request(endpoint, data=data, headers=headers, method="POST")
    start = time.perf_counter()
    try:
        with urllib.request.urlopen(request, timeout=5) as response:
            elapsed = time.perf_counter() - start
            ok = 200 <= response.status < 300
            return ok, response.status, elapsed, ""
    except urllib.error.HTTPError as exc:
        elapsed = time.perf_counter() - start
        return False, exc.code, elapsed, str(exc)
    except urllib.error.URLError as exc:
        elapsed = time.perf_counter() - start
        return False, 0, elapsed, str(exc)


def _load_events(path: Path) -> List[Dict[str, Any]]:
    events = []
    for line in path.read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if not line:
            continue
        events.append(json.loads(line))
    return events


def main() -> None:
    args = parse_args()
    path = Path(args.file)
    events = _load_events(path)
    if not events:
        print("no events loaded")
        return

    speed_factor = _speed_factor(args.speed)
    type_counts = Counter(event.get("event_type", "unknown") for event in events)

    sent = ok = failed = 0
    elapsed_times: List[float] = []
    last_ts: Optional[datetime] = None

    for batch in _chunks(events, args.batch):
        if speed_factor is not None:
            current_ts = _parse_ts(batch[0].get("ts"))
            if current_ts and last_ts:
                delay = max(0.0, (current_ts - last_ts).total_seconds() * speed_factor)
                if delay:
                    time.sleep(delay)
            if current_ts:
                last_ts = current_ts

        payload = batch[0] if len(batch) == 1 else batch
        sent += len(batch)
        ok_response, status, elapsed, error = _send_payload(
            args.endpoint, payload, args.token
        )
        elapsed_times.append(elapsed)
        if ok_response:
            ok += len(batch)
        else:
            failed += len(batch)
            print(f"request failed status={status} error={error}")

    avg_ms = (sum(elapsed_times) / len(elapsed_times)) * 1000.0
    print(f"sent={sent} ok={ok} failed={failed} avg_ms={avg_ms:.1f}")
    print("event_types:", dict(type_counts))


if __name__ == "__main__":
    main()
