"""Command line helpers for meeting analytics operations."""
from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Optional

from core.agents.meeting.pipeline import MeetingPipeline

from .analytics import format_dashboard, load_dashboard
from .ingest import ingest_file, ingest_folder
from .retraining import RetrainingQueueProcessor
from .retraining_runner import run_once as run_retraining_once


def _resolve_dir(path: Optional[str]) -> Optional[Path]:
    return Path(path).expanduser() if path else None


def dashboard_command(namespace: argparse.Namespace) -> int:
    analytics_dir = _resolve_dir(namespace.analytics_dir)
    dashboard = load_dashboard(analytics_dir)
    if namespace.json:
        sys.stdout.write(json.dumps(dashboard, ensure_ascii=False, indent=2) + "\n")
    else:
        sys.stdout.write(format_dashboard(dashboard) + "\n")
    return 0


def queue_list_command(namespace: argparse.Namespace) -> int:
    analytics_dir = _resolve_dir(namespace.analytics_dir)
    processor = RetrainingQueueProcessor(analytics_dir)
    entries = processor.pending()
    if namespace.json:
        payload = [entry.__dict__ for entry in entries]
        sys.stdout.write(json.dumps(payload, ensure_ascii=False, indent=2) + "\n")
    else:
        if not entries:
            sys.stdout.write("대기 중인 항목이 없습니다.\n")
        for entry in entries:
            sys.stdout.write(
                f"- {entry.meeting_id} (created_at={entry.created_at}, language={entry.language})\n"
            )
    return 0


def queue_claim_command(namespace: argparse.Namespace) -> int:
    analytics_dir = _resolve_dir(namespace.analytics_dir)
    processor = RetrainingQueueProcessor(analytics_dir)
    entry = processor.claim_next()
    if entry is None:
        sys.stdout.write("클레임할 항목이 없습니다.\n")
        return 0

    if namespace.json:
        sys.stdout.write(json.dumps(entry.__dict__, ensure_ascii=False, indent=2) + "\n")
    else:
        sys.stdout.write(
            f"클레임: {entry.meeting_id}\n"
            f"  summary: {entry.summary_path}\n"
            f"  transcript: {entry.transcript_path}\n"
        )
    if namespace.mark_status:
        processor.mark_processed(entry, status=namespace.mark_status)
    return 0


def queue_mark_command(namespace: argparse.Namespace) -> int:
    analytics_dir = _resolve_dir(namespace.analytics_dir)
    processor = RetrainingQueueProcessor(analytics_dir)
    entry = processor.make_entry(
        meeting_id=namespace.meeting_id,
        summary_path=namespace.summary_path or "",
        transcript_path=namespace.transcript_path or "",
        created_at=namespace.created_at or "",
        language=namespace.language or "",
    )
    processor.mark_processed(entry, status=namespace.status)
    return 0


def queue_run_command(namespace: argparse.Namespace) -> int:
    analytics_dir = _resolve_dir(namespace.analytics_dir)

    def handler(entry):
        if namespace.echo:
            sys.stdout.write(f"Processing {entry.meeting_id}\n")
        if namespace.status:
            return namespace.status
        return "completed"

    processed = run_retraining_once(base_dir=analytics_dir, handler=handler)
    if namespace.echo and not processed:
        sys.stdout.write("대기 중인 항목이 없습니다.\n")
    return 0


def ingest_command(namespace: argparse.Namespace) -> int:
    pipeline = MeetingPipeline()
    if not namespace.file and not namespace.input_dir:
        raise SystemExit("--file 또는 --input-dir 중 하나를 지정해야 합니다.")
    if namespace.file:
        ingest_file(Path(namespace.file), Path(namespace.output_dir), pipeline=pipeline)
        return 0

    input_dir = Path(namespace.input_dir)
    output_root = Path(namespace.output_dir)
    for _ in ingest_folder(
        input_dir,
        output_root,
        pattern=namespace.pattern,
        recursive=namespace.recursive,
        pipeline=pipeline,
    ):
        if namespace.echo:
            sys.stdout.write("ingested file\n")
    return 0


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Meeting analytics CLI")
    parser.add_argument("--analytics-dir", help="Path to analytics directory (defaults to env)")

    subparsers = parser.add_subparsers(dest="command", required=True)

    dashboard_parser = subparsers.add_parser("dashboard", help="Show analytics dashboard")
    dashboard_parser.add_argument("--json", action="store_true", help="Output raw JSON")
    dashboard_parser.set_defaults(func=dashboard_command)

    queue_parser = subparsers.add_parser("queue", help="Manage retraining queue")
    queue_sub = queue_parser.add_subparsers(dest="queue_command", required=True)

    queue_list = queue_sub.add_parser("list", help="List pending queue entries")
    queue_list.add_argument("--json", action="store_true", help="Output raw JSON")
    queue_list.set_defaults(func=queue_list_command)

    queue_claim = queue_sub.add_parser("claim", help="Claim next queue entry")
    queue_claim.add_argument("--json", action="store_true", help="Output raw JSON")
    queue_claim.add_argument(
        "--mark-status",
        help="Optional status to mark immediately after claiming",
    )
    queue_claim.set_defaults(func=queue_claim_command)

    queue_mark = queue_sub.add_parser("mark", help="Mark an entry as processed")
    queue_mark.add_argument("meeting_id", help="Meeting identifier to mark")
    queue_mark.add_argument("--status", default="completed", help="Processing status")
    queue_mark.add_argument("--summary-path", help="Summary path for bookkeeping")
    queue_mark.add_argument("--transcript-path", help="Transcript path for bookkeeping")
    queue_mark.add_argument("--created-at", help="Optional created timestamp")
    queue_mark.add_argument("--language", help="Language hint")
    queue_mark.set_defaults(func=queue_mark_command)

    queue_run = queue_sub.add_parser("run", help="Process the next queue entry")
    queue_run.add_argument("--status", help="Override status returned by handler")
    queue_run.add_argument("--echo", action="store_true", help="Print basic progress output")
    queue_run.set_defaults(func=queue_run_command)

    ingest_parser = subparsers.add_parser("ingest", help="Ingest meetings from a directory or file")
    ingest_parser.add_argument("--input-dir", help="Directory to scan", required=False)
    ingest_parser.add_argument("--file", help="Single audio file to process")
    ingest_parser.add_argument("--output-dir", required=True, help="Directory to store outputs")
    ingest_parser.add_argument("--pattern", default="*.wav", help="Glob pattern for files")
    ingest_parser.add_argument("--recursive", action="store_true", help="Scan subdirectories")
    ingest_parser.add_argument("--echo", action="store_true", help="Print progress")
    ingest_parser.set_defaults(func=ingest_command)

    return parser


def main(argv: Optional[list[str]] = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    return args.func(args)


if __name__ == "__main__":
    raise SystemExit(main())
