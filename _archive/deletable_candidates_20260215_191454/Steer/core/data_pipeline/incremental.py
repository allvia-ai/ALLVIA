"""Incremental scan state helpers."""

from __future__ import annotations

import json
from pathlib import Path
from typing import Dict, List, Tuple, Any


DEFAULT_STATE: Dict[str, Dict[str, float]] = {
    "paths": {},
    "last_scan_timestamp": 0.0,
}


def load_scan_state(path: Path) -> Dict[str, Dict[str, float]]:
    if not path or not path.exists():
        return {"paths": {}, "last_scan_timestamp": 0.0}
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
        paths = payload.get("paths") or {}
        if not isinstance(paths, dict):
            paths = {}
        return {
            "paths": {str(k): _coerce_meta(v) for k, v in paths.items()},
            "last_scan_timestamp": float(payload.get("last_scan_timestamp") or 0.0),
        }
    except Exception:
        return {"paths": {}, "last_scan_timestamp": 0.0}


def _coerce_meta(meta) -> Dict[str, float]:
    try:
        size = int(meta.get("size", 0))
    except Exception:
        size = 0
    try:
        mtime = float(meta.get("mtime", 0.0))
    except Exception:
        mtime = 0.0
    return {"size": size, "mtime": mtime}


def filter_incremental_rows(
    rows: List[Dict[str, float]],
    state: Dict[str, Dict[str, float]],
) -> Tuple[List[Dict[str, float]], List[Dict[str, float]]]:
    """Split rows into (needs_processing, cached) using stored metadata."""

    if not rows or not state:
        return list(rows or []), []

    path_state = state.get("paths") or {}
    to_process: List[Dict[str, float]] = []
    cached: List[Dict[str, float]] = []

    for row in rows:
        path = str(row.get("path") or "")
        if not path:
            to_process.append(row)
            continue
        entry = path_state.get(path)
        if entry is None:
            to_process.append(row)
            continue
        row_size = int(row.get("size") or 0)
        row_mtime = float(row.get("mtime") or 0.0)
        if row_size != int(entry.get("size", -1)) or abs(row_mtime - float(entry.get("mtime", 0.0))) > 1.0:
            to_process.append(row)
        else:
            cached.append(row)
    return to_process, cached


def update_scan_state(state: Dict[str, Dict[str, float]], rows: List[Dict[str, float]]) -> Dict[str, Dict[str, float]]:
    state = state or {"paths": {}, "last_scan_timestamp": 0.0}
    paths = state.setdefault("paths", {})
    seen = set()
    max_ts = state.get("last_scan_timestamp", 0.0)

    for row in rows:
        path = str(row.get("path") or "")
        if not path:
            continue
        meta = _coerce_meta(row)
        paths[path] = meta
        seen.add(path)
        if meta["mtime"] > max_ts:
            max_ts = meta["mtime"]

    for path in list(paths.keys()):
        if path not in seen:
            paths.pop(path, None)

    state["last_scan_timestamp"] = max_ts
    return state


def save_scan_state(path: Path, state: Dict[str, Dict[str, float]]) -> None:
    if not path:
        return
    path.parent.mkdir(parents=True, exist_ok=True)
    payload = {
        "last_scan_timestamp": float(state.get("last_scan_timestamp") or 0.0),
        "paths": state.get("paths") or {},
    }
    path.write_text(json.dumps(payload, ensure_ascii=False, indent=2), encoding="utf-8")
