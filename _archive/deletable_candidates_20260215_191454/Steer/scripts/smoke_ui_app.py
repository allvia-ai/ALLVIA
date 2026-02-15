#!/usr/bin/env python3
"""Lightweight smoke test for the CustomTkinter desktop shell.

The script instantiates the `App`, runs a couple of GUI update cycles, and
destroys the window. This verifies that the UI can boot without launching the
full event loop, which is useful in headless or sandboxed environments.
"""

from __future__ import annotations

import sys
from pathlib import Path

try:
    from tkinter import TclError
except Exception:  # pragma: no cover - Tkinter missing is treated as generic failure
    class TclError(Exception):  # type: ignore[override, misc]
        """Fallback when Tkinter is not importable."""


PROJECT_ROOT = Path(__file__).resolve().parent.parent
if str(PROJECT_ROOT) not in sys.path:
    sys.path.insert(0, str(PROJECT_ROOT))


def main() -> int:
    try:
        from ui.app import App  # pylint: disable=import-error
    except Exception as exc:  # pragma: no cover - import failure surfaces to caller
        print(f"[error] Unable to import ui.app.App: {exc}", file=sys.stderr)
        return 1

    app = App()
    try:
        app.withdraw()  # Prevent the window from flashing when running locally.
        app.update_idletasks()
        app.update()
        app.update_idletasks()
        app.update()
        print("App instantiated, processed updates, and was destroyed cleanly.")
    finally:
        app.destroy()

    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except TclError as err:
        print(f"[warning] Tkinter failed to initialise: {err}", file=sys.stderr)
        print("A graphical display is required to run the desktop shell.", file=sys.stderr)
        sys.exit(2)
