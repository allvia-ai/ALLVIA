"""Entry point used by the desktop UI to execute the meeting agent pipeline.

This script is intentionally conservative: it only touches files within the
selected smart-folder scope and returns structured JSON output for the UI.
"""
from __future__ import annotations

import argparse
import json
import os
import sys
import time
from dataclasses import asdict
from datetime import datetime
from pathlib import Path
from typing import Any, Dict, List, Optional

REPO_ROOT = Path(__file__).resolve().parents[1]
if str(REPO_ROOT) not in sys.path:
    sys.path.insert(0, str(REPO_ROOT))

from scripts.audit_log import record_event
from core.agents.meeting import MeetingJobConfig, MeetingPipeline, MeetingSummary

ALLOWED_AUDIO_EXTS = {".wav", ".mp3", ".m4a", ".aac", ".ogg", ".flac"}
ALLOWED_TRANSCRIPT_EXTS = {".txt", ".md"}
MEETING_AGENT = "meeting"


def _parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Run meeting agent pipeline")
    parser.add_argument("--query", default="", help="User query or instruction")
    parser.add_argument("--folder-path", required=True, help="Smart folder absolute path")
    parser.add_argument("--folder-label", default="", help="Display label for the folder")
    parser.add_argument("--folder-scope", default="", help="Scope identifier (e.g. local)")
    parser.add_argument("--policy-path", default="", help="Optional policy file path")
    parser.add_argument("--audio", "--audio-path", dest="audio_path", default="", help="Optional audio/transcript path")
    parser.add_argument("--output-dir", dest="output_dir", default="", help="Optional output directory")
    parser.add_argument("--output-json", action="store_true", help="Print structured JSON")
    parser.add_argument("--debug", action="store_true", help="Enable verbose debug logging")
    return parser.parse_args()


def _emit(payload: Dict[str, Any], *, exit_code: int = 0, ensure_json: bool = True) -> None:
    if ensure_json and "ok" not in payload:
        payload = {"ok": False, "error": "invalid payload", "data": None}
    json_payload = json.dumps(payload, ensure_ascii=False)
    sys.stdout.write(json_payload)
    if not json_payload.endswith("\n"):
        sys.stdout.write("\n")
    sys.stdout.flush()
    raise SystemExit(exit_code)


def _resolve_folder(path_str: str) -> Path:
    target = Path(path_str).expanduser().resolve()
    if not target.exists() or not target.is_dir():
        raise ValueError(f"folder not accessible: {target}")
    return target


def _load_policy_engine(policy_path: str) -> PolicyEngine:
    try:
        from core.data_pipeline.policies.engine import PolicyEngine
    except ModuleNotFoundError as exc:
        raise RuntimeError(
            "Smart folder policy support requires extra dependencies. "
            "Run inside the project env (e.g. `conda run -n ai-summary python3 ...`) "
            "or install missing packages (e.g. `pip install jsonschema`)."
        ) from exc

    raw = (policy_path or "").strip()
    if not raw or raw.lower() == "none":
        return PolicyEngine.empty()
    path = Path(raw).expanduser()
    return PolicyEngine.from_file(path)


def _discover_meeting_source(folder: Path, *, debug: bool = False, policy_engine: PolicyEngine | None = None) -> Path:
    candidates: List[Path] = []
    for entry in folder.rglob("*"):
        if not entry.is_file():
            continue
        try:
            entry.relative_to(folder)
        except ValueError:
            continue
        if entry.suffix.lower() in ALLOWED_AUDIO_EXTS | ALLOWED_TRANSCRIPT_EXTS:
            if policy_engine and policy_engine.has_policies:
                if not policy_engine.allows(entry, agent=MEETING_AGENT, include_manual=True):
                    continue
            candidates.append(entry)
    if debug:
        sys.stderr.write(f"[bridge] discovered {len(candidates)} candidate files\n")
    if not candidates:
        raise FileNotFoundError("meeting audio/transcript not found in folder")
    candidates.sort(key=lambda p: p.stat().st_mtime, reverse=True)
    return candidates[0]


def _make_output_dir(folder: Path) -> Path:
    timestamp = datetime.utcnow().strftime("%Y%m%d-%H%M%S")
    destination = folder / ".ai_agent" / "meetings" / timestamp
    destination.mkdir(parents=True, exist_ok=True)
    return destination


def _resolve_audio_path(raw: str) -> Path:
    path = Path(raw).expanduser().resolve()
    if not path.exists() or not path.is_file():
        raise FileNotFoundError(f"audio/transcript not accessible: {path}")
    if path.suffix.lower() not in ALLOWED_AUDIO_EXTS | ALLOWED_TRANSCRIPT_EXTS:
        raise ValueError(f"unsupported meeting file type: {path.suffix}")
    return path


def _resolve_output_dir(raw: str, *, folder: Path) -> Path:
    if raw:
        out = Path(raw).expanduser().resolve()
        out.mkdir(parents=True, exist_ok=True)
        return out
    return _make_output_dir(folder)


def _summary_payload(
    summary: Any,
    *,
    folder_label: str,
    folder_path: Path,
    source_path: Path,
    query: str,
    output_dir: Path,
    duration_seconds: float,
) -> Dict[str, Any]:
    metadata_path = output_dir / "metadata.json"
    segments_path = output_dir / "segments.json"
    return {
        "ok": True,
        "data": {
            "summary": summary.raw_summary,
            "highlights": summary.highlights,
            "actions": summary.action_items,
            "decisions": summary.decisions,
            "folder": {
                "label": folder_label or folder_path.name,
                "path": str(folder_path),
            },
            "source": str(source_path),
            "transcript": str(summary.transcript_path),
            "metadata": str(metadata_path) if metadata_path.exists() else None,
            "segments": str(segments_path) if segments_path.exists() else None,
            "query": query,
            "duration_seconds": duration_seconds,
        },
    }


def main() -> None:
    args = _parse_args()
    debug = bool(args.debug)

    policy_engine = _load_policy_engine(args.policy_path)

    try:
        folder_path = _resolve_folder(args.folder_path)
    except Exception as exc:  # noqa: BLE001
        record_event(
            agent="meeting",
            event="run",
            status="error",
            details={
                "reason": str(exc),
                "folder_path": args.folder_path,
            },
        )
        _emit({"ok": False, "error": str(exc), "data": None}, exit_code=1)

    if policy_engine and policy_engine.has_policies:
        # Folder itself must be allowed for meeting agent
        if not policy_engine.allows(folder_path, agent=MEETING_AGENT, include_manual=True):
            _emit(
                {
                    "ok": False,
                    "error": f"policy denied access to folder: {folder_path}",
                    "data": None,
                },
                exit_code=1,
            )

    meeting_file: Optional[Path] = None
    raw_audio = (args.audio_path or "").strip()
    if raw_audio:
        try:
            meeting_file = _resolve_audio_path(raw_audio)
            meeting_file.relative_to(folder_path)
        except Exception as exc:  # noqa: BLE001
            record_event(
                agent="meeting",
                event="resolve_audio",
                status="error",
                details={
                    "reason": str(exc),
                    "audio_path": raw_audio,
                    "folder_path": str(folder_path),
                },
            )
            _emit({"ok": False, "error": str(exc), "data": None}, exit_code=2)
    else:
        try:
            meeting_file = _discover_meeting_source(folder_path, debug=debug, policy_engine=policy_engine)
        except Exception as exc:  # noqa: BLE001
            record_event(
                agent="meeting",
                event="discover_source",
                status="error",
                details={
                    "reason": str(exc),
                    "folder_path": str(folder_path),
                },
            )
            _emit({"ok": False, "error": str(exc), "data": None}, exit_code=2)

    if meeting_file is None:
        _emit({"ok": False, "error": "meeting file not found", "data": None}, exit_code=2)

    if policy_engine and policy_engine.has_policies:
        if not policy_engine.allows(meeting_file, agent=MEETING_AGENT, include_manual=True):
            _emit({"ok": False, "error": "policy denied meeting file", "data": None}, exit_code=2)

    try:
        output_dir = _resolve_output_dir((args.output_dir or "").strip(), folder=folder_path)
    except Exception as exc:  # noqa: BLE001
        record_event(
            agent="meeting",
            event="prepare_output",
            status="error",
            details={
                "reason": str(exc),
                "folder_path": str(folder_path),
            },
        )
        _emit({"ok": False, "error": f"failed to prepare output dir: {exc}", "data": None}, exit_code=3)

    job = MeetingJobConfig(
        audio_path=meeting_file,
        output_dir=output_dir,
        context_dirs=[folder_path],
        policy_tag=args.policy_path or None,
    )

    mask_pii = False
    try:
        if policy_engine and policy_engine.has_policies:
            mask_pii = policy_engine.pii_mask_enabled_for_path(meeting_file, agent=MEETING_AGENT)
    except Exception:
        mask_pii = False

    pipeline = MeetingPipeline(mask_pii=mask_pii)

    start_ts = time.perf_counter()
    try:
        summary = pipeline.run(job)
    except Exception as exc:  # noqa: BLE001
        record_event(
            agent="meeting",
            event="run",
            status="error",
            details={
                "reason": str(exc),
                "meeting_file": str(meeting_file),
                "folder_path": str(folder_path),
            },
        )
        _emit(
            {
                "ok": False,
                "error": f"meeting pipeline failed: {exc}",
                "data": None,
            },
            exit_code=4,
        )
    elapsed = time.perf_counter() - start_ts

    result = _summary_payload(
        summary,
        folder_label=args.folder_label,
        folder_path=folder_path,
        source_path=meeting_file,
        query=args.query,
        output_dir=output_dir,
        duration_seconds=elapsed,
    )

    record_event(
        agent="meeting",
        event="run",
        status="success",
        details={
            "meeting_file": str(meeting_file),
            "folder_path": str(folder_path),
            "elapsed_seconds": elapsed,
            "highlight_count": len(summary.highlights),
            "action_count": len(summary.action_items),
        },
    )

    _emit(result, exit_code=0)


if __name__ == "__main__":
    main()
