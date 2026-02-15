# core/utils/cli_ui.py
from __future__ import annotations

import sys
import time
import threading

class Spinner:
    FRAMES = ["|", "/", "-", "\\"]

    def __init__(self, prefix="", interval=0.12):
        self.prefix = prefix
        self.interval = interval
        self._stop = threading.Event()
        self._t = None
        self._i = 0

    def start(self):
        if self._t:
            return
        self._stop.clear()
        self._t = threading.Thread(target=self._run, daemon=True)
        self._t.start()

    def _run(self):
        while not self._stop.is_set():
            frame = self.FRAMES[self._i % len(self.FRAMES)]
            sys.stdout.write(f"\r{self.prefix} {frame}")
            sys.stdout.flush()
            time.sleep(self.interval)
            self._i += 1

    def stop(self, clear=True):
        self._stop.set()
        if self._t:
            self._t.join()
            self._t = None
        if clear:
            sys.stdout.write("\r" + " " * (len(self.prefix) + 10) + "\r")
            sys.stdout.flush()

class ProgressLine:
    def __init__(self, total: int, label: str, update_every: int = 10):
        self.total = max(1, total)
        self.label = label
        self.update_every = max(1, update_every)
        self.start = time.time()
        self.n = 0

    def update(self, k: int = 1):
        self.n += k
        if self.n % self.update_every == 0 or self.n >= self.total:
            pct = 100.0 * self.n / self.total
            elapsed = time.time() - self.start
            rate = self.n / max(0.001, elapsed)
            sys.stdout.write(
                f"\r{self.label}: {self.n}/{self.total} ({pct:.1f}%) "
                f"- {self._fmt(elapsed)} ({rate:.1f} it/s)"
            )
            sys.stdout.flush()

    def close(self):
        sys.stdout.write("\n")
        sys.stdout.flush()

    @staticmethod
    def _fmt(s: float) -> str:
        if s < 60:
            return f"{s:.1f}s"
        m = int(s // 60)
        s = int(s % 60)
        return f"{m}m {s}s"
