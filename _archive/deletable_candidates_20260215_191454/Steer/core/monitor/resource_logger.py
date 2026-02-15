"""psutil 기반 주기적 리소스 로깅."""

from __future__ import annotations

import json
import threading
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Optional

try:
    import psutil
except Exception:  # pragma: no cover - optional dep
    psutil = None  # type: ignore[assignment]


@dataclass
class ResourceSample:
    timestamp: float
    context: str
    cpu_percent: float
    memory_percent: float
    rss: int
    available: int

    def to_dict(self) -> dict:
        return {
            "ts": self.timestamp,
            "context": self.context,
            "cpu_percent": self.cpu_percent,
            "memory_percent": self.memory_percent,
            "rss": self.rss,
            "available": self.available,
        }


class ResourceLogger:
    """Background psutil sampler that appends to a JSONL file."""

    def __init__(self, log_path: Path, interval: float = 30.0) -> None:
        self.log_path = Path(log_path)
        self.interval = max(1.0, float(interval))
        self._thread: Optional[threading.Thread] = None
        self._stop = threading.Event()
        self._context = "pipeline"

    def _write_sample(self) -> None:
        if psutil is None:
            return
        process = psutil.Process()
        mem = process.memory_info()
        sample = ResourceSample(
            timestamp=time.time(),
            context=self._context,
            cpu_percent=psutil.cpu_percent(interval=None),
            memory_percent=psutil.virtual_memory().percent,
            rss=int(mem.rss),
            available=int(psutil.virtual_memory().available),
        )
        self.log_path.parent.mkdir(parents=True, exist_ok=True)
        with self.log_path.open("a", encoding="utf-8") as f:
            f.write(json.dumps(sample.to_dict(), ensure_ascii=False))
            f.write("\n")

    def start(self, context: str = "pipeline") -> None:
        if psutil is None:
            print("⚠️ psutil 이 설치되어 있지 않아 리소스 로깅을 생략합니다.")
            return
        if self._thread:
            return
        self._context = context
        self._stop.clear()
        self._write_sample()

        def _run() -> None:
            while not self._stop.wait(self.interval):
                self._write_sample()

        self._thread = threading.Thread(target=_run, daemon=True)
        self._thread.start()

    def stop(self) -> None:
        if not self._thread:
            return
        self._stop.set()
        self._thread.join(timeout=self.interval + 1)
        self._thread = None

    def __enter__(self):
        self.start()
        return self

    def __exit__(self, exc_type, exc, tb):
        self.stop()

