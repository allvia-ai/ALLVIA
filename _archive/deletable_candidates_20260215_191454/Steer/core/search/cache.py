# cache.py - Query and Signature Caching for Retriever
"""
Caching utilities for the search retriever module.
Extracted from retriever.py for modularity.
"""
from __future__ import annotations

import copy
import threading
from collections import OrderedDict
from typing import Any, Callable, List, Optional, Tuple

try:
    import numpy as np
except Exception:
    np = None  # type: ignore

# Lazy import to avoid circular dependency
def _get_vector_index():
    from .vector_index import VectorIndex
    return VectorIndex


class CacheSignatureMonitor:
    """Polls cache signature changes and invokes a callback when stable."""

    def __init__(
        self,
        compute_signature: Callable[[], Tuple[float, float, float]],
        on_change: Callable[[Tuple[float, float, float], Tuple[float, float, float]], None],
        *,
        interval: float = 1.5,
        stability_checks: int = 2,
        thread_name: str = "retriever-cache-monitor",
    ) -> None:
        self._compute_signature = compute_signature
        self._on_change = on_change
        self._interval = max(0.1, float(interval))
        self._stability_checks = max(1, int(stability_checks))
        self._thread_name = thread_name

        self._lock = threading.Lock()
        self._stop_event = threading.Event()
        self._thread: Optional[threading.Thread] = None

        self._last_signature: Optional[Tuple[float, float, float]] = None
        self._pending_signature: Optional[Tuple[float, float, float]] = None
        self._pending_hits = 0
        self._last_error: Optional[BaseException] = None

    # ------------------------------------------------------------------
    # Control
    # ------------------------------------------------------------------
    def start(self) -> None:
        with self._lock:
            if self._thread and self._thread.is_alive():
                return
            self._stop_event.clear()
            self._thread = threading.Thread(target=self._run, name=self._thread_name, daemon=True)
            self._thread.start()

    def stop(self) -> None:
        with self._lock:
            thread = self._thread
            if not thread:
                self._stop_event.set()
                return
        self._stop_event.set()
        thread.join(timeout=max(0.1, self._interval * 2))
        with self._lock:
            self._thread = None

    close = stop

    def is_running(self) -> bool:
        with self._lock:
            return bool(self._thread and self._thread.is_alive())

    # ------------------------------------------------------------------
    # Public helpers
    # ------------------------------------------------------------------
    def prime(self, signature: Tuple[float, float, float]) -> None:
        with self._lock:
            self._last_signature = signature
            self._pending_signature = None
            self._pending_hits = 0

    def check_once(self) -> None:
        try:
            current = self._compute_signature()
        except BaseException as exc:  # pragma: no cover - defensive guard
            self._last_error = exc
            return

        with self._lock:
            previous = self._last_signature
            if previous is None:
                self._last_signature = current
                self._pending_signature = None
                self._pending_hits = 0
                return

            if current == previous:
                self._pending_signature = None
                self._pending_hits = 0
                return

            if self._pending_signature != current:
                self._pending_signature = current
                self._pending_hits = 1
                return

            self._pending_hits += 1
            if self._pending_hits < self._stability_checks:
                return

            self._pending_signature = None
            self._pending_hits = 0
            self._last_signature = current
            before = previous

        try:
            self._on_change(before, current)
        except Exception as exc:  # pragma: no cover - defensive guard
            self._last_error = exc

    @property
    def last_signature(self) -> Optional[Tuple[float, float, float]]:
        with self._lock:
            return self._last_signature

    @property
    def last_error(self) -> Optional[BaseException]:
        return self._last_error

    # ------------------------------------------------------------------
    # Internal
    # ------------------------------------------------------------------
    def _run(self) -> None:
        while not self._stop_event.is_set():
            self.check_once()
            self._stop_event.wait(self._interval)


class QueryResultCache:
    """Simple LRU cache for storing annotated search results."""

    def __init__(self, max_entries: int = 128) -> None:
        self.max_entries = max(1, int(max_entries or 1))
        self._store: "OrderedDict[Any, Any]" = OrderedDict()

    def get(self, key: Any) -> Optional[Any]:
        try:
            value = self._store.pop(key)
        except KeyError:
            return None
        self._store[key] = value
        return copy.deepcopy(value)

    def set(self, key: Any, value: Any) -> None:
        self._store[key] = copy.deepcopy(value)
        self._store.move_to_end(key)
        while len(self._store) > self.max_entries:
            self._store.popitem(last=False)

    def clear(self) -> None:
        self._store.clear()


class SemanticQueryCache:
    """Stores query vectors and associated results for approximate reuse."""

    def __init__(self, max_entries: int = 64, threshold: float = 0.97) -> None:
        self.max_entries = max(1, int(max_entries or 1))
        self.threshold = max(0.0, min(1.0, float(threshold)))
        self._entries: List[Tuple[Any, Any]] = []  # (vector, payload)

    def match(self, vector: Any) -> Optional[Any]:
        if not self._entries or np is None:
            return None
        VectorIndex = _get_vector_index()
        try:
            candidate = VectorIndex._normalize_vector(vector)
        except Exception:
            return None

        best_idx: Optional[int] = None
        best_score = self.threshold
        for idx, (entry_vec, entry_payload) in enumerate(self._entries):
            try:
                score = float(np.dot(entry_vec, candidate))
            except Exception:
                continue
            if score >= best_score:
                best_idx = idx
                best_score = score

        if best_idx is None:
            return None

        entry_vec, entry_payload = self._entries.pop(best_idx)
        self._entries.append((entry_vec, entry_payload))
        return copy.deepcopy(entry_payload)

    def store(self, vector: Any, payload: Any) -> None:
        if np is None:
            return
        VectorIndex = _get_vector_index()
        try:
            normalised = VectorIndex._normalize_vector(vector)
        except Exception:
            return
        self._entries.append((normalised, copy.deepcopy(payload)))
        if len(self._entries) > self.max_entries:
            self._entries.pop(0)

    def clear(self) -> None:
        self._entries.clear()

    def set_threshold(self, new_threshold: float) -> None:
        self.threshold = max(0.0, min(1.0, float(new_threshold)))
