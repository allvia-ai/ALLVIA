from __future__ import annotations

from dataclasses import dataclass
from typing import Optional

from .emit import build_event


@dataclass
class FocusBlockState:
    window_id: str
    app: str
    start_ts: float
    last_title: str = ""


class FocusBlocker:
    def __init__(self, debounce_seconds: float = 2.0) -> None:
        self._debounce_seconds = debounce_seconds
        self._state: Optional[FocusBlockState] = None

    def update(
        self,
        *,
        ts: float,
        window_id: str,
        app: str,
        window_title: str,
    ) -> Optional[dict]:
        if self._state is None:
            self._state = FocusBlockState(window_id, app, ts, window_title)
            return None

        if window_id == self._state.window_id:
            return None

        duration = max(0.0, ts - self._state.start_ts)
        if duration < self._debounce_seconds:
            self._state = FocusBlockState(window_id, app, ts, window_title)
            return None

        event = build_event(
            source="os",
            app=self._state.app,
            event_type="os.app_focus_block",
            resource_type="window",
            resource_id=self._state.window_id,
            payload={
                "duration_sec": int(duration),
                "window_title": self._state.last_title,
            },
            priority="P1",
            window_id=self._state.window_id,
        )

        self._state = FocusBlockState(window_id, app, ts, window_title)
        return event
