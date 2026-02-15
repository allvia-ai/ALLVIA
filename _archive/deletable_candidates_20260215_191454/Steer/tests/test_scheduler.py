from __future__ import annotations

from datetime import datetime
from pathlib import Path

import pytest

import infopilot
from core.policy.engine import PolicyEngine, SmartFolderPolicy
from core.infra import JobScheduler, ScheduleSpec

pytestmark = pytest.mark.smoke


def test_schedule_spec_from_policy_cron() -> None:
    policy = SmartFolderPolicy(
        path=Path("/tmp"),
        agents=frozenset({infopilot.KNOWLEDGE_AGENT}),
        security={},
        indexing={"mode": "scheduled", "cron": "* * * * *"},
        retention={},
    )
    spec = ScheduleSpec.from_policy(policy)
    assert spec.mode == "scheduled"
    now = datetime.utcnow().replace(second=0, microsecond=0)
    next_run = spec.next_run(now=now)
    assert next_run is not None
    assert next_run >= now


def test_register_policy_jobs_executes(monkeypatch, tmp_path) -> None:
    policy_root = tmp_path / "docs"
    policy_root.mkdir()
    (policy_root / "doc.txt").touch()

    policy = SmartFolderPolicy(
        path=policy_root,
        agents=frozenset({infopilot.KNOWLEDGE_AGENT}),
        security={},
        indexing={"mode": "scheduled", "cron": "* * * * *"},
        retention={},
    )
    engine = PolicyEngine((policy,), source=None)

    calls = {}

    def fake_run_scan(out, roots, **kwargs):
        print(f"DEBUG: fake_run_scan called with roots={roots}")
        calls["scan_out"] = out
        calls["roots"] = roots
        return [{"path": str(policy_root / "doc.txt")}]

    def fake_run_step2(rows, out_corpus, out_model, cfg, use_tqdm, translate):
        calls["rows"] = list(rows)
        calls["corpus"] = out_corpus
        calls["model"] = out_model
        calls["translate"] = translate
        return None, None

    import scripts.pipeline.infopilot_cli.schedule as schedule_module
    monkeypatch.setattr(schedule_module, "run_scan", fake_run_scan)
    monkeypatch.setattr(schedule_module, "run_step2", fake_run_step2)

    scheduler = JobScheduler()
    jobs = infopilot._register_policy_jobs(
        scheduler,
        policy_engine=engine,
        agent=infopilot.KNOWLEDGE_AGENT,
        output_root=tmp_path / "artifacts",
        translate=False,
    )

    assert len(jobs) == 1
    job = jobs[0]
    assert job.next_run is not None

    scheduler.run_pending(now=job.next_run)

    assert "rows" in calls
    assert calls["translate"] is False
    assert calls["scan_out"].parent.exists()
