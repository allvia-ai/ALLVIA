# query_parser.py - Extracted from retriever.py (GOD CLASS refactoring)
"""Metadata filter parsing helpers for hybrid retrieval."""

from __future__ import annotations

import calendar
import re
from dataclasses import dataclass, field
from datetime import datetime, timedelta
from typing import Any, Dict, List, Optional, Set, Tuple


def _clean_token(token: str) -> str:
    if not token:
        return ""
    cleaned = re.sub(r"\s+", " ", str(token)).strip().lower()
    return cleaned


@dataclass
class MetadataFilters:
    mtime_from: Optional[float] = None
    mtime_to: Optional[float] = None
    ctime_from: Optional[float] = None
    ctime_to: Optional[float] = None
    size_min: Optional[int] = None
    size_max: Optional[int] = None
    owners: Set[str] = field(default_factory=set)

    def is_active(self) -> bool:
        return any(
            value is not None
            for value in (
                self.mtime_from,
                self.mtime_to,
                self.ctime_from,
                self.ctime_to,
                self.size_min,
                self.size_max,
            )
        ) or bool(self.owners)

    def matches(self, hit: Dict[str, Any]) -> bool:
        if not self.is_active():
            return True
        mtime = _to_float(hit.get("mtime"))
        ctime = _to_float(hit.get("ctime"))
        size = _to_int(hit.get("size"))
        owner = _clean_token(hit.get("owner", ""))

        if self.mtime_from is not None and (mtime is None or mtime < self.mtime_from):
            return False
        if self.mtime_to is not None and (mtime is None or mtime > self.mtime_to):
            return False
        if self.ctime_from is not None and (ctime is None or ctime < self.ctime_from):
            return False
        if self.ctime_to is not None and (ctime is None or ctime > self.ctime_to):
            return False
        if self.size_min is not None and (size is None or size < self.size_min):
            return False
        if self.size_max is not None and (size is None or size > self.size_max):
            return False
        if self.owners and owner and owner not in self.owners:
            return False
        if self.owners and not owner:
            return False
        return True


def _to_float(value: Any) -> Optional[float]:
    if value is None:
        return None
    try:
        return float(value)
    except (TypeError, ValueError):
        return None


def _to_int(value: Any) -> Optional[int]:
    if value is None:
        return None
    try:
        return int(value)
    except (TypeError, ValueError):
        return None


def _year_bounds(year: int) -> Tuple[float, float]:
    start = datetime(year, 1, 1)
    end = datetime(year, 12, 31, 23, 59, 59)
    return start.timestamp(), end.timestamp()


def _month_bounds(now: datetime, months_ago: int) -> Tuple[float, float]:
    year = now.year
    month = now.month - months_ago
    while month <= 0:
        month += 12
        year -= 1
    start = datetime(year, month, 1)
    last_day = calendar.monthrange(year, month)[1]
    end = datetime(year, month, last_day, 23, 59, 59)
    return start.timestamp(), end.timestamp()


def _approx_range(value: int, *, tolerance: float = 0.15) -> Tuple[float, float]:
    delta = max(1.0, value * tolerance)
    return value - delta, value + delta


def _normalize_owner(owner: str) -> str:
    return _clean_token(owner)


def _parse_size_expression(value: str, unit: str) -> int:
    base = float(value)
    unit = unit.lower()
    multiplier = {
        "kb": 1024,
        "mb": 1024**2,
        "gb": 1024**3,
        "tb": 1024**4,
    }.get(unit, 1)
    return int(base * multiplier)


def _extract_metadata_filters(query: str) -> MetadataFilters:
    filters = MetadataFilters()
    lowered = query.lower()
    now = datetime.now()

    year_match = re.search(r"(20\d{2}|19\d{2})\s*년", lowered)
    if year_match:
        year = int(year_match.group(1))
        filters.mtime_from, filters.mtime_to = _year_bounds(year)

    rel_year = re.search(r"(\d+)\s*년\s*전", lowered)
    if rel_year:
        years = int(rel_year.group(1))
        target_year = now.year - years
        filters.mtime_from, filters.mtime_to = _year_bounds(target_year)

    if "작년" in lowered:
        filters.mtime_from, filters.mtime_to = _year_bounds(now.year - 1)
    if "재작년" in lowered:
        filters.mtime_from, filters.mtime_to = _year_bounds(now.year - 2)
    if "올해" in lowered or "올 해" in lowered or "금년" in lowered:
        filters.mtime_from, filters.mtime_to = _year_bounds(now.year)

    rel_month = re.search(r"(\d+)\s*개월\s*전", lowered)
    if rel_month:
        months = int(rel_month.group(1))
        filters.mtime_from, filters.mtime_to = _month_bounds(now, months)
    if "지난달" in lowered:
        filters.mtime_from, filters.mtime_to = _month_bounds(now, 1)
    if "이번달" in lowered or "이 달" in lowered:
        filters.mtime_from, filters.mtime_to = _month_bounds(now, 0)

    if any(keyword in lowered for keyword in ["최근", "요즘", "요근래", "최근에"]):
        horizon = now - timedelta(days=180)
        filters.mtime_from = horizon.timestamp()

    for match in re.finditer(
        r"(\d+(?:\.\d+)?)\s*(kb|mb|gb|tb)\s*(이상|이하|초과|미만|보다 큰|보다 작은|at least|over|under|at most)?",
        lowered,
    ):
        value = match.group(1)
        unit = match.group(2)
        qualifier = match.group(3) or ""
        size_bytes = _parse_size_expression(value, unit)
        if any(token in qualifier for token in ["이상", "초과", "보다 큰", "at least", "over"]):
            filters.size_min = max(filters.size_min or 0, size_bytes)
        elif any(token in qualifier for token in ["이하", "미만", "보다 작은", "at most", "under"]):
            filters.size_max = min(filters.size_max or size_bytes, size_bytes)
        else:
            approx_min, approx_max = _approx_range(size_bytes)
            filters.size_min = max(filters.size_min or 0, int(approx_min))
            filters.size_max = min(filters.size_max or int(approx_max), int(approx_max))

    for match in re.finditer(r"(?:작성자|author|owner)[:\s]+([\w가-힣@.]+)", query, re.IGNORECASE):
        filters.owners.add(_normalize_owner(match.group(1)))
    for mention in re.findall(r"@([\w가-힣._-]+)", query):
        filters.owners.add(_normalize_owner(mention))

    return filters


def _apply_metadata_filters(hits: List[Dict[str, Any]], filters: MetadataFilters) -> List[Dict[str, Any]]:
    if not filters.is_active():
        return hits
    return [hit for hit in hits if filters.matches(hit)]
