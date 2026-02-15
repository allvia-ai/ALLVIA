"""Training worker that consumes queued training jobs and executes pipelines."""
from __future__ import annotations

import argparse
import subprocess
import sys
import time
from pathlib import Path
from typing import List, Optional

ROOT = Path(__file__).resolve().parents[2]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from core.api.training import claim_next_job, update_training_status, TrainingJob
from core.config.paths import CACHE_DIR, CORPUS_PATH, TOPIC_MODEL_PATH
from core.utils import get_logger

LOGGER = get_logger("pipeline.train_worker")
INFOPILOT_SCRIPT = ROOT / "scripts" / "pipeline" / "infopilot.py"


def _build_pipeline_command(job: TrainingJob) -> List[str]:
    base = [
        sys.executable,
        str(INFOPILOT_SCRIPT),
        "pipeline",
        "--corpus",
        str(CORPUS_PATH),
        "--model",
        str(TOPIC_MODEL_PATH),
        "--cache",
        str(CACHE_DIR),
    ]

    payload = job.payload or {}
    if job.mode == "global":
        roots = payload.get("roots") or []
        for root in roots:
            base.extend(["--root", root])
        exclude = payload.get("exclude") or []
        if exclude:
            LOGGER.info("Exclude paths are not yet supported directly. Ignoring: %s", ", ".join(exclude))
    elif job.mode == "smart-folder":
        folder_path = payload.get("path")
        if folder_path:
            base.extend(["--root", folder_path])
        else:
            LOGGER.warning("Smart-folder job %s missing path", job.id)
    return base


def _run_command(cmd: List[str]) -> subprocess.CompletedProcess[bytes]:
    LOGGER.info("Running training command: %s", " ".join(cmd))
    return subprocess.run(cmd, cwd=ROOT, capture_output=True, text=True)


def _process_job(job: TrainingJob) -> None:
    update_training_status(job.id, status="running")
    if not INFOPILOT_SCRIPT.exists():
        update_training_status(job.id, status="failed", payload={"error": "infopilot.py missing"})
        return

    cmd = _build_pipeline_command(job)
    result = _run_command(cmd)
    payload = {
        "stdout": result.stdout[-4000:],
        "stderr": result.stderr[-4000:],
    }
    if result.returncode != 0:
        payload["error"] = f"pipeline exited with {result.returncode}"
        update_training_status(job.id, status="failed", payload=payload)
    else:
        payload["message"] = "pipeline completed"
        update_training_status(job.id, status="completed", payload=payload)


def worker_loop(poll_seconds: float, once: bool) -> None:
    while True:
        job = claim_next_job()
        if not job:
            if once:
                LOGGER.info("No queued jobs. Exiting.")
                return
            time.sleep(poll_seconds)
            continue
        LOGGER.info("Processing job %s (mode=%s)", job.id, job.mode)
        try:
            _process_job(job)
        except Exception as exc:
            LOGGER.exception("Training job %s failed", job.id)
            update_training_status(job.id, status="failed", payload={"error": str(exc)})
        if once:
            return


def parse_args(argv: Optional[List[str]] = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Training worker for InfoPilot")
    parser.add_argument("--poll", type=float, default=30.0, help="Polling interval when watching (seconds)")
    parser.add_argument("--once", action="store_true", help="Process at most one job then exit")
    return parser.parse_args(argv)


def main(argv: Optional[List[str]] = None) -> int:
    args = parse_args(argv)
    poll = max(5.0, float(args.poll or 30.0))
    worker_loop(poll_seconds=poll, once=args.once)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
