from __future__ import annotations

from pathlib import Path
from typing import Set

import click

from core.errors import ScanError
from core.monitor import check_drift

from .policy import enforce_cache_limit, load_policy_engine
from .session import command_session


def require_pandas() -> None:
    try:
        import pandas  # noqa: F401
    except Exception:
        raise ScanError("pandas 라이브러리가 필요합니다.", hint="pip install pandas 또는 scripts/setup_env.sh 실행")


def perform_drift_check(
    ctx: click.Context,
    *,
    run_name: str,
    scan_csv: str,
    corpus: str,
    cache_dir: str,
    semantic_baseline: str,
    semantic_threshold: float,
    log_path: str,
    alert_threshold: float,
    policy: str,
    policy_agent: str,
    default_policy_path: Path,
    cache_hard_limit: bool,
    cache_clean_on_limit: bool,
):
    require_pandas()
    policy_engine = load_policy_engine(policy, default_policy_path=default_policy_path, fail_if_missing=False, stage="drift")
    cache_path = Path(cache_dir)
    baseline_path = Path(semantic_baseline) if semantic_baseline else None
    cache_action = None
    if cache_hard_limit:
        cache_action = enforce_cache_limit(
            cache_path,
            policy_engine,
            hard_limit=True,
            clean_on_limit=cache_clean_on_limit,
        )
    with command_session(ctx, run_name) as session:
        report = check_drift(
            Path(scan_csv),
            Path(corpus),
            cache_dir=cache_path,
            log_path=Path(log_path),
            alert_threshold=alert_threshold,
            semantic_baseline=baseline_path,
            semantic_threshold=semantic_threshold,
            policy_engine=policy_engine,
            policy_agent=policy_agent,
            log_policy_metadata=True,
            cache_action=cache_action,
            policy_id=str(getattr(policy_engine, "source", "")) if policy_engine else None,
        )
        if session:
            session.log_metrics(
                {
                    "hash_drift_ratio": report.hash_drift_ratio,
                    "semantic_shift": report.semantic_shift,
                    "new_files": float(len(report.new_files)),
                    "changed_files": float(len(report.changed_files)),
                    "missing_files": float(len(report.missing_files)),
                }
            )
            session.set_tags(
                {
                    "policy": str(policy),
                    "cache_dir": str(cache_dir),
                    "policy_source": str(getattr(policy_engine, "source", "")) if policy_engine else "",
                }
            )
            session.log_params({"cache_action": cache_action or ""})
    return report


def print_drift_report(report, semantic_threshold: float) -> None:
    click.echo(f"📈 hash drift ratio={report.hash_drift_ratio:.3f} (scan={report.scan_rows}, corpus={report.corpus_rows})")
    if report.new_files:
        click.echo(f"➕ 신규 문서 {len(report.new_files)}건 (상위 5개):")
        for path in report.new_files[:5]:
            click.echo(f"   + {path}")
    if report.changed_files:
        click.echo(f"🌀 변경 감지 {len(report.changed_files)}건 (상위 5개):")
        for path in report.changed_files[:5]:
            click.echo(f"   * {path}")
    if report.missing_files:
        click.echo(f"➖ 누락 문서 {len(report.missing_files)}건 (상위 5개):")
        for path in report.missing_files[:5]:
            click.echo(f"   - {path}")
    click.echo(
        f"🎯 semantic shift={report.semantic_shift:.3f} (threshold={semantic_threshold:.2f}, sample={report.semantic_sample_size})"
    )
    if report.reembed_candidates:
        click.echo(f"🔁 재임베딩 후보 {len(report.reembed_candidates)}건 (로그에 기록)")
    if report.recommendations:
        click.echo(f"✅ 권장 조치: {', '.join(report.recommendations)}")


def auto_reembed_targets(report, *, max_candidates: int, include_changed: bool, include_new: bool) -> Set[str]:
    ordered = []
    ordered.extend(report.reembed_candidates or [])
    if include_changed:
        ordered.extend(report.changed_files or [])
    if include_new:
        ordered.extend(report.new_files or [])
    deduped = []
    seen = set()
    for path in ordered:
        path = str(path or "")
        if not path or path in seen:
            continue
        deduped.append(path)
        seen.add(path)
        if max_candidates and len(deduped) >= max_candidates:
            break
    return set(deduped)


__all__ = ["perform_drift_check", "print_drift_report", "auto_reembed_targets"]

