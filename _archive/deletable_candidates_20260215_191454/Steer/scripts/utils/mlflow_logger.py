"""Lightweight MLflow session helper for the CLI."""

from __future__ import annotations

import contextlib
import os
import socket
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Dict, Iterator, Optional

try:
    import mlflow
except Exception:  # pragma: no cover - optional dependency
    mlflow = None  # type: ignore[assignment]


DEFAULT_TRACKING_DIR = Path(".mlruns")
DEFAULT_TRACKING_URI = f"file:{DEFAULT_TRACKING_DIR.resolve()}"
DEFAULT_EXPERIMENT = "AI-summary"


def _ensure_tracking_dir(uri: str) -> None:
    if not uri.startswith("file:"):
        return
    path = Path(uri[5:])
    path.mkdir(parents=True, exist_ok=True)


def mlflow_available() -> bool:
    """Return True when mlflow package can be imported."""
    return mlflow is not None


@dataclass
class MLflowSession:
    """Convenience wrapper around mlflow.start_run / end_run."""

    run_name: str
    experiment: str = DEFAULT_EXPERIMENT
    tracking_uri: str = DEFAULT_TRACKING_URI
    tags: Dict[str, Any] = field(default_factory=dict)
    _active_run: Any = field(init=False, default=None)

    def start(self) -> None:
        if mlflow is None:
            print("⚠️ MLflow 가 설치되어 있지 않아 로깅을 건너뜁니다.")
            return
        _ensure_tracking_dir(self.tracking_uri)
        mlflow.set_tracking_uri(self.tracking_uri)
        mlflow.set_experiment(self.experiment)
        default_tags = {
            "host": socket.gethostname(),
            "pid": str(os.getpid()),
        }
        final_tags = {**default_tags, **self.tags}
        self._active_run = mlflow.start_run(run_name=self.run_name, tags=final_tags)

    def log_params(self, params: Dict[str, Any]) -> None:
        if mlflow is None or self._active_run is None or not params:
            return
        mlflow.log_params(params)

    def log_metrics(self, metrics: Dict[str, float], step: Optional[int] = None) -> None:
        if mlflow is None or self._active_run is None or not metrics:
            return
        mlflow.log_metrics(metrics, step=step)

    def log_artifact(self, path: Path) -> None:
        if mlflow is None or self._active_run is None:
            return
        try:
            mlflow.log_artifact(str(path))
        except Exception as exc:  # pragma: no cover - best effort
            print(f"⚠️ MLflow artifact 업로드 실패: {exc}")

    def end(self) -> None:
        if mlflow is None or self._active_run is None:
            return
        mlflow.end_run()
        self._active_run = None


@contextlib.contextmanager
def mlflow_session(
    run_name: str,
    *,
    experiment: str = DEFAULT_EXPERIMENT,
    tracking_uri: str = DEFAULT_TRACKING_URI,
    tags: Optional[Dict[str, Any]] = None,
) -> Iterator[MLflowSession]:
    """Context manager used by CLI commands."""

    session = MLflowSession(run_name=run_name, experiment=experiment, tracking_uri=tracking_uri, tags=tags or {})
    session.start()
    try:
        yield session
    finally:
        session.end()

