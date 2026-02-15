from __future__ import annotations

import tempfile
from pathlib import Path
from typing import Optional

import click

try:
    import pandas as pd
except Exception:  # pragma: no cover
    pd = None  # type: ignore

from core.agents.document import DocumentAgent, DocumentAgentConfig
from core.data_pipeline.pipeline import PARQUET_ENGINE
from core.errors import PolicyViolationError

from .drift import require_pandas
from .policy import dir_size_bytes, enforce_cache_limit, load_policy_engine


def cmd_index(
    args,
    *,
    default_policy_path: Path,
    agent: str,
) -> dict:
    policy_arg = getattr(args, "policy", None)
    policy_normalized = (policy_arg or "").strip().lower()
    policy_required = policy_normalized != "none"
    policy_engine = load_policy_engine(
        policy_arg,
        default_policy_path=default_policy_path,
        fail_if_missing=policy_required,
        stage="index",
    )
    scope = getattr(args, "scope", "auto")

    limit = max(0, int(getattr(args, "limit_files", 0) or 0))
    corpus_path = Path(args.corpus)
    tmp_corpus: Optional[Path] = None

    if limit:
        require_pandas()
        if pd is None:
            raise PolicyViolationError("pandas 라이브러리가 필요합니다.")
        try:
            df = pd.read_parquet(corpus_path)
        except Exception as exc:
            raise PolicyViolationError(f"코퍼스를 불러오지 못했습니다: {exc}") from exc
        if len(df) > limit:
            df = df.iloc[:limit].copy()
        tmp_dir = Path(args.cache) / "tmp_index"
        try:
            tmp_dir.mkdir(parents=True, exist_ok=True)
        except PermissionError:
            try:
                tmp_dir = Path(tempfile.mkdtemp(prefix="tmp_index_", dir=str(Path(args.cache).parent)))
            except Exception:
                tmp_dir = Path(tempfile.mkdtemp(prefix="tmp_index_"))
        tmp_corpus = tmp_dir / f"corpus_limit_{limit}.parquet"
        engine = PARQUET_ENGINE or "pyarrow"
        df.to_parquet(tmp_corpus, engine=engine, index=False)
        corpus_path = tmp_corpus
        click.echo(f"⚡ 상위 {limit:,}개 문서로 제한하여 인덱싱합니다. ({corpus_path})")

    cfg = DocumentAgentConfig(
        model_path=Path(args.model),
        corpus_path=corpus_path,
        cache_dir=Path(args.cache),
        translate=getattr(args, "translate", False),
        rerank=False,
        llm_backend="none",
        policy_engine=policy_engine,
        policy_scope=scope,
        policy_agent=agent,
        rebuild_index=True,
    )
    agent_runner = DocumentAgent(cfg)
    agent_runner.prepare()
    cache_usage = dir_size_bytes(cfg.cache_dir)
    enforce_cache_limit(
        cfg.cache_dir,
        policy_engine,
        hard_limit=getattr(args, "cache_hard_limit", False),
        clean_on_limit=getattr(args, "cache_clean_on_limit", False),
    )
    print(f"✅ 인덱스/캐시 갱신 완료 (cache ~{cache_usage:,} bytes)")
    return {
        "cache": str(cfg.cache_dir),
        "corpus": str(cfg.corpus_path),
        "cache_usage_bytes": cache_usage,
        "tmp_corpus": str(tmp_corpus) if tmp_corpus else "",
    }


__all__ = ["cmd_index"]
