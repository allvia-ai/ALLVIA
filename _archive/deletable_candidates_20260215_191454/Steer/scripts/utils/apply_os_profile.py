"""Apply OS-specific smart folder profile placeholders."""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
from typing import Any, Dict, List


def _replace_placeholders(data: Any, user: str) -> Any:
    if isinstance(data, str):
        return data.replace("<USER>", user)
    if isinstance(data, list):
        return [_replace_placeholders(item, user) for item in data]
    if isinstance(data, dict):
        return {k: _replace_placeholders(v, user) for k, v in data.items()}
    return data


def main() -> None:
    parser = argparse.ArgumentParser(description="Apply OS smart folder profile to smart_folders.json")
    parser.add_argument("--profile", required=True, help="Profile JSON (e.g., core/config/os_profiles/smart_folders_macos.json)")
    parser.add_argument("--output", default="core/config/smart_folders.json", help="Destination smart_folders.json")
    parser.add_argument("--user", default=os.getenv("USER") or os.getenv("USERNAME") or "", help="Username placeholder replacement")
    args = parser.parse_args()

    profile_path = Path(args.profile).expanduser()
    output_path = Path(args.output).expanduser()
    user = args.user.strip()

    raw = json.loads(profile_path.read_text(encoding="utf-8"))
    replaced = _replace_placeholders(raw, user) if user else raw
    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_text(json.dumps(replaced, ensure_ascii=False, indent=2), encoding="utf-8")
    print(f"✅ Applied profile {profile_path} → {output_path}")


if __name__ == "__main__":
    main()
