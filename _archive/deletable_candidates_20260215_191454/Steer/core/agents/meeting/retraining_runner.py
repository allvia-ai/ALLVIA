"""Runnable helpers for processing the meeting retraining queue."""
from __future__ import annotations

import argparse
import logging
import os
import subprocess
import sys
import time
from datetime import datetime
import json
from pathlib import Path
from typing import Callable, Optional

from .retraining import QueueEntry, process_next
from .retraining_taskgraph import RetrainingTaskGraphRunner

LOGGER = logging.getLogger(__name__)


def default_handler(entry: QueueEntry) -> str:
    """Baseline handler that validates artefacts and emits telemetry."""

    summary_path = Path(entry.summary_path)
    transcript_path = Path(entry.transcript_path)

    missing = []
    if summary_path and not summary_path.exists():
        missing.append(f"summary:{summary_path}")
    if transcript_path and not transcript_path.exists():
        missing.append(f"transcript:{transcript_path}")

    if missing:
        LOGGER.warning("retraining artefact missing for %s -> %s", entry.meeting_id, ", ".join(missing))
        return "missing"

    if summary_path:
        try:
            text = summary_path.read_text(encoding="utf-8")
            LOGGER.info("summary size for %s: %s bytes", entry.meeting_id, len(text))
        except Exception as exc:  # pragma: no cover - defensive
            LOGGER.warning("failed to inspect summary for %s: %s", entry.meeting_id, exc)
            return "error"

    LOGGER.info(
        "retrieval queue entry validated: meeting_id=%s summary=%s transcript=%s",
        entry.meeting_id,
        entry.summary_path,
        entry.transcript_path,
    )
    return "validated"


def _detect_dataset_root(entry: QueueEntry, override: Optional[Path]) -> Path:
    if override:
        return override

    summary_path = Path(entry.summary_path) if entry.summary_path else None
    if summary_path and summary_path.exists():
        return summary_path.parent.parent if summary_path.parent.parent.exists() else summary_path.parent

    transcript_path = Path(entry.transcript_path) if entry.transcript_path else None
    if transcript_path and transcript_path.exists():
        return transcript_path.parent.parent if transcript_path.parent.parent.exists() else transcript_path.parent

    raise ValueError("Unable to infer dataset root; please supply --dataset-root")


def _build_training_command(
    *,
    dataset_root: Path,
    output_dir: Path,
    base_model: str,
    learning_rate: float,
    batch_size: int,
    epochs: float,
    max_source_length: int,
    max_target_length: int,
    eval_split: float,
) -> list[str]:
    cmd = [
        sys.executable,
        str(Path("scripts") / "train_meeting_summariser.py"),
        "--input-dir",
        str(dataset_root.resolve()),
        "--model-name",
        base_model,
        "--output-dir",
        str(output_dir.resolve()),
        "--learning-rate",
        str(learning_rate),
        "--batch-size",
        str(batch_size),
        "--num-epochs",
        str(epochs),
        "--max-source-length",
        str(max_source_length),
        "--max-target-length",
        str(max_target_length),
        "--eval-split",
        str(eval_split),
    ]
    return cmd


def _extract_training_metrics(run_dir: Path) -> dict:
    trainer_state = run_dir / "trainer_state.json"
    if not trainer_state.exists():
        return {}

    try:
        state = json.loads(trainer_state.read_text(encoding="utf-8"))
    except json.JSONDecodeError:
        return {}

    log_history = state.get("log_history", []) or []
    metrics: dict = {}
    for entry in reversed(log_history):
        if isinstance(entry, dict) and "eval_loss" in entry:
            metrics = entry
            break
    return metrics


def finetune_handler(
    entry: QueueEntry,
    *,
    dataset_root: Optional[Path] = None,
    output_root: Optional[Path] = None,
    base_model: Optional[str] = None,
    learning_rate: float = 5e-5,
    batch_size: int = 2,
    epochs: float = 3.0,
    max_source_length: int = 512,
    max_target_length: int = 128,
    eval_split: float = 0.1,
) -> str:
    """Fine-tune the summariser using the available training dataset."""

    dataset_root = _detect_dataset_root(entry, dataset_root)

    output_root = output_root or Path(os.getenv("MEETING_RETRAIN_OUTPUT_DIR", "artifacts/retraining"))
    output_root = output_root.resolve()
    output_root.mkdir(parents=True, exist_ok=True)

    timestamp = datetime.now().strftime("%Y%m%d-%H%M%S")
    run_dir = output_root / f"checkpoint-{entry.meeting_id}-{timestamp}"
    base_model = base_model or os.getenv("MEETING_BASE_MODEL") or os.getenv("MEETING_SUMMARY_MODEL", "gogamza/kobart-base-v2")

    cmd = _build_training_command(
        dataset_root=dataset_root,
        output_dir=run_dir,
        base_model=base_model,
        learning_rate=learning_rate,
        batch_size=batch_size,
        epochs=epochs,
        max_source_length=max_source_length,
        max_target_length=max_target_length,
        eval_split=eval_split,
    )

    LOGGER.info("Starting retraining run for %s using dataset %s", entry.meeting_id, dataset_root)
    LOGGER.debug("Executing command: %s", " ".join(cmd))

    completed = subprocess.run(cmd, check=False)
    if completed.returncode != 0:
        LOGGER.error("Retraining command failed with exit code %s", completed.returncode)
        raise RuntimeError(f"Retraining failed for {entry.meeting_id}")

    LOGGER.info("Retraining completed for %s; artefacts stored in %s", entry.meeting_id, run_dir)
    metrics = _extract_training_metrics(run_dir)
    summary_path = run_dir / "run_summary.json"
    summary_payload = {
        "meeting_id": entry.meeting_id,
        "dataset_root": str(dataset_root),
        "base_model": base_model,
        "learning_rate": learning_rate,
        "batch_size": batch_size,
        "epochs": epochs,
        "max_source_length": max_source_length,
        "max_target_length": max_target_length,
        "eval_split": eval_split,
        "metrics": metrics,
        "completed_at": datetime.now().isoformat(),
    }
    summary_path.write_text(json.dumps(summary_payload, ensure_ascii=False, indent=2), encoding="utf-8")

    LOGGER.info("Retraining completed for %s; metrics: %s", entry.meeting_id, metrics or "(none)")
    return "completed"


def run_once(
    *,
    base_dir: Optional[Path] = None,
    handler: Callable[[QueueEntry], str] = default_handler,
) -> bool:
    """Process at most one queue entry using the provided handler.

    Returns True if an entry was claimed and processed; False otherwise.
    """

    runner = RetrainingTaskGraphRunner(handler=handler, base_dir=base_dir)
    return runner.run_once()


def run_many(
    *,
    base_dir: Optional[Path] = None,
    handler: Callable[[QueueEntry], str] = default_handler,
    max_runs: Optional[int] = None,
) -> int:
    """Process multiple queue entries until exhausted or reaching limit."""

    runner = RetrainingTaskGraphRunner(handler=handler, base_dir=base_dir)
    return runner.run_many(max_runs=max_runs)


def watch_queue(
    *,
    base_dir: Optional[Path] = None,
    handler: Callable[[QueueEntry], str] = default_handler,
    interval_seconds: int = 60,
) -> None:
    LOGGER.info("Watching retraining queue (interval=%ss)", interval_seconds)
    try:
        while True:
            processed = run_many(base_dir=base_dir, handler=handler, max_runs=None)
            if processed == 0:
                time.sleep(max(1, interval_seconds))
            else:
                LOGGER.info("Processed %s entries; continuing watch", processed)
    except KeyboardInterrupt:  # pragma: no cover - interactive use
        LOGGER.info("Stopping queue watcher")


def _build_argparser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--mode", choices=["validate", "finetune"], default="validate", help="Handler to use when processing the queue")
    parser.add_argument("--analytics-dir", type=Path, default=None, help="Override analytics directory (defaults to MEETING_ANALYTICS_DIR)")
    parser.add_argument("--dataset-root", type=Path, default=None, help="Root directory containing transcript/summary pairs")
    parser.add_argument("--output-root", type=Path, default=None, help="Directory to place fine-tuned checkpoints")
    parser.add_argument("--base-model", type=str, default=None, help="Model name or path to fine-tune from")
    parser.add_argument("--learning-rate", type=float, default=5e-5)
    parser.add_argument("--batch-size", type=int, default=2)
    parser.add_argument("--epochs", type=float, default=3.0)
    parser.add_argument("--max-source-length", type=int, default=512)
    parser.add_argument("--max-target-length", type=int, default=128)
    parser.add_argument("--eval-split", type=float, default=0.1)
    parser.add_argument("--max-runs", type=int, default=1, help="Number of queue entries to process (0 for all)")
    parser.add_argument("--watch", action="store_true", help="Keep processing queue at the specified interval")
    parser.add_argument("--watch-interval", type=int, default=300, help="Polling interval in seconds for watch mode")
    return parser


def main() -> None:
    logging.basicConfig(level=logging.INFO, format="%(asctime)s | %(levelname)s | %(name)s | %(message)s")

    args = _build_argparser().parse_args()

    if args.mode == "validate":
        handler = default_handler
    else:
        handler = lambda entry: finetune_handler(
            entry,
            dataset_root=args.dataset_root,
            output_root=args.output_root,
            base_model=args.base_model,
            learning_rate=args.learning_rate,
            batch_size=args.batch_size,
            epochs=args.epochs,
            max_source_length=args.max_source_length,
            max_target_length=args.max_target_length,
            eval_split=args.eval_split,
        )

    if args.mode == "validate":
        processed = run_many(
            base_dir=args.analytics_dir,
            handler=handler,
            max_runs=args.max_runs if args.max_runs != 0 else None,
        )
        if processed == 0:
            LOGGER.info("No entries available in the retraining queue")
        else:
            LOGGER.info("Processed %s queue entries", processed)
        return

    if args.watch:
        watch_queue(
            base_dir=args.analytics_dir,
            handler=handler,
            interval_seconds=max(1, args.watch_interval),
        )
        return

    max_runs = args.max_runs if args.max_runs != 0 else None
    processed = run_many(base_dir=args.analytics_dir, handler=handler, max_runs=max_runs)
    if processed == 0:
        LOGGER.info("No entries available in the retraining queue")
    else:
        LOGGER.info("Retraining completed for %s entries", processed)


if __name__ == "__main__":  # pragma: no cover - CLI entry point
    main()
