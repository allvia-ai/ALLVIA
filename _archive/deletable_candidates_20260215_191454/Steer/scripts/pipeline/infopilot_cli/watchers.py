# scripts/pipeline/infopilot_cli/watchers.py
from __future__ import annotations

import logging
import queue
import time
from pathlib import Path
from typing import Callable, Optional, Set, Tuple

# Third-party
try:
    from watchdog.events import FileSystemEventHandler
except ImportError:
    FileSystemEventHandler = object  # Fallback

from core.policy.engine import PolicyEngine

logger = logging.getLogger(__name__)


class WatchEventHandler(FileSystemEventHandler):
    """Handles file system events for the document corpus."""

    def __init__(
        self,
        event_queue: "queue.Queue[Tuple[str, str]]",
        allowed_exts: Set[str],
        *,
        policy_engine: Optional[PolicyEngine] = None,
        policy_engine_provider: Optional[Callable[[], Optional[PolicyEngine]]] = None,
        ignore_paths: Optional[Set[str]] = None,
        agent: str,
        base_handler_cls: type,
    ):
        base_handler_cls.__init__(self)
        self._queue = event_queue
        self._allowed_exts = allowed_exts
        self._policy_engine = policy_engine
        self._policy_engine_provider = policy_engine_provider
        self._ignore_paths = ignore_paths or set()
        self._policy_agent = agent

    def _normalize_path(self, raw: str) -> str:
        # unicodedata normalization could go here if needed
        return str(Path(raw).resolve())

    def _current_policy_engine(self) -> Optional[PolicyEngine]:
        if self._policy_engine:
            return self._policy_engine
        if self._policy_engine_provider:
            return self._policy_engine_provider()
        return None

    def _should_process(self, path: str) -> bool:
        if any(ign in path for ign in self._ignore_paths):
            return False
        
        # Policy check
        engine = self._current_policy_engine()
        if engine:
            if not engine.check(Path(path), self._policy_agent):
                return False

        # Extension check
        ext = Path(path).suffix.lower()
        return ext in self._allowed_exts

    def on_created(self, event):
        if event.is_directory:
            return
        path = self._normalize_path(event.src_path)
        if self._should_process(path):
            self._queue.put(("add", path))

    def on_modified(self, event):
        if event.is_directory:
            return
        path = self._normalize_path(event.src_path)
        if self._should_process(path):
            self._queue.put(("add", path))

    def on_moved(self, event):
        if event.is_directory:
            return
        src = self._normalize_path(event.src_path)
        dest = self._normalize_path(event.dest_path)
        
        # Treat move as delete + add
        # (Though we might not strictly need to delete if src wasn't tracked, 
        #  but safe to enqueue remove just in case)
        self._queue.put(("remove", src))
        
        if self._should_process(dest):
            self._queue.put(("add", dest))

    def on_deleted(self, event):
        if event.is_directory:
            return
        path = self._normalize_path(event.src_path)
        self._queue.put(("remove", path))


class PolicyEventHandler(FileSystemEventHandler):
    """Monitors the policy file (smart folders) for changes."""

    def __init__(
        self,
        event_queue: "queue.Queue[Tuple[str, str]]",
        policy_path: Path,
        *,
        base_handler_cls: type,
    ) -> None:
        base_handler_cls.__init__(self)
        self._queue = event_queue
        self._policy_path = self._normalize_path(policy_path)

    def _normalize_path(self, path: Path) -> Path:
        return path.resolve()

    def _is_target(self, raw: str) -> bool:
        try:
            p = Path(raw).resolve()
            return p == self._policy_path
        except Exception:
            return False

    def on_created(self, event):
        if not event.is_directory and self._is_target(event.src_path):
            logger.info("📜 Policy file created.")
            self._queue.put(("policy_reload", ""))

    def on_modified(self, event):
        if not event.is_directory and self._is_target(event.src_path):
            logger.info("📜 Policy file modified.")
            self._queue.put(("policy_reload", ""))

    def on_moved(self, event):
        if not event.is_directory and self._is_target(event.dest_path):
            logger.info("📜 Policy file moved/renamed.")
            self._queue.put(("policy_reload", ""))

    def on_deleted(self, event):
        if not event.is_directory and self._is_target(event.src_path):
            logger.warning("📜 Policy file deleted.")
            # Depending on logic, might want to clear policy or just warn
            self._queue.put(("policy_reload", ""))
