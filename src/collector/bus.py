from __future__ import annotations

import json
import logging
import queue
import threading
import time
from typing import Any, Dict, Optional

from .normalize import NormalizationError, normalize_event
from .privacy import PrivacyGuard
from .priority import PriorityProcessor
from .store import SQLiteStore

try:
    from .observability import Observability
except ImportError:  # pragma: no cover - optional for test import order
    Observability = None  # type: ignore

logger = logging.getLogger(__name__)


class EventBus:
    def __init__(
        self,
        store: SQLiteStore,
        privacy_guard: PrivacyGuard,
        priority: PriorityProcessor,
        validation_level: str = "lenient",
        queue_size: int = 1000,
        insert_batch_size: int = 100,
        insert_flush_ms: int = 1000,
        insert_retry_attempts: int = 3,
        insert_retry_backoff_ms: int = 50,
        metrics: Optional["Observability"] = None,
    ) -> None:
        self._store = store
        self._privacy_guard = privacy_guard
        self._priority = priority
        self._validation_level = validation_level
        self._queue: queue.Queue[Dict[str, Any]] = queue.Queue(maxsize=queue_size)
        self._stop_event = threading.Event()
        self._worker = threading.Thread(target=self._run, daemon=True)
        self._metrics = metrics
        self._buffer = []
        self._last_flush = time.time()
        self._batch_size = max(1, int(insert_batch_size))
        self._flush_interval = max(0.1, int(insert_flush_ms) / 1000.0)
        self._retry_attempts = max(0, int(insert_retry_attempts))
        self._retry_backoff_ms = max(0, int(insert_retry_backoff_ms))

    def start(self) -> None:
        self._worker.start()

    def stop(self, drain_seconds: int = 0) -> None:
        deadline = time.time() + max(0, int(drain_seconds))
        while drain_seconds > 0 and not self._queue.empty() and time.time() < deadline:
            time.sleep(0.05)
        self._stop_event.set()
        self._worker.join(timeout=5)
        self._buffer.extend(self._priority.flush())
        self._flush_buffer(force=True)

    def enqueue(self, event: Dict[str, Any]) -> bool:
        try:
            self._queue.put_nowait(event)
            if self._metrics:
                self._metrics.set_gauge("queue.depth", self._queue.qsize())
            return True
        except queue.Full:
            if self._metrics:
                self._metrics.set_gauge("queue.depth", self._queue.qsize())
            return False

    def _run(self) -> None:
        while not self._stop_event.is_set():
            try:
                item = self._queue.get(timeout=0.5)
            except queue.Empty:
                if self._metrics:
                    self._metrics.set_gauge("queue.depth", self._queue.qsize())
                self._flush_buffer()
                continue
            try:
                envelope = normalize_event(item, validation_level=self._validation_level)
                envelope = self._privacy_guard.apply(envelope)
                if envelope is None:
                    continue
                queue_ratio = _queue_ratio(self._queue)
                for output in self._priority.process(envelope, queue_ratio):
                    self._buffer.append(output)
                    if len(self._buffer) >= self._batch_size:
                        self._flush_buffer(force=True)
                self._flush_buffer()
            except NormalizationError as exc:
                logger.warning("drop event: %s", exc)
                if self._metrics:
                    self._metrics.record_ingest_invalid()
            except Exception:
                logger.exception("failed to process event")
                if self._metrics:
                    self._metrics.record_store_insert_fail()
            finally:
                if self._metrics:
                    self._metrics.set_gauge("queue.depth", self._queue.qsize())
                    self._metrics.maybe_log(logger, self._store.get_db_size())

    def _flush_buffer(self, force: bool = False) -> None:
        if not self._buffer:
            return
        now = time.time()
        if not force and (now - self._last_flush) < self._flush_interval:
            return
        batch = self._buffer
        self._buffer = []
        try:
            self._store.insert_events(
                batch,
                retry_attempts=self._retry_attempts,
                retry_backoff_ms=self._retry_backoff_ms,
            )
            if self._metrics:
                for output in batch:
                    self._metrics.record_priority(output.priority)
                    self._metrics.record_store_insert_ok()
                    self._metrics.set_last_event_ts(output.ts)
                    self._metrics.record_activity(
                        output.app, output.event_type, output.payload, output.priority
                    )
                    activity_payload = self._metrics.activity_block_payload(
                        output.app, output.event_type, output.payload
                    )
                    if activity_payload:
                        logger.info(
                            json.dumps(activity_payload, separators=(",", ":"))
                        )
        except Exception:
            logger.exception("failed to insert batch")
            if self._metrics:
                self._metrics.record_store_insert_fail()
        finally:
            self._last_flush = now


def _queue_ratio(q: queue.Queue) -> float:
    maxsize = q.maxsize
    if maxsize <= 0:
        return 0.0
    return q.qsize() / maxsize
