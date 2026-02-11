from __future__ import annotations

import argparse
import ctypes
import logging
import sys
import time
from ctypes import wintypes
from typing import Optional

from .emit import EmitConfig, HttpEmitter, build_event

logger = logging.getLogger(__name__)


class LASTINPUTINFO(ctypes.Structure):
    _fields_ = [("cbSize", wintypes.UINT), ("dwTime", wintypes.DWORD)]


class IdleSensor:
    def __init__(
        self,
        emitter: HttpEmitter,
        idle_threshold_sec: int = 900,
        poll_interval: float = 5.0,
    ) -> None:
        self._emitter = emitter
        self._idle_threshold_sec = idle_threshold_sec
        self._poll_interval = poll_interval
        self._idle = False

    def run(self) -> None:
        if sys.platform != "win32":
            logger.error("windows_idle sensor requires Windows")
            return

        while True:
            idle_sec = _get_idle_seconds()
            if idle_sec is None:
                time.sleep(self._poll_interval)
                continue

            if not self._idle and idle_sec >= self._idle_threshold_sec:
                self._idle = True
                self._emit("os.idle_start", idle_sec)
            elif self._idle and idle_sec < self._idle_threshold_sec:
                self._idle = False
                self._emit("os.idle_end", idle_sec)

            time.sleep(self._poll_interval)

    def _emit(self, event_type: str, idle_sec: float) -> None:
        event = build_event(
            source="os",
            app="OS",
            event_type=event_type,
            resource_type="device",
            resource_id="local",
            payload={
                "idle_threshold_sec": self._idle_threshold_sec,
                "idle_sec": int(idle_sec),
            },
            priority="P1",
        )
        self._emitter.send_event(event)


def _get_idle_seconds() -> Optional[float]:
    user32 = ctypes.WinDLL("user32", use_last_error=True)
    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)

    last_input = LASTINPUTINFO()
    last_input.cbSize = ctypes.sizeof(LASTINPUTINFO)
    if not user32.GetLastInputInfo(ctypes.byref(last_input)):
        return None

    tick_count = kernel32.GetTickCount()
    idle_ms = tick_count - last_input.dwTime
    return idle_ms / 1000.0


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Windows idle sensor")
    parser.add_argument(
        "--ingest-url",
        default="http://127.0.0.1:8080/events",
        help="collector ingest URL",
    )
    parser.add_argument(
        "--idle-threshold",
        type=int,
        default=900,
        help="idle threshold in seconds",
    )
    parser.add_argument("--poll", type=float, default=5.0, help="poll interval sec")
    return parser.parse_args()


def main() -> None:
    logging.basicConfig(level=logging.INFO, format="%(asctime)s %(levelname)s %(message)s")
    args = parse_args()
    emitter = HttpEmitter(EmitConfig(ingest_url=args.ingest_url))
    sensor = IdleSensor(
        emitter,
        idle_threshold_sec=args.idle_threshold,
        poll_interval=args.poll,
    )
    sensor.run()


if __name__ == "__main__":
    main()
