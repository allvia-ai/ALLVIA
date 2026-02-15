from __future__ import annotations

from pathlib import Path

from core.data_pipeline.pipeline import run_step2
from core.errors import PolicyViolationError

from .policy import load_policy_engine
from .scan_rows import load_scan_rows, resolve_scan_csv
from .train_config import build_train_config, maybe_limit_rows


def cmd_train(
    args,
    *,
    default_policy_path: Path,
    default_chunk_cache: Path,
    default_scan_state: Path,
    agent: str,
):
    scan_csv = resolve_scan_csv(Path(args.scan_csv))
    policy_arg = getattr(args, "policy", None)
    policy_normalized = (policy_arg or "").strip().lower()
    policy_required = policy_normalized != "none"
    policy_engine = load_policy_engine(policy_arg, default_policy_path=default_policy_path, fail_if_missing=policy_required, stage="train")
    row_iter = load_scan_rows(scan_csv, policy_engine=policy_engine, include_manual=True, agent=agent)
    rows = maybe_limit_rows(row_iter, getattr(args, "limit_files", 0))

    if not rows:
        raise ValueError("유효한 학습 대상 행이 없습니다. 스캔 CSV를 확인해주세요.")

    cfg = build_train_config(args)
    out_corpus = Path(args.corpus)
    out_model = Path(args.model)
    chunk_cache_path = Path(getattr(args, "chunk_cache", default_chunk_cache))
    state_path = Path(getattr(args, "state_file", default_scan_state))
    df, _ = run_step2(
        rows,
        out_corpus=out_corpus,
        out_model=out_model,
        cfg=cfg,
        use_tqdm=True,
        translate=args.translate,
        scan_state_path=state_path,
        chunk_cache_path=chunk_cache_path,
        skip_extract=bool(getattr(args, "skip_extract", False)),
    )
    metrics = df.attrs.get("metrics", {}) if hasattr(df, "attrs") else {}
    incremental = df.attrs.get("incremental", {}) if hasattr(df, "attrs") else {}
    if metrics:
        metric_str = ", ".join(f"{k}={v}" for k, v in metrics.items())
        print(f"📊 임베딩 품질 지표: {metric_str}")
    print("✅ 학습 완료")
    return {
        "rows": len(rows),
        "corpus": str(out_corpus),
        "model": str(out_model),
        "metrics": metrics,
        "incremental": incremental,
    }


def cmd_extract(
    args,
    *,
    default_policy_path: Path,
    default_chunk_cache: Path,
    default_scan_state: Path,
    agent: str,
):
    scan_csv = resolve_scan_csv(Path(args.scan_csv))
    policy_arg = getattr(args, "policy", None)
    policy_normalized = (policy_arg or "").strip().lower()
    policy_required = policy_normalized != "none"
    policy_engine = load_policy_engine(policy_arg, default_policy_path=default_policy_path, fail_if_missing=policy_required, stage="extract")
    row_iter = load_scan_rows(scan_csv, policy_engine=policy_engine, include_manual=True, agent=agent)
    rows = maybe_limit_rows(row_iter, getattr(args, "limit_files", 0))

    if not rows:
        raise ValueError("유효한 추출 대상 행이 없습니다. 스캔 CSV를 확인해주세요.")

    cfg = build_train_config(args)
    out_corpus = Path(args.corpus)
    out_model = Path(args.model)
    chunk_cache_path = Path(getattr(args, "chunk_cache", default_chunk_cache))
    state_path = Path(getattr(args, "state_file", default_scan_state))
    df, _ = run_step2(
        rows,
        out_corpus=out_corpus,
        out_model=out_model,
        cfg=cfg,
        use_tqdm=True,
        translate=args.translate,
        scan_state_path=state_path,
        chunk_cache_path=chunk_cache_path,
        skip_extract=False,
        train_embeddings=False,
    )
    incremental = df.attrs.get("incremental", {}) if hasattr(df, "attrs") else {}
    print("✅ 추출 완료 (임베딩/모델 생성 없음)")
    return {
        "rows": len(rows),
        "corpus": str(out_corpus),
        "incremental": incremental,
    }


def cmd_embed(
    args,
    *,
    default_policy_path: Path,
    default_chunk_cache: Path,
    default_scan_state: Path,
    agent: str,
):
    scan_csv = resolve_scan_csv(Path(args.scan_csv))
    corpus_path = Path(args.corpus)
    if not corpus_path.exists():
        raise FileNotFoundError(
            f"기존 corpus가 없어 임베딩을 진행할 수 없습니다: {corpus_path}. 먼저 extract/train을 실행하세요."
        )

    policy_arg = getattr(args, "policy", None)
    policy_normalized = (policy_arg or "").strip().lower()
    policy_required = policy_normalized != "none"
    policy_engine = load_policy_engine(policy_arg, default_policy_path=default_policy_path, fail_if_missing=policy_required, stage="embed")
    row_iter = load_scan_rows(scan_csv, policy_engine=policy_engine, include_manual=True, agent=agent)
    rows = maybe_limit_rows(row_iter, getattr(args, "limit_files", 0))

    if not rows:
        raise ValueError("유효한 임베딩 대상 행이 없습니다. 스캔 CSV를 확인해주세요.")

    cfg = build_train_config(args)
    out_model = Path(args.model)
    chunk_cache_path = Path(getattr(args, "chunk_cache", default_chunk_cache))
    state_path = Path(getattr(args, "state_file", default_scan_state))
    df, _ = run_step2(
        rows,
        out_corpus=corpus_path,
        out_model=out_model,
        cfg=cfg,
        use_tqdm=True,
        translate=args.translate,
        scan_state_path=state_path,
        chunk_cache_path=chunk_cache_path,
        skip_extract=True,
        train_embeddings=True,
    )
    metrics = df.attrs.get("metrics", {}) if hasattr(df, "attrs") else {}
    incremental = df.attrs.get("incremental", {}) if hasattr(df, "attrs") else {}
    if metrics:
        metric_str = ", ".join(f"{k}={v}" for k, v in metrics.items())
        print(f"📊 임베딩 품질 지표: {metric_str}")
    print("✅ 임베딩/모델 생성 완료 (기존 corpus 사용)")
    return {
        "rows": len(rows),
        "corpus": str(corpus_path),
        "model": str(out_model),
        "metrics": metrics,
        "incremental": incremental,
    }


def require_policy_or_roots(roots, policy_engine, *, message: str) -> None:
    if roots:
        return
    if policy_engine and getattr(policy_engine, "has_policies", False):
        return
    raise PolicyViolationError(message)


__all__ = ["cmd_embed", "cmd_extract", "cmd_train", "require_policy_or_roots"]

