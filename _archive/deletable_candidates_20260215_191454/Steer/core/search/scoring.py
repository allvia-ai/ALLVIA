# scoring.py - Extracted from retriever.py (GOD CLASS refactoring)
"""Scoring and formatting helpers for hybrid retrieval and reranking."""

from __future__ import annotations

import math
from datetime import datetime
from pathlib import Path
from typing import Any, Dict, List, Optional, Sequence, Set, Tuple

from .query_parser import _normalize_owner, _to_float, _to_int
from .session import SessionState

_EXTENSION_MATCH_BONUS = 0.05
_SESSION_EXT_PREF_SCALE = 0.05
_SESSION_OWNER_PREF_SCALE = 0.04


def _normalize_ext(ext: Any) -> str:
    if not ext:
        return ""
    ext_str = str(ext).strip().lower()
    if not ext_str:
        return ""
    if not ext_str.startswith("."):
        ext_str = f".{ext_str}"
    return ext_str


def _compute_extension_bonus(
    ext: Optional[str],
    desired_exts: Set[str],
    session: Optional[SessionState],
) -> Tuple[float, float, float]:
    normalized = _normalize_ext(ext)
    desired_bonus = _EXTENSION_MATCH_BONUS if normalized and normalized in desired_exts else 0.0
    session_bonus = 0.0
    if session is not None and normalized:
        preference = session.preferred_exts.get(normalized, 0.0)
        if preference:
            session_bonus = preference * _SESSION_EXT_PREF_SCALE
    return desired_bonus + session_bonus, desired_bonus, session_bonus


def _compute_owner_bonus(owner: Optional[str], session: Optional[SessionState]) -> float:
    if session is None or not owner:
        return 0.0
    normalized = _normalize_owner(str(owner))
    if not normalized:
        return 0.0
    preference = session.owner_prior.get(normalized, 0.0)
    if not preference:
        return 0.0
    return preference * _SESSION_OWNER_PREF_SCALE


def _prioritize_ext_hits(hits: List[Dict[str, Any]], *, desired_exts: Set[str], top_k: int) -> List[Dict[str, Any]]:
    if not hits:
        return []
    if not desired_exts:
        return hits[:top_k]

    desired_hits: List[Dict[str, Any]] = []
    other_hits: List[Dict[str, Any]] = []

    for hit in hits:
        ext = _normalize_ext(hit.get("ext", ""))
        if ext in desired_exts:
            desired_hits.append(hit)
        else:
            other_hits.append(hit)

    if not desired_hits:
        return hits[:top_k]

    required_matches = max(1, min(top_k, int(math.ceil(top_k * 0.95))))
    take_from_desired = min(len(desired_hits), required_matches)

    ordered: List[Dict[str, Any]] = desired_hits[:take_from_desired]

    remaining_slots = top_k - len(ordered)
    if remaining_slots > 0 and take_from_desired < len(desired_hits):
        additional = desired_hits[take_from_desired : take_from_desired + remaining_slots]
        ordered.extend(additional)
        remaining_slots = top_k - len(ordered)

    if remaining_slots > 0:
        ordered.extend(other_hits[:remaining_slots])

    return ordered[:top_k]


def _minmax_scale(values: Sequence[float]) -> List[float]:
    data = [float(v) for v in values if v is not None]
    if not data:
        return []
    vmin = min(data)
    vmax = max(data)
    if math.isclose(vmax, vmin, abs_tol=1e-12):
        return [0.5] * len(values)
    span = vmax - vmin
    return [((float(v) - vmin) / span) if v is not None else 0.0 for v in values]


def _mask_path(path: str) -> str:
    if not path:
        return ""
    try:
        return Path(path).name
    except Exception:
        return "<invalid-path>"


def _format_human_time(epoch: Any) -> str:
    value = _to_float(epoch)
    if value is None or value <= 0:
        return ""
    try:
        return datetime.fromtimestamp(value).strftime("%Y-%m-%d")
    except Exception:
        return ""


def _format_size(size: Any) -> str:
    num = _to_int(size)
    if num is None or num <= 0:
        return ""
    units = ["B", "KB", "MB", "GB", "TB"]
    value = float(num)
    for unit in units:
        if value < 1024 or unit == units[-1]:
            if unit == "B":
                return f"{int(value)}{unit}"
            return f"{value:.1f}{unit}"
        value /= 1024
    return f"{num}B"


def _compose_rerank_document(hit: Dict[str, Any]) -> str:
    sections: List[str] = []
    path = str(hit.get("path") or "").strip()
    if path:
        sections.append(f"파일 경로: {_mask_path(path)}")
    ext = str(hit.get("ext") or "").strip()
    if ext:
        sections.append(f"확장자: {ext}")
    drive = str(hit.get("drive") or "").strip()
    if drive:
        sections.append(f"드라이브: {drive}")
    owner = str(hit.get("owner") or "").strip()
    if owner:
        sections.append(f"작성자: {owner}")
    mtime_label = _format_human_time(hit.get("mtime"))
    if mtime_label:
        sections.append(f"수정일: {mtime_label}")
    size_label = _format_size(hit.get("size"))
    if size_label:
        sections.append(f"파일 크기: {size_label}")
    preview = str(hit.get("preview") or "").strip()
    if preview:
        sections.append(preview)
    return "\n".join(section for section in sections if section)


def _similarity_to_percent(value: Any, *, decimals: int = 1) -> str:
    try:
        score = float(value)
    except (TypeError, ValueError):
        return "-"
    score = max(0.0, min(score, 1.0))
    pct = score * 100.0
    return f"{pct:.{decimals}f}%"

