"""Prefect 기반 파이프라인 오케스트레이션 + 재사용 가능한 Runner."""

from __future__ import annotations

import argparse
import json
import threading
import time
from dataclasses import asdict, dataclass, field
from pathlib import Path
from types import SimpleNamespace
from typing import Any, Dict, Iterable, List, Optional, Tuple
import sys

REPO_ROOT = Path(__file__).resolve().parents[1]
if str(REPO_ROOT) not in sys.path:
    sys.path.insert(0, str(REPO_ROOT))

from core.config.paths import CACHE_DIR, CORPUS_PATH, DATA_DIR, TOPIC_MODEL_PATH
from core.data_pipeline.pipeline import DEFAULT_EMBED_MODEL, DEFAULT_N_COMPONENTS
from core.search.retriever import Retriever
from scripts.evaluate_rag import evaluate as evaluate_cases
from scripts.evaluate_rag import load_cases as load_eval_cases
from scripts.pipeline.infopilot import (
    DEFAULT_CHUNK_CACHE,
    DEFAULT_FOUND_FILES,
    DEFAULT_POLICY_PATH,
    DEFAULT_SCAN_STATE,
    cmd_index,
    cmd_scan,
    cmd_train,
)

try:  # Prefect는 선택적 의존성
    from prefect import flow, get_run_logger

    PREFECT_AVAILABLE = True
except Exception:  # pragma: no cover - Prefect 미설치 환경 대응
    PREFECT_AVAILABLE = False


# ---------------------------------------------------------------------------
# 데이터 클래스 및 설정
# ---------------------------------------------------------------------------
def _as_path(value: Optional[str | Path], default: Path) -> Path:
    if value is None:
        return default
    return Path(value)


@dataclass
class PipelineConfig:
    """파이프라인 실행 컨피그(공통 사용)."""

    scan_csv: Path = DEFAULT_FOUND_FILES
    roots: Tuple[str, ...] = field(default_factory=tuple)
    exts: Tuple[str, ...] = field(default_factory=tuple)
    policy_path: Path = DEFAULT_POLICY_PATH
    corpus_path: Path = CORPUS_PATH
    model_path: Path = TOPIC_MODEL_PATH
    cache_dir: Path = CACHE_DIR
    chunk_cache: Path = DEFAULT_CHUNK_CACHE
    state_file: Path = DEFAULT_SCAN_STATE
    translate: bool = False
    limit_files: int = 0
    max_features: int = 50_000
    n_components: int = DEFAULT_N_COMPONENTS
    n_clusters: int = 25
    min_df: int = 2
    max_df: float = 0.7
    embedding_model: str = DEFAULT_EMBED_MODEL
    embedding_batch_size: int = 32
    embedding_concurrency: int = 2
    async_embed: bool = True
    evaluation_cases: Optional[Path] = None
    evaluation_top_k: int = 5

    def to_dict(self) -> Dict[str, Any]:
        payload = asdict(self)
        for key, value in list(payload.items()):
            if isinstance(value, Path):
                payload[key] = str(value)
            elif isinstance(value, tuple):
                payload[key] = list(value)
        return payload

    @classmethod
    def from_dict(cls, payload: Dict[str, Any]) -> "PipelineConfig":
        data = dict(payload or {})
        return cls(
            scan_csv=_as_path(data.get("scan_csv"), DEFAULT_FOUND_FILES),
            roots=tuple(data.get("roots") or ()),
            exts=tuple(data.get("exts") or ()),
            policy_path=_as_path(data.get("policy_path"), DEFAULT_POLICY_PATH),
            corpus_path=_as_path(data.get("corpus_path"), CORPUS_PATH),
            model_path=_as_path(data.get("model_path"), TOPIC_MODEL_PATH),
            cache_dir=_as_path(data.get("cache_dir"), CACHE_DIR),
            chunk_cache=_as_path(data.get("chunk_cache"), DEFAULT_CHUNK_CACHE),
            state_file=_as_path(data.get("state_file"), DEFAULT_SCAN_STATE),
            translate=bool(data.get("translate", False)),
            limit_files=int(data.get("limit_files") or 0),
            max_features=int(data.get("max_features") or 50_000),
            n_components=int(data.get("n_components") or DEFAULT_N_COMPONENTS),
            n_clusters=int(data.get("n_clusters") or 25),
            min_df=int(data.get("min_df") or 2),
            max_df=float(data.get("max_df") or 0.7),
            embedding_model=str(data.get("embedding_model") or DEFAULT_EMBED_MODEL),
            embedding_batch_size=int(data.get("embedding_batch_size") or 32),
            embedding_concurrency=int(data.get("embedding_concurrency") or 2),
            async_embed=bool(data.get("async_embed", True)),
            evaluation_cases=Path(data["evaluation_cases"]) if data.get("evaluation_cases") else None,
            evaluation_top_k=int(data.get("evaluation_top_k") or 5),
        )


@dataclass
class StageSnapshot:
    name: str
    status: str
    started_at: float
    finished_at: float
    duration: float
    details: Dict[str, Any]

    def to_dict(self) -> Dict[str, Any]:
        return {
            "name": self.name,
            "status": self.status,
            "started_at": self.started_at,
            "finished_at": self.finished_at,
            "duration": self.duration,
            "details": self.details,
        }


@dataclass
class PipelineResult:
    started_at: float
    finished_at: float
    stages: Dict[str, StageSnapshot]

    def to_dict(self) -> Dict[str, Any]:
        return {
            "started_at": self.started_at,
            "finished_at": self.finished_at,
            "duration": self.finished_at - self.started_at,
            "stages": {name: snapshot.to_dict() for name, snapshot in self.stages.items()},
        }

    @classmethod
    def from_dict(cls, payload: Dict[str, Any]) -> "PipelineResult":
        stages = {
            name: StageSnapshot(
                name=name,
                status=data.get("status", "unknown"),
                started_at=float(data.get("started_at") or 0.0),
                finished_at=float(data.get("finished_at") or 0.0),
                duration=float(data.get("duration") or 0.0),
                details=dict(data.get("details") or {}),
            )
            for name, data in (payload.get("stages") or {}).items()
        }
        return cls(
            started_at=float(payload.get("started_at") or time.time()),
            finished_at=float(payload.get("finished_at") or time.time()),
            stages=stages,
        )


# ---------------------------------------------------------------------------
# Reporter 인터페이스
# ---------------------------------------------------------------------------
class PipelineReporter:
    """파이프라인 상태 업데이트 콜백."""

    def pipeline_started(self, config: PipelineConfig) -> None:
        pass

    def pipeline_completed(self, result: PipelineResult) -> None:
        pass

    def pipeline_failed(self, error: Exception) -> None:
        pass

    def stage_started(self, stage: str) -> None:
        pass

    def stage_succeeded(self, stage: str, details: Dict[str, Any]) -> None:
        pass

    def stage_failed(self, stage: str, error: Exception) -> None:
        pass

    def stage_skipped(self, stage: str, reason: str) -> None:
        pass


class NullReporter(PipelineReporter):
    pass


class PrefectReporter(PipelineReporter):
    """Prefect 로그와 연동되는 Reporter."""

    def __init__(self) -> None:
        self.logger = get_run_logger()

    def pipeline_started(self, config: PipelineConfig) -> None:
        self.logger.info("Pipeline started with config: %s", json.dumps(config.to_dict(), ensure_ascii=False))

    def pipeline_completed(self, result: PipelineResult) -> None:
        self.logger.info("Pipeline completed in %.2fs", result.finished_at - result.started_at)

    def pipeline_failed(self, error: Exception) -> None:
        self.logger.error("Pipeline failed: %s", error)

    def stage_started(self, stage: str) -> None:
        self.logger.info("Stage '%s' started", stage)

    def stage_succeeded(self, stage: str, details: Dict[str, Any]) -> None:
        self.logger.info("Stage '%s' succeeded → %s", stage, details)

    def stage_failed(self, stage: str, error: Exception) -> None:
        self.logger.error("Stage '%s' failed: %s", stage, error)

    def stage_skipped(self, stage: str, reason: str) -> None:
        self.logger.warning("Stage '%s' skipped (%s)", stage, reason)


# ---------------------------------------------------------------------------
# 오케스트레이터
# ---------------------------------------------------------------------------
class PipelineOrchestrator:
    STAGES = ("scan", "train", "index", "evaluate")

    def run(
        self,
        config: PipelineConfig,
        *,
        reporter: Optional[PipelineReporter] = None,
        cancel_event: Optional[threading.Event] = None,
    ) -> PipelineResult:
        reporter = reporter or NullReporter()
        started = time.time()
        stages: Dict[str, StageSnapshot] = {}

        def _run_stage(name: str, func):
            self._check_cancel(cancel_event)
            reporter.stage_started(name)
            stage_start = time.time()
            try:
                details = func()
            except Exception as exc:
                reporter.stage_failed(name, exc)
                raise
            stage_end = time.time()
            reporter.stage_succeeded(name, details)
            stages[name] = StageSnapshot(
                name=name,
                status="completed",
                started_at=stage_start,
                finished_at=stage_end,
                duration=stage_end - stage_start,
                details=details,
            )

        reporter.pipeline_started(config)
        try:
            _run_stage("scan", lambda: self._run_scan(config))
            _run_stage("train", lambda: self._run_train(config))
            _run_stage("index", lambda: self._run_index(config))

            if config.evaluation_cases:
                _run_stage("evaluate", lambda: self._run_evaluate(config))
            else:
                reporter.stage_skipped("evaluate", "no_evaluation_cases")
                stages["evaluate"] = StageSnapshot(
                    name="evaluate",
                    status="skipped",
                    started_at=time.time(),
                    finished_at=time.time(),
                    duration=0.0,
                    details={"skipped": True, "reason": "no_evaluation_cases"},
                )
        except Exception as exc:
            reporter.pipeline_failed(exc)
            raise

        finished = time.time()
        result = PipelineResult(started_at=started, finished_at=finished, stages=stages)
        reporter.pipeline_completed(result)
        return result

    @staticmethod
    def _check_cancel(cancel_event: Optional[threading.Event]) -> None:
        if cancel_event and cancel_event.is_set():
            raise RuntimeError("pipeline cancelled")

    def _run_scan(self, config: PipelineConfig) -> Dict[str, Any]:
        config.scan_csv.parent.mkdir(parents=True, exist_ok=True)
        DATA_DIR.mkdir(parents=True, exist_ok=True)
        args = SimpleNamespace(
            out=str(config.scan_csv),
            roots=tuple(config.roots),
            policy=str(config.policy_path),
            exts=tuple(config.exts) if config.exts else None,
        )
        count = cmd_scan(args)
        return {"files": count, "scan_csv": str(config.scan_csv)}

    def _run_train(self, config: PipelineConfig) -> Dict[str, Any]:
        args = SimpleNamespace(
            scan_csv=str(config.scan_csv),
            corpus=str(config.corpus_path),
            model=str(config.model_path),
            max_features=config.max_features,
            n_components=config.n_components,
            n_clusters=config.n_clusters,
            min_df=config.min_df,
            max_df=config.max_df,
            embedding_model=config.embedding_model,
            embedding_batch_size=config.embedding_batch_size,
            limit_files=config.limit_files,
            translate=config.translate,
            use_embedding=True,
            policy=str(config.policy_path),
            state_file=str(config.state_file),
            chunk_cache=str(config.chunk_cache),
            embedding_concurrency=config.embedding_concurrency,
            async_embed=config.async_embed,
        )
        return cmd_train(args)

    def _run_index(self, config: PipelineConfig) -> Dict[str, Any]:
        config.cache_dir.mkdir(parents=True, exist_ok=True)
        args = SimpleNamespace(
            model=str(config.model_path),
            corpus=str(config.corpus_path),
            cache=str(config.cache_dir),
            translate=config.translate,
            policy=str(config.policy_path),
            scope="auto",
        )
        return cmd_index(args)

    def _run_evaluate(self, config: PipelineConfig) -> Dict[str, Any]:
        cases_path = config.evaluation_cases
        if cases_path is None:
            return {"skipped": True}
        cases = load_eval_cases(Path(cases_path))
        retriever = Retriever(
            model_path=config.model_path,
            corpus_path=config.corpus_path,
            cache_dir=config.cache_dir,
            auto_refresh=False,
        )
        try:
            retriever.ready(rebuild=False, wait=True)
            metrics = evaluate_cases(retriever, cases, top_k=max(1, config.evaluation_top_k))
        finally:
            retriever.shutdown()
        return metrics


# ---------------------------------------------------------------------------
# Helper functions
# ---------------------------------------------------------------------------
def run_pipeline(
    config: PipelineConfig,
    *,
    reporter: Optional[PipelineReporter] = None,
    cancel_event: Optional[threading.Event] = None,
) -> PipelineResult:
    orchestrator = PipelineOrchestrator()
    return orchestrator.run(config, reporter=reporter, cancel_event=cancel_event)


def run_prefect_flow(config: PipelineConfig) -> Dict[str, Any]:
    if not PREFECT_AVAILABLE:  # pragma: no cover - Prefect 미설치
        raise RuntimeError("Prefect가 설치되어 있지 않습니다. pip install prefect 로 설치하세요.")
    result_dict = prefect_pipeline_flow(config.to_dict())
    return result_dict


# Prefect Flow 정의 (Prefect 설치 시에만)
if PREFECT_AVAILABLE:  # pragma: no cover - Prefect 설치 환경에서만 실행

    @flow(name="infopilot-pipeline")
    def prefect_pipeline_flow(config_payload: Dict[str, Any]) -> Dict[str, Any]:
        cfg = PipelineConfig.from_dict(config_payload)
        reporter = PrefectReporter()
        result = run_pipeline(cfg, reporter=reporter)
        return result.to_dict()


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------
def _build_arg_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Prefect + FastAPI에서 재사용하는 파이프라인 Runner")
    parser.add_argument("--scan-csv", default=str(DEFAULT_FOUND_FILES), help="스캔 결과 CSV")
    parser.add_argument("--corpus", default=str(CORPUS_PATH), help="코퍼스 parquet 경로")
    parser.add_argument("--model", default=str(TOPIC_MODEL_PATH), help="토픽 모델 joblib 경로")
    parser.add_argument("--cache", default=str(CACHE_DIR), help="인덱스 캐시 디렉터리")
    parser.add_argument("--policy", default=str(DEFAULT_POLICY_PATH), help="스마트 폴더 정책 파일")
    parser.add_argument("--root", dest="roots", action="append", default=[], help="스캔 루트를 여러 번 지정 가능")
    parser.add_argument("--ext", dest="exts", action="append", default=[], help="스캔 확장자 제한")
    parser.add_argument("--translate", action="store_true", help="번역 모드 활성화")
    parser.add_argument("--evaluation-cases", help="평가용 JSONL 경로")
    parser.add_argument("--evaluation-top-k", type=int, default=5, help="평가 시 top-k")
    parser.add_argument("--use-prefect", action="store_true", help="Prefect Flow로 실행")
    parser.add_argument("--limit-files", type=int, default=0, help="테스트용 상위 N개 파일만 처리")
    return parser


def main(argv: Optional[Iterable[str]] = None) -> None:
    parser = _build_arg_parser()
    args = parser.parse_args(argv)
    config = PipelineConfig(
        scan_csv=Path(args.scan_csv),
        corpus_path=Path(args.corpus),
        model_path=Path(args.model),
        cache_dir=Path(args.cache),
        policy_path=Path(args.policy),
        roots=tuple(args.roots or ()),
        exts=tuple(args.exts or ()),
        translate=bool(args.translate),
        evaluation_cases=Path(args.evaluation_cases) if args.evaluation_cases else None,
        evaluation_top_k=max(1, int(args.evaluation_top_k)),
        limit_files=max(0, int(args.limit_files)),
    )

    if args.use_prefect:
        result = run_prefect_flow(config)
    else:
        pipeline_result = run_pipeline(config)
        result = pipeline_result.to_dict()

    print(json.dumps(result, ensure_ascii=False, indent=2))


if __name__ == "__main__":  # pragma: no cover - CLI entry
    main()
