from __future__ import annotations

import contextlib
from pathlib import Path
from typing import Optional

import click

from core.monitor import ResourceLogger
from scripts.utils.mlflow_logger import DEFAULT_EXPERIMENT, DEFAULT_TRACKING_URI, mlflow_session


@contextlib.contextmanager
def command_session(ctx: click.Context, run_name: str):
    """Attach MLflow + resource logger lifecycle to each CLI command."""

    settings = ctx.ensure_object(dict)
    use_mlflow: bool = settings.get("use_mlflow", True)
    tracking_uri: str = settings.get("mlflow_uri", DEFAULT_TRACKING_URI)
    experiment: str = settings.get("mlflow_experiment", DEFAULT_EXPERIMENT)
    resource_path: Optional[Path] = settings.get("resource_log_path")
    resource_interval: float = settings.get("resource_interval", 30.0)

    if use_mlflow:
        mlflow_cm = mlflow_session(
            run_name,
            experiment=experiment,
            tracking_uri=tracking_uri,
            tags={"command": run_name},
        )
    else:
        mlflow_cm = contextlib.nullcontext(None)

    resource_logger = None
    if resource_path:
        resource_logger = ResourceLogger(Path(resource_path), interval=resource_interval)
        resource_logger.start(context=run_name)

    try:
        with mlflow_cm as session:
            yield session
    finally:
        if resource_logger:
            resource_logger.stop()


__all__ = ["command_session"]

