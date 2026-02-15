"""Lightweight cooperative job scheduler used by InfoPilot core services."""
from __future__ import annotations

import threading
from dataclasses import dataclass, field
from datetime import datetime, timedelta
from typing import Any, Callable, Dict, Iterable, List, Optional, Sequence

from core.utils import get_logger

LOGGER = get_logger("infra.scheduler")

ScheduleCallback = Callable[[], Any]


@dataclass(frozen=True)
class ScheduleSpec:
    """Describe when a job should run."""

    mode: str = "realtime"
    interval: Optional[timedelta] = None
    cron: Optional[str] = None

    _cron_minutes: Optional[Sequence[int]] = field(init=False, default=None, repr=False)
    _cron_hours: Optional[Sequence[int]] = field(init=False, default=None, repr=False)
    _cron_days: Optional[Sequence[int]] = field(init=False, default=None, repr=False)
    _cron_months: Optional[Sequence[int]] = field(init=False, default=None, repr=False)
    _cron_weekdays: Optional[Sequence[int]] = field(init=False, default=None, repr=False)

    def __post_init__(self) -> None:
        object.__setattr__(self, "mode", (self.mode or "realtime").strip().lower())
        if self.mode not in {"realtime", "scheduled", "manual"}:
            object.__setattr__(self, "mode", "realtime")

        if self.mode == "scheduled" and not (self.interval or self.cron):
            # Default to a daily schedule when none is provided.
            object.__setattr__(self, "interval", timedelta(days=1))

        if self.interval is not None and self.interval.total_seconds() <= 0:
            object.__setattr__(self, "interval", timedelta(minutes=1))

        if self.cron:
            minutes, hours, dom, month, dow = _parse_cron(self.cron)
            object.__setattr__(self, "_cron_minutes", minutes)
            object.__setattr__(self, "_cron_hours", hours)
            object.__setattr__(self, "_cron_days", dom)
            object.__setattr__(self, "_cron_months", month)
            object.__setattr__(self, "_cron_weekdays", dow)

    @classmethod
    def from_policy(cls, policy: Any) -> "ScheduleSpec":
        indexing = getattr(policy, "indexing", {}) or {}
        mode = str(indexing.get("mode", "realtime") or "realtime").lower()
        cron = indexing.get("cron")
        interval_minutes = indexing.get("interval_minutes")
        interval = None
        if isinstance(interval_minutes, (int, float)) and interval_minutes > 0:
            interval = timedelta(minutes=float(interval_minutes))
        return cls(mode=mode, interval=interval, cron=cron)

    def next_run(self, *, last_run: Optional[datetime] = None, now: Optional[datetime] = None) -> Optional[datetime]:
        now = (now or datetime.utcnow()).replace(second=0, microsecond=0)
        if self.mode == "manual":
            return None
        if self.mode == "realtime":
            return now

        # Scheduled mode
        if self.interval is not None and not self.cron:
            if last_run is None:
                return now
            baseline = last_run
            candidate = (baseline + self.interval).replace(second=0, microsecond=0)
            if candidate <= now:
                elapsed = now - baseline
                interval_seconds = max(self.interval.total_seconds(), 1.0)
                steps = int(elapsed.total_seconds() // interval_seconds) + 1
                candidate = (baseline + (self.interval * steps)).replace(second=0, microsecond=0)
            return candidate

        # Cron-based schedule fallback
        start = now
        if last_run and last_run > start:
            start = last_run
        start = start.replace(second=0, microsecond=0)
        if start < now:
            start = now
        if start.second or start.microsecond:
            start = start.replace(second=0, microsecond=0) + timedelta(minutes=1)

        horizon = start + timedelta(days=366)
        current = start
        while current <= horizon:
            if self._matches_cron(current):
                return current
            current += timedelta(minutes=1)
        LOGGER.warning("Cron horizon exceeded without finding a match (cron=%s)", self.cron)
        return None

    def _matches_cron(self, candidate: datetime) -> bool:
        if self.cron is None:
            return True
        minute = candidate.minute
        hour = candidate.hour
        dom = candidate.day
        month = candidate.month
        weekday = (candidate.weekday() + 1) % 7  # convert Monday=0 to Sunday=0

        if self._cron_minutes is not None and minute not in self._cron_minutes:
            return False
        if self._cron_hours is not None and hour not in self._cron_hours:
            return False
        if self._cron_months is not None and month not in self._cron_months:
            return False
        dom_match = self._cron_days is None or dom in self._cron_days
        dow_match = self._cron_weekdays is None or weekday in self._cron_weekdays
        # Cron semantics: day-of-month and day-of-week are OR when both specified.
        if self._cron_days is not None and self._cron_weekdays is not None:
            return dom_match or dow_match
        return dom_match and dow_match


@dataclass
class ScheduledJob:
    name: str
    callback: ScheduleCallback
    schedule: ScheduleSpec
    metadata: Dict[str, Any] = field(default_factory=dict)
    last_run: Optional[datetime] = None
    next_run: Optional[datetime] = None

    def update_next_run(self, *, now: Optional[datetime] = None) -> None:
        self.next_run = self.schedule.next_run(last_run=self.last_run, now=now)


class JobScheduler:
    """Simple cooperative scheduler suitable for background maintenance jobs."""

    def __init__(self) -> None:
        self._jobs: Dict[str, ScheduledJob] = {}
        self._lock = threading.RLock()

    def register(self, job: ScheduledJob, *, overwrite: bool = False) -> ScheduledJob:
        with self._lock:
            if not overwrite and job.name in self._jobs:
                raise ValueError(f"Job '{job.name}' already registered")
            job.update_next_run()
            self._jobs[job.name] = job
        LOGGER.info("Registered job '%s' (mode=%s)", job.name, job.schedule.mode)
        return job

    def register_callable(
        self,
        name: str,
        callback: ScheduleCallback,
        schedule: ScheduleSpec,
        *,
        metadata: Optional[Dict[str, Any]] = None,
        overwrite: bool = False,
    ) -> ScheduledJob:
        job = ScheduledJob(name=name, callback=callback, schedule=schedule, metadata=metadata or {})
        return self.register(job, overwrite=overwrite)

    def cancel(self, name: str) -> None:
        with self._lock:
            self._jobs.pop(name, None)
        LOGGER.info("Cancelled job '%s'", name)

    def pending(self) -> List[ScheduledJob]:
        with self._lock:
            return list(self._jobs.values())

    def due_jobs(self, *, now: Optional[datetime] = None) -> List[ScheduledJob]:
        current = now or datetime.utcnow()
        current = current.replace(second=0, microsecond=0)
        due: List[ScheduledJob] = []
        with self._lock:
            for job in self._jobs.values():
                if job.next_run is None:
                    continue
                if job.next_run <= current:
                    due.append(job)
        return due

    def run_pending(self, *, now: Optional[datetime] = None, catch: bool = True) -> None:
        for job in self.due_jobs(now=now):
            self._run_job(job, now=now, catch=catch)

    def _run_job(self, job: ScheduledJob, *, now: Optional[datetime], catch: bool) -> None:
        start = (now or datetime.utcnow()).replace(second=0, microsecond=0)
        LOGGER.info("Running job '%s'", job.name)
        try:
            job.callback()
        except Exception as exc:  # pragma: no cover - exception path logged
            if catch:
                LOGGER.error("Job '%s' failed: %s", job.name, exc)
            else:
                raise
        finally:
            job.last_run = start
            job.update_next_run(now=start)


def _parse_cron(expr: str) -> List[Optional[Sequence[int]]]:
    parts = [part.strip() for part in expr.split()] if expr else []
    if len(parts) != 5:
        raise ValueError(f"Cron expression must have 5 fields, got {len(parts)}: {expr!r}")
    ranges = [(0, 59), (0, 23), (1, 31), (1, 12), (0, 6)]
    parsed: List[Optional[Sequence[int]]] = []
    for part, (minimum, maximum) in zip(parts, ranges):
        parsed.append(_parse_cron_field(part, minimum, maximum))
    return parsed


def _parse_cron_field(field: str, minimum: int, maximum: int) -> Optional[Sequence[int]]:
    field = field.strip()
    if not field or field in {"*", "?"}:
        return None
    result: set[int] = set()
    for chunk in field.split(","):
        chunk = chunk.strip()
        if not chunk:
            continue
        step = 1
        if "/" in chunk:
            head, step_str = chunk.split("/", 1)
            chunk = head or "*"
            try:
                step_val = int(step_str)
            except ValueError as exc:
                raise ValueError(f"Invalid cron step value: {step_str!r}") from exc
            step = max(1, step_val)
        if chunk in {"*", "?"}:
            start, end = minimum, maximum
        elif "-" in chunk:
            start_str, end_str = chunk.split("-", 1)
            try:
                start = int(start_str)
                end = int(end_str)
            except ValueError as exc:
                raise ValueError(f"Invalid cron range: {chunk!r}") from exc
        else:
            try:
                value = int(chunk)
            except ValueError as exc:
                raise ValueError(f"Invalid cron token: {chunk!r}") from exc
            start = end = value
        if start < minimum or end > maximum:
            raise ValueError(f"Cron value out of range [{minimum}, {maximum}]: {chunk!r}")
        for value in range(start, end + 1, step):
            result.add(value)
    return tuple(sorted(result))
