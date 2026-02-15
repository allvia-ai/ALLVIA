"""Model selection and lifecycle helpers."""
from __future__ import annotations

import threading
from dataclasses import dataclass
from typing import Any, Callable, Dict, Iterable, Optional

from core.utils import get_logger

LOGGER = get_logger("infra.models")


@dataclass
class ModelSelector:
    registry: Dict[str, str]
    default_model: str

    def pick(self, policy_tag: Optional[str] = None, prefer_gpu: bool = False) -> str:
        key = f"{policy_tag or 'default'}:{'gpu' if prefer_gpu else 'cpu'}"
        return self.registry.get(key, self.registry.get(policy_tag or "default", self.default_model))


class ModelManager:
    """Caches loaded models and tracks reference counts for reuse."""

    def __init__(
        self,
        loader: Callable[[str], Any],
        *,
        disposer: Optional[Callable[[Any], None]] = None,
    ) -> None:
        self._loader = loader
        self._disposer = disposer
        self._lock = threading.RLock()
        self._cache: Dict[str, Any] = {}
        self._refcounts: Dict[str, int] = {}

    def get(self, name: str, *, auto_load: bool = True) -> Optional[Any]:
        normalized = name.strip()
        if not normalized:
            raise ValueError("Model name must be a non-empty string")
        with self._lock:
            cached = self._cache.get(normalized)
            if cached is not None:
                self._refcounts[normalized] = self._refcounts.get(normalized, 0) + 1
                return cached
        if not auto_load:
            return None
        model = self._loader(normalized)
        with self._lock:
            self._cache[normalized] = model
            self._refcounts[normalized] = self._refcounts.get(normalized, 0) + 1
        LOGGER.info("Model '%s' loaded (refcount=%d)", normalized, self._refcounts[normalized])
        return model

    def preload(self, names: Iterable[str]) -> None:
        for name in names:
            self.get(name)

    def release(self, name: str) -> None:
        normalized = name.strip()
        if not normalized:
            return
        with self._lock:
            count = self._refcounts.get(normalized, 0)
            if count <= 1:
                self._refcounts.pop(normalized, None)
                model = self._cache.pop(normalized, None)
            else:
                self._refcounts[normalized] = count - 1
                model = None
        if model is not None and self._disposer:
            try:
                self._disposer(model)
            except Exception:  # pragma: no cover - defensive cleanup
                LOGGER.warning("Model disposer raised an error for '%s'", normalized)

    def clear(self) -> None:
        with self._lock:
            cached = list(self._cache.items())
            self._cache.clear()
            self._refcounts.clear()
        if self._disposer:
            for name, model in cached:
                try:
                    self._disposer(model)
                except Exception:
                    LOGGER.warning("Model disposer raised an error for '%s'", name)

    def stats(self) -> Dict[str, int]:
        with self._lock:
            return dict(self._refcounts)
