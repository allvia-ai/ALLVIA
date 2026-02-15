"""Index management utilities for lightweight background loading/building."""
from __future__ import annotations

import threading
from concurrent.futures import Future, ThreadPoolExecutor
from typing import Callable, Optional, TypeVar

T = TypeVar("T")


class IndexManager:
    """Coordinates loading and rebuilding of a document index in the background."""

    def __init__(
        self,
        loader: Callable[[], Optional[T]],
        builder: Callable[[], T],
        *,
        max_workers: int = 1,
        thread_name_prefix: str = "index-manager",
    ) -> None:
        self._loader = loader
        self._builder = builder
        self._executor = ThreadPoolExecutor(max_workers=max_workers, thread_name_prefix=thread_name_prefix)
        self._lock = threading.Lock()
        self._ready = threading.Event()
        self._index: Optional[T] = None
        self._future: Optional[Future[T]] = None
        self._last_error: Optional[BaseException] = None

    # ------------------------------------------------------------------
    # Lifecycle helpers
    # ------------------------------------------------------------------
    def ensure_loaded(self) -> Optional[T]:
        """Attempt to synchronously load a cached index, scheduling a rebuild if missing."""
        with self._lock:
            if self._index is not None:
                return self._index

        index = self._loader()
        if index is not None:
            self._set_index(index)
            return index

        self.schedule_rebuild()
        return None

    def schedule_rebuild(self, *, priority: bool = False) -> Future[T]:
        """Schedule a background rebuild of the index."""
        with self._lock:
            if self._future and not self._future.done():
                if not priority:
                    return self._future
                self._future.cancel()

            self._ready.clear()
            self._future = self._executor.submit(self._build_and_set)
            return self._future

    def clear(self) -> None:
        """Drop the in-memory index reference."""
        with self._lock:
            self._index = None
            self._ready.clear()

    # ------------------------------------------------------------------
    # Query helpers
    # ------------------------------------------------------------------
    def get_index(self, *, wait: bool = False, timeout: Optional[float] = None) -> Optional[T]:
        if wait:
            self._ready.wait(timeout=timeout)
        with self._lock:
            return self._index

    def wait_until_ready(self, timeout: Optional[float] = None) -> bool:
        return self._ready.wait(timeout=timeout)

    @property
    def last_error(self) -> Optional[BaseException]:
        return self._last_error

    # ------------------------------------------------------------------
    # Internal
    # ------------------------------------------------------------------
    def _build_and_set(self) -> T:
        try:
            index = self._builder()
        except BaseException as exc:  # pragma: no cover - defensive safeguard
            self._last_error = exc
            self._ready.set()
            raise
        self._set_index(index)
        return index

    def _set_index(self, index: T) -> None:
        with self._lock:
            self._index = index
            self._last_error = None
            self._ready.set()

    # ------------------------------------------------------------------
    # Shutdown
    # ------------------------------------------------------------------
    def shutdown(self) -> None:
        self._executor.shutdown(wait=False)

    def __del__(self) -> None:  # pragma: no cover - best-effort cleanup
        try:
            self.shutdown()
        except Exception:
            pass
