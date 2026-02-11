from __future__ import annotations

import argparse
import ctypes
import logging
import sys
import time
from ctypes import wintypes
from typing import Optional, Tuple

from .emit import EmitConfig, HttpEmitter, build_event

logger = logging.getLogger(__name__)

PROCESS_QUERY_LIMITED_INFORMATION = 0x1000
PROCESS_VM_READ = 0x0010


class ForegroundSensor:
    def __init__(self, emitter: HttpEmitter, poll_interval: float = 1.0) -> None:
        self._emitter = emitter
        self._poll_interval = poll_interval
        self._last_window: Optional[int] = None

    def run(self) -> None:
        if sys.platform != "win32":
            logger.error("windows_foreground sensor requires Windows")
            return

        while True:
            info = _get_foreground_info()
            if info and info[0] != self._last_window:
                hwnd, pid, title, app = info
                self._last_window = hwnd
                event = build_event(
                    source="os",
                    app=app,
                    event_type="os.foreground_changed",
                    resource_type="window",
                    resource_id=str(hwnd),
                    payload={"window_title": title},
                    priority="P2",
                    window_id=str(hwnd),
                    pid=pid,
                )
                self._emitter.send_event(event)
            time.sleep(self._poll_interval)


def _get_foreground_info() -> Optional[Tuple[int, int, str, str]]:
    user32 = ctypes.WinDLL("user32", use_last_error=True)
    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    psapi = ctypes.WinDLL("psapi", use_last_error=True)

    hwnd = user32.GetForegroundWindow()
    if not hwnd:
        return None

    pid = wintypes.DWORD()
    user32.GetWindowThreadProcessId(hwnd, ctypes.byref(pid))

    title = _get_window_text(user32, hwnd)
    app = _get_process_name(kernel32, psapi, pid.value)

    return hwnd, pid.value, title, app


def _get_window_text(user32, hwnd: int) -> str:
    length = user32.GetWindowTextLengthW(hwnd)
    if length == 0:
        return ""
    buffer = ctypes.create_unicode_buffer(length + 1)
    user32.GetWindowTextW(hwnd, buffer, length + 1)
    return buffer.value


def _get_process_name(kernel32, psapi, pid: int) -> str:
    handle = kernel32.OpenProcess(
        PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_VM_READ, False, pid
    )
    if not handle:
        return "UNKNOWN"

    try:
        buffer = ctypes.create_unicode_buffer(260)
        if psapi.GetModuleBaseNameW(handle, None, buffer, 260) == 0:
            return "UNKNOWN"
        name = buffer.value
        if not name:
            return "UNKNOWN"
        return name.upper()
    finally:
        kernel32.CloseHandle(handle)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Windows foreground sensor")
    parser.add_argument(
        "--ingest-url",
        default="http://127.0.0.1:8080/events",
        help="collector ingest URL",
    )
    parser.add_argument("--poll", type=float, default=1.0, help="poll interval sec")
    return parser.parse_args()


def main() -> None:
    logging.basicConfig(level=logging.INFO, format="%(asctime)s %(levelname)s %(message)s")
    args = parse_args()
    emitter = HttpEmitter(EmitConfig(ingest_url=args.ingest_url))
    sensor = ForegroundSensor(emitter, poll_interval=args.poll)
    sensor.run()


if __name__ == "__main__":
    main()
