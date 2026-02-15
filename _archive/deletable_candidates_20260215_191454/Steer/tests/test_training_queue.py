from pathlib import Path
import json
import os

import pytest

from core.api import training


def test_queue_training_job_creates_record(tmp_path, monkeypatch):
    data_dir = tmp_path / "data"
    data_dir.mkdir()
    monkeypatch.setattr(training, "TRAINING_QUEUE_PATH", data_dir / "training_jobs.jsonl", raising=False)
    monkeypatch.setattr(training, "TRAINING_STATUS_PATH", data_dir / "training_status.json", raising=False)

    job = training.queue_training_job("global", {"roots": ["/tmp"]})
    assert job.mode == "global"
    queue_contents = (data_dir / "training_jobs.jsonl").read_text(encoding="utf-8").strip().splitlines()
    assert queue_contents
    payload = json.loads(queue_contents[-1])
    assert payload["id"] == job.id
    status_doc = json.loads((data_dir / "training_status.json").read_text(encoding="utf-8"))
    assert job.id in status_doc
    assert status_doc[job.id]["status"] == "queued"


def test_list_training_jobs_returns_latest_first(tmp_path, monkeypatch):
    data_dir = tmp_path / "data"
    data_dir.mkdir()
    monkeypatch.setattr(training, "TRAINING_QUEUE_PATH", data_dir / "training_jobs.jsonl", raising=False)
    monkeypatch.setattr(training, "TRAINING_STATUS_PATH", data_dir / "training_status.json", raising=False)

    first = training.queue_training_job("global", {"roots": []})
    second = training.queue_training_job("smart-folder", {"folder_id": "abc", "path": "/tmp"})

    jobs = training.list_training_jobs()
    assert jobs[0].id == second.id
    assert jobs[1].id == first.id


def test_claim_next_job_marks_running(tmp_path, monkeypatch):
    data_dir = tmp_path / "data"
    data_dir.mkdir()
    monkeypatch.setattr(training, "TRAINING_QUEUE_PATH", data_dir / "training_jobs.jsonl", raising=False)
    monkeypatch.setattr(training, "TRAINING_STATUS_PATH", data_dir / "training_status.json", raising=False)

    job = training.queue_training_job("smart-folder", {"folder_id": "a", "path": "/tmp"})
    claimed = training.claim_next_job()
    assert claimed is not None
    assert claimed.id == job.id
    status = training.get_job_status(job.id)
    assert status["status"] == "running"
