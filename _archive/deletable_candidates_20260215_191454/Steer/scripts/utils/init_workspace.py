"""Initialize default workspace directories based on smart folder config."""
from __future__ import annotations

import json
import os
import re
import sys
from pathlib import Path

CONFIG_PATH = Path(__file__).resolve().parents[2] / "core" / "config" / "smart_folders.json"

_WINDOWS_ABS_RE = re.compile(r"^[a-zA-Z]:[\\\\/]")


def expand(path: str) -> Path:
    expanded = os.path.expanduser(path)
    if "<USER>" in expanded:
        raise ValueError("unresolved <USER> placeholder")
    if _WINDOWS_ABS_RE.match(expanded):
        if os.name != "nt":
            raise ValueError("windows absolute path on non-windows host")
        return Path(expanded).resolve()
    candidate = Path(expanded)
    if candidate.is_absolute():
        return candidate.resolve()
    raise ValueError("expected absolute path (use ~ or an absolute path)")


def ensure_directory(path: Path) -> None:
    path.mkdir(parents=True, exist_ok=True)


def main() -> None:
    if not CONFIG_PATH.exists():
        raise SystemExit(f"Config file not found: {CONFIG_PATH}")

    with CONFIG_PATH.open("r", encoding="utf-8") as fp:
        entries = json.load(fp)

    created = []
    skipped: list[tuple[str, str]] = []
    base_root = expand("~/Desktop/AI Summary")
    ensure_directory(base_root)

    for entry in entries:
        raw_path = entry.get("path")
        if not raw_path:
            continue
        try:
            target = expand(str(raw_path))
        except ValueError as exc:
            skipped.append((str(raw_path), str(exc)))
            continue
        ensure_directory(target)
        created.append(target)

    print("✅ Workspace initialized. Directories:")
    for target in created:
        print(f"  - {target}")
    if skipped:
        print("\n⚠️ Skipped invalid smart folder paths:")
        for raw, reason in skipped:
            print(f"  - {raw} ({reason})")
        print(
            "\nHint: run `python3 scripts/util/setup_profiles.py --validate` "
            "to generate `core/config/smart_folders.json` for your OS.",
            file=sys.stderr,
        )


if __name__ == "__main__":
    main()
