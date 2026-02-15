"""CLI bridge to run the photo agent in "organize" mode (safe by default).

This is intended as a UI/terminal-friendly wrapper similar to `scripts/run_meeting_agent.py`.
It supports dry-run planning first and only moves files when `--apply` is provided.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Dict, List, Optional

REPO_ROOT = Path(__file__).resolve().parents[1]
if str(REPO_ROOT) not in sys.path:
    sys.path.insert(0, str(REPO_ROOT))

from core.agents.photo.models import PhotoJobConfig
from core.agents.photo.pipeline import PhotoPipeline


PHOTO_AGENT = "photo"


def _parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Organize photos within a smart-folder scope")
    parser.add_argument("--roots", nargs="+", required=True, help="Photo folder roots to scan (one or more)")
    parser.add_argument("--policy-path", default=str(REPO_ROOT / "core" / "config" / "smart_folders.json"))
    parser.add_argument("--dest-root", default="", help="Destination root (default: first root)")
    parser.add_argument("--strategy", default="month", choices=("month", "year"), help="Folder grouping strategy")
    parser.add_argument("--dedupe", action="store_true", help="Detect duplicates by sha256 (slower)")
    parser.add_argument("--apply", action="store_true", help="Actually move files (default: dry-run)")
    parser.add_argument("--output-dir", default="", help="Output directory for logs (default: <root>/.ai_agent/photos/<ts>)")
    parser.add_argument("--json", dest="output_json", action="store_true", help="Print JSON payload to stdout")
    return parser.parse_args()


def _load_policy_engine(policy_path: str):
    from core.data_pipeline.policies.engine import PolicyEngine

    raw = (policy_path or "").strip()
    if not raw or raw.lower() == "none":
        return PolicyEngine.empty()
    return PolicyEngine.from_file(Path(raw).expanduser())


def _resolve_roots(values: List[str]) -> List[Path]:
    roots: List[Path] = []
    for raw in values:
        p = Path(raw).expanduser().resolve()
        if p.exists() and p.is_dir():
            roots.append(p)
    return roots


def _default_output_dir(root: Path) -> Path:
    stamp = datetime.now(timezone.utc).strftime("%Y%m%d-%H%M%S")
    return root / ".ai_agent" / "photos" / stamp


def _emit(payload: Dict[str, Any], *, exit_code: int = 0) -> None:
    sys.stdout.write(json.dumps(payload, ensure_ascii=False))
    sys.stdout.write("\n")
    sys.stdout.flush()
    raise SystemExit(exit_code)


def main() -> None:
    args = _parse_args()
    roots = _resolve_roots(args.roots)
    if not roots:
        raise SystemExit("no valid roots provided")

    policy_engine = _load_policy_engine(args.policy_path)
    if policy_engine and getattr(policy_engine, "has_policies", False):
        for root in roots:
            if not policy_engine.allows(root, agent=PHOTO_AGENT, include_manual=True):
                raise SystemExit(f"policy denied root: {root}")

    dest_root = Path(args.dest_root).expanduser().resolve() if args.dest_root else roots[0]
    if not args.dest_root:
        dest_root = roots[0] / "organized"
    # Safety: keep destination inside one of the roots unless explicitly overridden.
    allowed_external = (str(Path(dest_root)).strip() and (os.getenv("PHOTO_ALLOW_EXTERNAL_DEST", "0") == "1"))
    if not allowed_external:
        if not any(str(dest_root).startswith(str(root)) for root in roots):
            raise SystemExit(f"dest_root must be within one of the roots: {dest_root}")

    output_dir = Path(args.output_dir).expanduser().resolve() if args.output_dir else _default_output_dir(roots[0])
    warnings: List[str] = []
    try:
        output_dir.mkdir(parents=True, exist_ok=True)
    except PermissionError:
        fallback = REPO_ROOT / "data" / "photo_outputs" / datetime.now(timezone.utc).strftime("%Y%m%d-%H%M%S")
        fallback.mkdir(parents=True, exist_ok=True)
        warnings.append(f"output_dir_not_writable: {output_dir} -> {fallback}")
        output_dir = fallback

    job = PhotoJobConfig(
        roots=roots,
        output_dir=output_dir,
        policy_engine=policy_engine,
        policy_agent=PHOTO_AGENT,
        organize=True,
        dry_run=not bool(args.apply),
        dest_root=dest_root,
        organize_strategy=args.strategy,
        dedupe=bool(args.dedupe),
    )

    pipeline = PhotoPipeline()
    recommendation = pipeline.run(job)

    payload = {
        "ok": True,
        "data": {
            "report_path": str(recommendation.report_path),
            "output_dir": str(output_dir),
            "dry_run": not bool(args.apply),
        },
    }
    if warnings:
        payload["warnings"] = warnings
    if args.output_json:
        _emit(payload, exit_code=0)
    sys.stdout.write(f"photo report: {recommendation.report_path}\n")


if __name__ == "__main__":
    main()
