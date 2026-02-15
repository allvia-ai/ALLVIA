"""Setup helper to apply OS-specific smart folder profiles."""
from __future__ import annotations

import argparse
import platform
import subprocess
import sys
from pathlib import Path


def main() -> None:
    parser = argparse.ArgumentParser(description="Apply smart folder profile for current OS")
    parser.add_argument("--user", dest="user", default="", help="Username for placeholder replacement")
    parser.add_argument(
        "--profile",
        dest="profile",
        default="",
        help="Override profile path (otherwise auto-select by OS)",
    )
    parser.add_argument(
        "--validate",
        action="store_true",
        help="Validate smart_folders.json after applying profile",
    )
    args = parser.parse_args()

    system = platform.system().lower()
    default_profile = None
    if not args.profile:
        if system == "darwin":
            default_profile = "core/config/os_profiles/smart_folders_macos.json"
        elif system == "windows":
            default_profile = "core/config/os_profiles/smart_folders_windows.json"
    profile = args.profile or default_profile
    if not profile:
        raise SystemExit("No profile selected. Use --profile to specify a profile JSON.")

    python = sys.executable or "python3"
    cmd = [
        python,
        "scripts/util/apply_os_profile.py",
        "--profile",
        profile,
    ]
    if args.user:
        cmd.extend(["--user", args.user])

    print("Applying profile:", profile)
    subprocess.check_call(cmd)
    if args.validate:
        subprocess.check_call(
            [python, "scripts/util/validate_smart_folders.py", "--config", "core/config/smart_folders.json"]
        )


if __name__ == "__main__":
    main()
