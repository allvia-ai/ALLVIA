"""TaskGraph-based runner for meeting retraining queue."""
from __future__ import annotations

import json
import logging
import os
import subprocess
import sys
from datetime import datetime
from pathlib import Path
from typing import Callable, Optional

from core.agents.taskgraph import TaskContext, TaskGraph

from .retraining import QueueEntry, process_next

LOGGER = logging.getLogger(__name__)


class RetrainingTaskGraphRunner:
    def __init__(self, *, handler: Optional[Callable[[QueueEntry], str]] = None, base_dir: Optional[Path] = None) -> None:
        self.base_dir = base_dir
        self.handler = handler or self._default_handler

    def run_once(self) -> bool:
        return process_next(self._handle_entry, base_dir=self.base_dir)

    def run_many(self, *, max_runs: Optional[int] = None) -> int:
        count = 0
        while max_runs is None or count < max_runs:
            if not self.run_once():
                break
            count += 1
        return count

    def _handle_entry(self, entry: QueueEntry) -> str:
        context = TaskContext(pipeline=self, job=entry)
        graph = TaskGraph("meeting_retraining")
        graph.add_stage("validate", self._stage_validate)
        graph.add_stage("prepare", self._stage_prepare, dependencies=("validate",))
        graph.add_stage("train", self._stage_train, dependencies=("prepare",))
        graph.add_stage("finalise", self._stage_finalise, dependencies=("train",))

        graph.run(context)
        for event in context.stage_status():
            LOGGER.info(
                "retraining stage: %s status=%s", event.get("stage"), event.get("status")
            )
        return context.get("status", "completed")

    def _default_handler(self, entry: QueueEntry) -> str:
        return "completed"

    def _stage_validate(self, context: TaskContext) -> None:
        entry: QueueEntry = context.job
        summary_path = Path(entry.summary_path) if entry.summary_path else None
        transcript_path = Path(entry.transcript_path) if entry.transcript_path else None
        missing = []
        if summary_path and not summary_path.exists():
            missing.append(f"summary:{summary_path}")
        if transcript_path and not transcript_path.exists():
            missing.append(f"transcript:{transcript_path}")
        if missing:
            LOGGER.warning("retraining artefact missing for %s -> %s", entry.meeting_id, ", ".join(missing))
            context.set("status", "missing")
            raise RuntimeError("missing artefacts")
        context.set("summary_path", summary_path)
        context.set("transcript_path", transcript_path)

    def _stage_prepare(self, context: TaskContext) -> None:
        entry: QueueEntry = context.job
        dataset_root = self._detect_dataset_root(entry, context.extras.get("dataset_root"))
        output_root = context.extras.get("output_root") or Path(os.getenv("MEETING_RETRAIN_OUTPUT_DIR", "artifacts/retraining")).resolve()
        output_root.mkdir(parents=True, exist_ok=True)
        context.set("dataset_root", dataset_root)
        context.set("output_root", output_root)

    def _stage_train(self, context: TaskContext) -> None:
        entry: QueueEntry = context.job
        dataset_root: Path = context.get("dataset_root")
        output_root: Path = context.get("output_root")

        timestamp = datetime.now().strftime("%Y%m%d-%H%M%S")
        run_dir = output_root / f"checkpoint-{entry.meeting_id}-{timestamp}"
        base_model = context.extras.get("base_model") or os.getenv("MEETING_BASE_MODEL") or os.getenv("MEETING_SUMMARY_MODEL", "gogamza/kobart-base-v2")

        cmd = self._build_training_command(
            dataset_root=dataset_root,
            output_dir=run_dir,
            base_model=base_model,
            learning_rate=float(context.extras.get("learning_rate", 5e-5)),
            batch_size=int(context.extras.get("batch_size", 2)),
            epochs=float(context.extras.get("epochs", 3.0)),
            max_source_length=int(context.extras.get("max_source_length", 512)),
            max_target_length=int(context.extras.get("max_target_length", 128)),
            eval_split=float(context.extras.get("eval_split", 0.1)),
        )

        LOGGER.info("Starting retraining run for %s using dataset %s", entry.meeting_id, dataset_root)
        LOGGER.debug("Executing command: %s", " ".join(cmd))

        completed = subprocess.run(cmd, check=False)
        if completed.returncode != 0:
            LOGGER.error("Retraining command failed with exit code %s", completed.returncode)
            context.set("status", "error")
            raise RuntimeError(f"Retraining failed for {entry.meeting_id}")

        LOGGER.info("Retraining completed for %s; artefacts stored in %s", entry.meeting_id, run_dir)
        context.set("run_dir", run_dir)
        context.set("base_model", base_model)
        context.set("status", "completed")

    def _stage_finalise(self, context: TaskContext) -> None:
        entry: QueueEntry = context.job
        run_dir: Path = context.get("run_dir")
        dataset_root: Path = context.get("dataset_root")
        base_model: str = context.get("base_model")
        metrics = self._extract_training_metrics(run_dir)
        summary_path = run_dir / "run_summary.json"
        summary_payload = {
            "meeting_id": entry.meeting_id,
            "dataset_root": str(dataset_root),
            "base_model": base_model,
            "metrics": metrics,
            "completed_at": datetime.now().isoformat(),
        }
        summary_path.write_text(json.dumps(summary_payload, ensure_ascii=False, indent=2), encoding="utf-8")
        context.set("summary_payload", summary_payload)

    def _detect_dataset_root(self, entry: QueueEntry, override: Optional[Path]) -> Path:
        if override:
            return override
        summary_path = Path(entry.summary_path) if entry.summary_path else None
        if summary_path and summary_path.exists():
            return summary_path.parent.parent if summary_path.parent.parent.exists() else summary_path.parent
        transcript_path = Path(entry.transcript_path) if entry.transcript_path else None
        if transcript_path and transcript_path.exists():
            return transcript_path.parent.parent if transcript_path.parent.parent.exists() else transcript_path.parent
        raise ValueError("Unable to infer dataset root; please supply dataset_root explicitly")

    def _build_training_command(
        self,
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

    @staticmethod
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
