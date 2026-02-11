from __future__ import annotations

import os
import subprocess
import sys
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[1]
MANIFEST_PATH = PROJECT_ROOT / "core" / "Cargo.toml"


def _binary_path() -> Path:
    name = "build_handoff_rs.exe" if os.name == "nt" else "build_handoff_rs"
    return PROJECT_ROOT / "core" / "target" / "debug" / name


def _ensure_binary(path: Path) -> None:
    if path.exists():
        return
    subprocess.run(
        [
            "cargo",
            "build",
            "--manifest-path",
            str(MANIFEST_PATH),
            "--bin",
            "build_handoff_rs",
        ],
        cwd=str(PROJECT_ROOT),
        check=True,
    )


def main() -> None:
    binary = _binary_path()
    _ensure_binary(binary)
    result = subprocess.run([str(binary), *sys.argv[1:]], cwd=str(PROJECT_ROOT))
    raise SystemExit(result.returncode)


if __name__ == "__main__":
    main()
