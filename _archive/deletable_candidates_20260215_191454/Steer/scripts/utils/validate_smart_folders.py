"""Validate smart_folders.json after placeholder substitution."""
from __future__ import annotations

import argparse
import json
from pathlib import Path


def main() -> None:
    parser = argparse.ArgumentParser(description="Validate smart_folders.json paths")
    parser.add_argument("--config", default="core/config/smart_folders.json", help="smart_folders.json path")
    args = parser.parse_args()

    path = Path(args.config).expanduser()
    data = json.loads(path.read_text(encoding="utf-8"))
    missing = []
    for entry in data:
        raw = entry.get("path", "")
        if not raw:
            continue
        p = Path(raw).expanduser()
        if not p.exists():
            missing.append(p)
    if missing:
        print("⚠️ 접근 불가 경로:")
        for m in missing:
            print(f" - {m}")
    else:
        print("✅ 모든 스마트 폴더 경로 접근 가능")


if __name__ == "__main__":
    main()
