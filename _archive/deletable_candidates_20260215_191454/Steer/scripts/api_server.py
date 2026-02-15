"""FastAPI 기반 파이프라인 관리 서버."""

from __future__ import annotations

import os
import threading
import time
from pathlib import Path
from typing import Any, Dict, List, Optional
import sys

repo_root = Path(__file__).resolve().parents[1]
if str(repo_root) not in sys.path:
    sys.path.insert(0, str(repo_root))

from fastapi import Depends, FastAPI, Header, HTTPException
from pydantic import BaseModel, Field

from core.config.paths import CACHE_DIR, CORPUS_PATH, TOPIC_MODEL_PATH
from scripts.pipeline.infopilot import DEFAULT_FOUND_FILES, DEFAULT_POLICY_PATH
from scripts.prefect_dag import (
    PipelineConfig,
    PipelineReporter,
    PipelineResult,
    PREFECT_AVAILABLE,
    run_pipeline,
    run_prefect_flow,
)


# ---------------------------------------------------------------------------
# Reporter & Runner
# ---------------------------------------------------------------------------
class StatusTracker(PipelineReporter):
    """Thread-safe reporter → FastAPI 응답으로 그대로 사용."""

    def __init__(self) -> None:
        self._lock = threading.Lock()
        self._state: Dict[str, Any] = {
            "state": "idle",
            "stage": None,
            "started_at": None,
            "finished_at": None,
            "history": [],
            "result": None,
            "error": None,
        }

    def _update(self, **kwargs) -> None:
        with self._lock:
            self._state.update(kwargs)

    def _append_history(self, record: Dict[str, Any]) -> None:
        with self._lock:
            history = list(self._state.get("history") or [])
            history.append(record)
            self._state["history"] = history[-50:]  # 최근 50개만 유지

    def pipeline_started(self, config: PipelineConfig) -> None:
        self._update(
            state="running",
            stage=None,
            started_at=time.time(),
            finished_at=None,
            error=None,
            result=None,
        )

    def pipeline_completed(self, result: PipelineResult) -> None:
        self._update(
            state="completed",
            finished_at=result.finished_at,
            stage=None,
            result=result.to_dict(),
        )

    def pipeline_failed(self, error: Exception) -> None:
        self._update(
            state="failed",
            finished_at=time.time(),
            stage=None,
            error=str(error),
        )

    def stage_started(self, stage: str) -> None:
        self._update(stage=stage)
        self._append_history(
            {"stage": stage, "status": "started", "timestamp": time.time()}
        )

    def stage_succeeded(self, stage: str, details: Dict[str, Any]) -> None:
        self._append_history(
            {
                "stage": stage,
                "status": "completed",
                "timestamp": time.time(),
                "details": details,
            }
        )

    def stage_failed(self, stage: str, error: Exception) -> None:
        self._append_history(
            {
                "stage": stage,
                "status": "failed",
                "timestamp": time.time(),
                "error": str(error),
            }
        )

    def stage_skipped(self, stage: str, reason: str) -> None:
        self._append_history(
            {
                "stage": stage,
                "status": "skipped",
                "timestamp": time.time(),
                "reason": reason,
            }
        )

    def snapshot(self) -> Dict[str, Any]:
        with self._lock:
            return {
                "state": self._state.get("state"),
                "stage": self._state.get("stage"),
                "started_at": self._state.get("started_at"),
                "finished_at": self._state.get("finished_at"),
                "history": list(self._state.get("history") or []),
                "result": self._state.get("result"),
                "error": self._state.get("error"),
            }


class PipelineRunner:
    def __init__(self) -> None:
        self._lock = threading.Lock()
        self._thread: Optional[threading.Thread] = None
        self._cancel_event = threading.Event()
        self._tracker = StatusTracker()
        self._use_prefect = False

    def start(self, config: PipelineConfig, *, use_prefect: bool = False) -> Dict[str, Any]:
        with self._lock:
            if self._thread and self._thread.is_alive():
                raise RuntimeError("이미 실행 중인 파이프라인이 있습니다.")
            self._cancel_event.clear()
            self._use_prefect = bool(use_prefect)
            tracker = StatusTracker()
            self._tracker = tracker
            self._thread = threading.Thread(
                target=self._run_pipeline,
                args=(config, tracker, use_prefect),
                daemon=True,
            )
            self._thread.start()
            return tracker.snapshot()

    def _run_pipeline(self, config: PipelineConfig, tracker: StatusTracker, use_prefect: bool) -> None:
        try:
            if use_prefect:
                tracker.pipeline_started(config)
                tracker.stage_started("prefect")
                result_dict = run_prefect_flow(config)
                tracker.stage_succeeded("prefect", {"mode": "prefect"})
                tracker.pipeline_completed(PipelineResult.from_dict(result_dict))
                return

            run_pipeline(
                config,
                reporter=tracker,
                cancel_event=self._cancel_event,
            )
        except Exception as exc:
            tracker.pipeline_failed(exc)

    def cancel(self) -> Dict[str, Any]:
        with self._lock:
            if not self._thread or not self._thread.is_alive():
                raise RuntimeError("실행 중인 파이프라인이 없습니다.")
            if self._use_prefect:
                raise RuntimeError("Prefect 실행은 현재 취소를 지원하지 않습니다.")
            self._cancel_event.set()
        return self._tracker.snapshot()

    def status(self) -> Dict[str, Any]:
        return self._tracker.snapshot()


# ---------------------------------------------------------------------------
# FastAPI 모델
# ---------------------------------------------------------------------------
class PipelineRunRequest(BaseModel):
    roots: List[str] = Field(default_factory=list, description="스캔 루트 디렉터리")
    exts: List[str] = Field(default_factory=list, description="스캔 확장자 필터")
    scan_csv: Optional[str] = Field(None, description="스캔 CSV 경로")
    corpus: Optional[str] = Field(None, description="코퍼스 parquet")
    model: Optional[str] = Field(None, description="토픽 모델 joblib")
    cache: Optional[str] = Field(None, description="캐시 디렉터리")
    policy: Optional[str] = Field(None, description="스마트 폴더 정책 파일")
    translate: bool = False
    evaluation_cases: Optional[str] = Field(None, description="평가용 JSONL")
    evaluation_top_k: int = 5
    limit_files: int = 0
    use_prefect: bool = False

    def to_config(self) -> PipelineConfig:
        return PipelineConfig(
            scan_csv=Path(self.scan_csv) if self.scan_csv else DEFAULT_FOUND_FILES,
            corpus_path=Path(self.corpus) if self.corpus else CORPUS_PATH,
            model_path=Path(self.model) if self.model else TOPIC_MODEL_PATH,
            cache_dir=Path(self.cache) if self.cache else CACHE_DIR,
            policy_path=Path(self.policy) if self.policy else DEFAULT_POLICY_PATH,
            roots=tuple(self.roots or ()),
            exts=tuple(self.exts or ()),
            translate=self.translate,
            evaluation_cases=Path(self.evaluation_cases) if self.evaluation_cases else None,
            evaluation_top_k=max(1, int(self.evaluation_top_k)),
            limit_files=max(0, int(self.limit_files)),
        )


class CancelResponse(BaseModel):
    status: Dict[str, Any]


# ---------------------------------------------------------------------------
# FastAPI 앱
# ---------------------------------------------------------------------------
app = FastAPI(
    title="InfoPilot Pipeline API",
    version="1.0",
    description="로컬 CLI 파이프라인을 FastAPI로 래핑한 제어 서버",
)

RUNNER = PipelineRunner()
API_TOKEN = os.getenv("INFOPILOT_API_TOKEN", "").strip()


def require_token(header_token: str = Header(default="", alias="X-API-Token")) -> None:
    if not API_TOKEN:
        return
    if header_token != API_TOKEN:
        raise HTTPException(status_code=401, detail="invalid or missing API token")


@app.get("/health")
def health() -> Dict[str, Any]:
    return {
        "status": "ok",
        "prefect_available": PREFECT_AVAILABLE,
    }


@app.get("/pipeline/status", dependencies=[Depends(require_token)])
def pipeline_status() -> Dict[str, Any]:
    return RUNNER.status()


@app.post("/pipeline/run", dependencies=[Depends(require_token)])
def pipeline_run(body: PipelineRunRequest) -> Dict[str, Any]:
    config = body.to_config()
    try:
        status = RUNNER.start(config, use_prefect=body.use_prefect)
    except RuntimeError as exc:
        raise HTTPException(status_code=409, detail=str(exc))
    return {"message": "pipeline started", "status": status}


@app.post("/pipeline/cancel", response_model=CancelResponse, dependencies=[Depends(require_token)])
def pipeline_cancel() -> CancelResponse:
    try:
        status = RUNNER.cancel()
    except RuntimeError as exc:
        raise HTTPException(status_code=409, detail=str(exc))
    return CancelResponse(status=status)


if __name__ == "__main__":  # pragma: no cover - 수동 실행 지원
    import uvicorn

    uvicorn.run("scripts.api_server:app", host="127.0.0.1", port=8080, reload=False)
