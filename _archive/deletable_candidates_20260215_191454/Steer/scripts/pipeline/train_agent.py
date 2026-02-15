"""Command-line entrypoint for queuing training jobs."""
from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any, Dict, List

import sys

ROOT = Path(__file__).resolve().parents[2]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from core.api.training import queue_training_job


def parse_args(argv: List[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Queue InfoPilot training jobs")
    subparsers = parser.add_subparsers(dest="command", required=True)

    global_parser = subparsers.add_parser("global", help="Queue a global training job")
    global_parser.add_argument(
        "--roots",
        nargs="*",
        default=[],
        help="Root directories to scan (default: install-time root)",
    )
    global_parser.add_argument(
        "--exclude",
        nargs="*",
        default=[],
        help="Directories to exclude",
    )
    global_parser.add_argument(
        "--policy",
        default=None,
        help="Optional policy file overriding defaults",
    )

    folder_parser = subparsers.add_parser("smart-folder", help="Queue a smart-folder training job")
    folder_parser.add_argument("folder_id", help="Smart folder identifier")
    folder_parser.add_argument("path", help="Absolute path to folder root")
    folder_parser.add_argument(
        "--types",
        nargs="*",
        default=["documents"],
        help="Asset types to include (documents/images/audio)",
    )

    parser.add_argument("--json", action="store_true", help="Print JSON response")

    return parser.parse_args(argv)


def main(argv: List[str] | None = None) -> int:
    args = parse_args(argv)

    if args.command == "global":
        payload: Dict[str, Any] = {
            "mode": "global",
            "roots": args.roots,
            "exclude": args.exclude,
        }
        if args.policy:
            payload["policy"] = args.policy
        job = queue_training_job("global", payload)
    else:
        payload = {
            "mode": "smart-folder",
            "folder_id": args.folder_id,
            "path": args.path,
            "types": args.types,
        }
        job = queue_training_job("smart-folder", payload)

    if args.json:
        print(json.dumps({"id": job.id, "status": job.status, "mode": job.mode, "payload": job.payload}, ensure_ascii=False))
    else:
        print(f"Queued training job {job.id} (mode={job.mode})")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
