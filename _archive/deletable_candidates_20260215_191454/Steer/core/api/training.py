"""Queue-based training orchestration helpers.

이 모듈은 UI나 외부 서비스가 학습 작업을 요청할 때 사용할 수 있는
가벼운 헬퍼를 제공합니다. 요청은 JSONLines 파일에 기록되어 이후
백그라운드 워커가 실제 파이프라인을 실행할 때 사용할 수 있습니다.
"""
from __future__ import annotations

import json
from dataclasses import dataclass, asdict
from datetime import datetime
from pathlib import Path
from typing import Any, Dict, Iterable, List, Literal, Optional
from uuid import uuid4

from core.config.paths import DATA_DIR
from core.utils import get_logger

LOGGER = get_logger("api.training")

TRAINING_QUEUE_PATH = DATA_DIR / "training_jobs.jsonl"
TRAINING_STATUS_PATH = DATA_DIR / "training_status.json"

TrainingMode = Literal["global", "smart-folder"]
TrainingStatus = Literal["queued", "running", "failed", "completed"]


@dataclass
class TrainingJob:
    """A serialisable representation of a training request."""

    id: str
    mode: TrainingMode
    payload: Dict[str, Any]
    status: TrainingStatus
    created_at: str
    updated_at: str

    def to_json(self) -> str:
        return json.dumps(asdict(self), ensure_ascii=False)


def _ensure_storage() -> None:
    TRAINING_QUEUE_PATH.parent.mkdir(parents=True, exist_ok=True)
    if not TRAINING_QUEUE_PATH.exists():
        TRAINING_QUEUE_PATH.write_text("", encoding="utf-8")
    if not TRAINING_STATUS_PATH.exists():
        TRAINING_STATUS_PATH.write_text("{}", encoding="utf-8")


def _load_status() -> Dict[str, Any]:
    _ensure_storage()
    try:
        with TRAINING_STATUS_PATH.open("r", encoding="utf-8") as fh:
            return json.load(fh) or {}
    except json.JSONDecodeError:
        LOGGER.warning("training status file corrupted; resetting")
        return {}


def _store_status(data: Dict[str, Any]) -> None:
    TRAINING_STATUS_PATH.write_text(json.dumps(data, ensure_ascii=False, indent=2), encoding="utf-8")


def _stamp() -> str:
    return datetime.utcnow().isoformat(timespec="seconds") + "Z"


def _append_job(job: TrainingJob) -> None:
    with TRAINING_QUEUE_PATH.open("a", encoding="utf-8") as fh:
        fh.write(job.to_json())
        fh.write("\n")


def queue_training_job(mode: TrainingMode, payload: Dict[str, Any]) -> TrainingJob:
    """Queue a training job and return the created record."""

    if mode not in {"global", "smart-folder"}:
        raise ValueError("Invalid training mode")

    timestamp = _stamp()
    job = TrainingJob(
        id=str(uuid4()),
        mode=mode,
        payload=payload,
        status="queued",
        created_at=timestamp,
        updated_at=timestamp,
    )

    _ensure_storage()
    _append_job(job)

    status = _load_status()
    status[job.id] = {
        "mode": job.mode,
        "status": job.status,
        "payload": job.payload,
        "created_at": job.created_at,
        "updated_at": job.updated_at,
    }
    _store_status(status)

    LOGGER.info("Queued training job %s (mode=%s)", job.id, job.mode)
    return job


def list_training_jobs(limit: Optional[int] = None) -> List[TrainingJob]:
    """Return queued jobs (most recent first)."""

    _ensure_storage()
    jobs: List[TrainingJob] = []
    lines: Iterable[str]
    with TRAINING_QUEUE_PATH.open("r", encoding="utf-8") as fh:
        lines = list(filter(None, (line.strip() for line in fh)))

    for raw in reversed(list(lines)):
        try:
            data = json.loads(raw)
        except json.JSONDecodeError:
            continue
        try:
            job = TrainingJob(
                id=data["id"],
                mode=data["mode"],
                payload=data.get("payload", {}),
                status=data.get("status", "queued"),
                created_at=data.get("created_at", ""),
                updated_at=data.get("updated_at", ""),
            )
        except KeyError:
            continue
        jobs.append(job)
        if limit is not None and len(jobs) >= limit:
            break
    return jobs


def update_training_status(job_id: str, *, status: TrainingStatus, payload: Optional[Dict[str, Any]] = None) -> None:
    """Update status metadata for an existing job.

    백그라운드 워커가 진행률을 보고할 때 사용할 수 있는 헬퍼입니다.
    """

    _ensure_storage()
    state = _load_status()
    if job_id not in state:
        state[job_id] = {
            "mode": "unknown",
            "status": status,
            "payload": payload or {},
            "created_at": _stamp(),
            "updated_at": _stamp(),
        }
    else:
        state[job_id]["status"] = status
        if payload is not None:
            state[job_id]["payload"] = payload
        state[job_id]["updated_at"] = _stamp()
    _store_status(state)


def claim_next_job() -> Optional[TrainingJob]:
    """Mark the next queued job as running and return it."""

    state = _load_status()
    claimed_id: Optional[str] = None
    for job_id, info in state.items():
        if info.get("status") == "queued":
            claimed_id = job_id
            break
    if claimed_id is None:
        return None

    info = state[claimed_id]
    info["status"] = "running"
    info["updated_at"] = _stamp()
    _store_status(state)

    return TrainingJob(
        id=claimed_id,
        mode=info.get("mode", "global"),
        payload=info.get("payload", {}),
        status="running",
        created_at=info.get("created_at", _stamp()),
        updated_at=info.get("updated_at", _stamp()),
    )


def get_job_status(job_id: str) -> Optional[Dict[str, Any]]:
    state = _load_status()
    return state.get(job_id)


__all__ = [
    "TRAINING_QUEUE_PATH",
    "TRAINING_STATUS_PATH",
    "TrainingJob",
    "TrainingMode",
    "queue_training_job",
    "list_training_jobs",
    "update_training_status",
    "claim_next_job",
    "get_job_status",
]

