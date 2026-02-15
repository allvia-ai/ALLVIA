# core/conversation/chat_ui.py
"""
UI components for the LNPChat system (Console, Spinner, etc).
"""
from __future__ import annotations

import sys
import threading
import time
from typing import Optional

try:
    from rich.console import Console
    from rich.markdown import Markdown
    from rich.panel import Panel
    from rich.text import Text
    HAS_RICH = True
except ImportError:
    HAS_RICH = False

class ChatUI:
    """Encapsulates console I/O and visual feedback."""
    
    def __init__(self):
        self._console = Console() if HAS_RICH else None

    def print_system(self, message: str, title: Optional[str] = None) -> None:
        """Print a system message (e.g. from the AI)."""
        if HAS_RICH and self._console:
            panel = Panel(
                Markdown(message),
                title=title or "InfoPilot",
                border_style="blue",
                padding=(1, 2),
            )
            self._console.print(panel)
        else:
            print(f"\n[InfoPilot] {message}\n")

    def print_user(self, query: str) -> None:
        """Print the user's query confirmation."""
        if HAS_RICH and self._console:
            self._console.print(f"\n[bold green]User:[/bold green] {query}")
        else:
            print(f"\nUser: {query}")

    def print_error(self, message: str) -> None:
        if HAS_RICH and self._console:
            self._console.print(f"[bold red]Error:[/bold red] {message}")
        else:
            print(f"Error: {message}")

    def spinner(self, prefix: str = "Processing") -> "Spinner":
        return Spinner(prefix=prefix)


class Spinner:
    """
    Simpler console spinner context manager.
    Updates in a background thread so it doesn't block main execution.
    """
    FRAMES = ["|", "/", "-", "\\"]

    def __init__(self, prefix="검색 준비", interval=0.12):
        self.prefix = prefix
        self.interval = interval
        self._stop = threading.Event()
        self._t = None
        self._i = 0
        self._enabled = False # Force disable to prevent BlockingIO in GUI Bridge pipeline

    def __enter__(self):
        self.start()
        return self

    def __exit__(self, exc_type, exc_val, exc_tb):
        self.stop()

    def start(self):
        if not self._enabled:
            return
        self._stop.clear()
        self._t = threading.Thread(target=self._run, daemon=True)
        self._t.start()

    def _run(self):
        while not self._stop.is_set():
            frame = self.FRAMES[self._i % len(self.FRAMES)]
            sys.stdout.write(f"\r{self.prefix} {frame} ")
            sys.stdout.flush()
            time.sleep(self.interval)
            self._i += 1

    def stop(self, clear=True):
        if not self._enabled:
            return
        self._stop.set()
        if self._t:
            self._t.join()
        if clear:
            sys.stdout.write("\r" + " " * (len(self.prefix) + 10) + "\r")
            sys.stdout.flush()
