from __future__ import annotations

import argparse
import logging
import os
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Dict, Iterable, List, Optional, Tuple

from .emit import EmitConfig, HttpEmitter, build_event

logger = logging.getLogger(__name__)


@dataclass
class FileState:
    mtime: float
    size: int


@dataclass
class WatchConfig:
    watch_paths: List[Path]
    exclude_paths: List[Path] = field(default_factory=list)
    poll_interval: float = 2.0
    debounce_seconds: float = 2.0


class FileWatcher:
    def __init__(self, emitter: HttpEmitter, config: WatchConfig) -> None:
        self._emitter = emitter
        self._config = config
        self._snapshot: Dict[str, FileState] = {}
        self._last_emit: Dict[str, float] = {}

    def run(self) -> None:
        self._snapshot = self._scan()
        while True:
            events = self._diff()
            if events:
                self._emitter.send_events(events)
            time.sleep(self._config.poll_interval)

    def _diff(self) -> List[dict]:
        new_snapshot = self._scan()
        now = time.time()
        events: List[dict] = []

        for path, state in new_snapshot.items():
            prev = self._snapshot.get(path)
            if prev is None:
                if self._debounced(path, now):
                    continue
                events.append(self._build_file_event(path, "created"))
            elif state.mtime != prev.mtime or state.size != prev.size:
                if self._debounced(path, now):
                    continue
                events.append(self._build_file_event(path, "modified"))

        for path in self._snapshot.keys() - new_snapshot.keys():
            if self._debounced(path, now):
                continue
            events.append(self._build_file_event(path, "deleted"))

        self._snapshot = new_snapshot
        return events

    def _build_file_event(self, path: str, action: str) -> dict:
        payload = {
            "action": action,
            "path": path,
            "extension": Path(path).suffix.lower().lstrip("."),
        }
        return build_event(
            source="os",
            app="OS",
            event_type="os.file_changed",
            resource_type="file",
            resource_id=path,
            payload=payload,
            priority="P1" if action != "deleted" else "P2",
        )

    def _debounced(self, path: str, now: float) -> bool:
        last = self._last_emit.get(path)
        if last is not None and (now - last) < self._config.debounce_seconds:
            return True
        self._last_emit[path] = now
        return False

    def _scan(self) -> Dict[str, FileState]:
        snapshot: Dict[str, FileState] = {}
        for root in self._config.watch_paths:
            if not root.exists():
                continue
            for dirpath, dirnames, filenames in os.walk(root):
                dir_path = Path(dirpath)
                if self._is_excluded(dir_path):
                    dirnames[:] = []
                    continue
                for name in filenames:
                    file_path = dir_path / name
                    if self._is_excluded(file_path):
                        continue
                    try:
                        stat = file_path.stat()
                    except OSError:
                        continue
                    snapshot[str(file_path)] = FileState(
                        mtime=stat.st_mtime,
                        size=stat.st_size,
                    )
        return snapshot

    def _is_excluded(self, path: Path) -> bool:
        for exclude in self._config.exclude_paths:
            try:
                path.relative_to(exclude)
                return True
            except ValueError:
                continue
        return False


def _parse_paths(values: Iterable[str]) -> List[Path]:
    paths: List[Path] = []
    for value in values:
        for part in value.split(","):
            part = part.strip()
            if part:
                paths.append(Path(part).expanduser())
    return paths


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="File watcher sensor")
    parser.add_argument(
        "--ingest-url",
        default="http://127.0.0.1:8080/events",
        help="collector ingest URL",
    )
    parser.add_argument(
        "--paths",
        action="append",
        default=[],
        help="comma-separated paths to watch",
    )
    parser.add_argument(
        "--exclude",
        action="append",
        default=[],
        help="comma-separated paths to exclude",
    )
    parser.add_argument("--poll", type=float, default=2.0, help="poll interval sec")
    parser.add_argument(
        "--debounce",
        type=float,
        default=2.0,
        help="debounce seconds per file",
    )
    return parser.parse_args()


def main() -> None:
    logging.basicConfig(level=logging.INFO, format="%(asctime)s %(levelname)s %(message)s")
    args = parse_args()
    watch_paths = _parse_paths(args.paths) or [Path.home() / "Documents"]
    exclude_paths = _parse_paths(args.exclude)
    config = WatchConfig(
        watch_paths=watch_paths,
        exclude_paths=exclude_paths,
        poll_interval=args.poll,
        debounce_seconds=args.debounce,
    )
    emitter = HttpEmitter(EmitConfig(ingest_url=args.ingest_url))
    watcher = FileWatcher(emitter, config)
    watcher.run()


if __name__ == "__main__":
    main()
