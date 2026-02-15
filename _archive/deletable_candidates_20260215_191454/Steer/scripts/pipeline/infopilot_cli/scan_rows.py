from __future__ import annotations

import csv
import math
from pathlib import Path
from typing import Any, Dict, Iterator, Optional

from core.policy.engine import PolicyEngine


NORMALIZED_ALIASES = {
    "path": ("path", "filepath", "file_path", "fullpath", "full_path", "absolute_path"),
    "size": ("size", "filesize", "file_size", "bytes"),
    "mtime": ("mtime", "modified", "modified_time", "lastmodified", "timestamp"),
    "ctime": ("ctime", "created", "created_time", "creation", "creation_time"),
    "ext": ("ext", "extension", "suffix"),
    "drive": ("drive", "volume", "root"),
    "owner": ("owner", "user", "username", "author", "created_by"),
}


def _normalize_key(name: str) -> str:
    """Normalize header names by stripping non-alphanumerics and lowering case."""
    return "".join(ch for ch in (name or "").lower() if ch.isalnum())


def _pick_value(row: Dict[str, str], aliases) -> str:
    normalized = {_normalize_key(k): (k, v) for k, v in row.items() if k}
    for alias in aliases:
        alias_norm = _normalize_key(alias)
        data = normalized.get(alias_norm)
        if data:
            value = (data[1] or "").strip()
            if value:
                return value
    return ""


def normalize_scan_row(raw: Dict[str, str], *, context: str = "") -> Dict[str, Any] | None:
    path = _pick_value(raw, NORMALIZED_ALIASES["path"])
    if not path:
        columns = ", ".join(k for k in raw.keys() if k)
        location = f" ({context})" if context else ""
        print(f"⚠️ 경고: 'path' 값을 찾지 못해 행을 건너뜁니다{location}. (감지한 열: {columns or '없음'})")
        return None

    size_raw = _pick_value(raw, NORMALIZED_ALIASES["size"])
    mtime_raw = _pick_value(raw, NORMALIZED_ALIASES["mtime"])
    ext = _pick_value(raw, NORMALIZED_ALIASES["ext"])
    drive = _pick_value(raw, NORMALIZED_ALIASES["drive"])
    ctime_raw = _pick_value(raw, NORMALIZED_ALIASES["ctime"])
    owner = _pick_value(raw, NORMALIZED_ALIASES["owner"])

    def to_int(value: str) -> int:
        try:
            return int(float(value))
        except (TypeError, ValueError):
            return 0

    def to_float(value: str) -> float:
        try:
            out = float(value)
            if math.isnan(out) or math.isinf(out):
                return 0.0
            return out
        except (TypeError, ValueError):
            return 0.0

    normalized = dict(raw)
    normalized["path"] = path
    normalized["size"] = to_int(size_raw)
    normalized["mtime"] = to_float(mtime_raw)
    normalized["ctime"] = to_float(ctime_raw)
    if ext:
        normalized["ext"] = ext
    if drive:
        normalized["drive"] = drive
    if owner:
        normalized["owner"] = owner
    return normalized


def resolve_scan_csv(path: Path) -> Path:
    if path.exists():
        return path

    search_root = path.parent if path.parent else Path(".")
    candidates = []
    for candidate in sorted(search_root.glob("*.csv"), key=lambda p: p.stat().st_mtime, reverse=True):
        try:
            with candidate.open("r", encoding="utf-8", newline="") as handle:
                reader = csv.DictReader(handle)
                headers = reader.fieldnames or []
        except OSError:
            continue
        header_norm = {_normalize_key(h) for h in headers}
        if any(_normalize_key(alias) in header_norm for alias in NORMALIZED_ALIASES["path"]):
            candidates.append(candidate)

    if candidates:
        picked = candidates[0]
        print(f"⚠️ '{path}' 파일이 없어 '{picked}'을(를) 사용합니다.")
        return picked

    raise FileNotFoundError(f"스캔 CSV를 찾을 수 없습니다: {path}")


def iter_scan_rows(scan_csv: Path) -> Iterator[Dict[str, Any]]:
    with scan_csv.open("r", encoding="utf-8", newline="") as handle:
        reader = csv.DictReader(handle)
        for idx, raw in enumerate(reader, start=2):
            normalized = normalize_scan_row(raw, context=f"{scan_csv}:{idx}")
            if normalized:
                yield normalized


def load_scan_rows(
    scan_csv: Path,
    *,
    policy_engine: Optional[PolicyEngine] = None,
    include_manual: bool = True,
    agent: str,
) -> Iterator[Dict[str, Any]]:
    for row in iter_scan_rows(scan_csv):
        allowed_raw = row.get("allowed")
        if allowed_raw is not None and str(allowed_raw).strip().lower() in {"0", "false", "no"}:
            continue
        if policy_engine and policy_engine.has_policies:
            raw_path = row.get("path")
            if not raw_path:
                continue
            path = Path(str(raw_path))
            if not policy_engine.allows(path, agent=agent, include_manual=include_manual):
                continue
            enriched = dict(row)
            enriched["policy_mask_pii"] = policy_engine.pii_mask_enabled_for_path(path, agent=agent)
            yield enriched
            continue
        yield row


__all__ = [
    "NORMALIZED_ALIASES",
    "load_scan_rows",
    "resolve_scan_csv",
]
