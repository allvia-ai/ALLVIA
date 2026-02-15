# scripts/pipeline/infopilot_cli/schedule.py
from __future__ import annotations

import time
from pathlib import Path
from typing import List

from core.infra.scheduler import JobScheduler, ScheduleSpec, ScheduledJob
from core.policy.engine import PolicyEngine
from core.data_pipeline.pipeline import run_step2, default_train_config
from scripts.pipeline.infopilot_cli.scan import run_scan
from scripts.pipeline.infopilot_cli.policy_utils import get_policy_artifacts, policy_slug

def register_policy_jobs(
    scheduler: JobScheduler,
    *,
    policy_engine: PolicyEngine,
    agent: str,
    output_root: Path,
    translate: bool,
) -> List[ScheduledJob]:
    if not policy_engine or not policy_engine.has_policies:
        return []

    registered: List[ScheduledJob] = []
    output_root = output_root.expanduser()

    for policy in policy_engine.iter_policies():
        if not policy.allows_agent(agent):
            continue
        schedule = ScheduleSpec.from_policy(policy)
        if schedule.mode != "scheduled":
            continue

        artifacts = get_policy_artifacts(output_root, policy)

        def _job(policy=policy, artifacts=artifacts) -> None:
            artifacts.ensure_dirs()
            # Reuse core run_scan but adapt arguments if needed
            rows = run_scan(artifacts.scan_csv, [policy.path], policy_engine=policy_engine, agent=agent)
            
            # Re-filter just in case
            filtered = policy_engine.filter_records(rows, agent=agent, include_manual=True)
            if not filtered and rows:
                filtered = rows
            
            if not filtered:
                print(f"⚠️ 스케줄러: {policy.path}에 처리할 문서가 없어 건너뜁니다.")
                return
            
            cfg = default_train_config()
            run_step2(
                filtered,
                out_corpus=artifacts.corpus,
                out_model=artifacts.model,
                cfg=cfg,
                use_tqdm=False,
                translate=translate,
            )
            print(f"✅ 스케줄러: {policy.path} 학습 완료 → {artifacts.base_dir}")

        job_name = f"{agent}:{policy_slug(policy)}"
        metadata = {
            "path": str(policy.path),
            "artifact_dir": str(artifacts.base_dir),
            "mode": schedule.mode,
        }
        job = scheduler.register_callable(
            job_name,
            _job,
            schedule,
            metadata=metadata,
            overwrite=True,
        )
        registered.append(job)

    return registered

def cmd_schedule(args, knowledge_agent: str):
    from scripts.pipeline.infopilot_cli.policy import load_policy_engine
    
    # We need to import default policy path from somewhere or pass it
    # For now, let's assume args carries necessary info or we import constants if needed
    # But ideally, we shouldn't depend on global constants from infopilot.py
    # We will accept policy path in args or use a utility
    
    policy_engine = load_policy_engine(
        getattr(args, "policy", None), 
        default_policy_path=None, # Will be handled by loader if None
        fail_if_missing=True, 
        stage="schedule"
    )
    
    if not policy_engine or not policy_engine.has_policies:
        print("⚠️ 스케줄러: 정책이 없어 종료합니다.")
        return

    if args.agent != knowledge_agent:
        print("⚠️ 스케줄러: 현재는 knowledge_search 에이전트 예약만 지원합니다.")
        return

    scheduler = JobScheduler()
    jobs = register_policy_jobs(
        scheduler,
        policy_engine=policy_engine,
        agent=args.agent,
        output_root=Path(args.output_root),
        translate=args.translate,
    )

    if not jobs:
        print("⚠️ 스케줄러: 예약 작업이 없습니다. 정책의 indexing.mode를 확인해주세요.")
        return

    for job in jobs:
        next_run = job.next_run.isoformat() if job.next_run else "manual"
        print(f"⏱️ {job.metadata.get('path', job.name)} → 다음 실행: {next_run}")

    poll = max(5.0, float(getattr(args, "poll_seconds", 60.0)))
    if getattr(args, "once", False):
        scheduler.run_pending()
        return

    print("🚀 정책 스케줄러를 시작합니다. (Ctrl+C로 종료)")
    try:
        while True:
            scheduler.run_pending()
            time.sleep(poll)
    except KeyboardInterrupt:
        print("👋 스케줄러를 종료합니다.")
