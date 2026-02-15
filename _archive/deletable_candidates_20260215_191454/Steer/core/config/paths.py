"""Centralised default path definitions for runtime artifacts."""
from __future__ import annotations

from pathlib import Path

import os

PROJECT_ROOT = Path(__file__).resolve().parents[2]
env_data = os.getenv("INFOPILOT_DATA_DIR")
DATA_DIR = Path(env_data) if env_data else PROJECT_ROOT / "data"
CACHE_DIR = DATA_DIR / "cache"
SUMMARIES_DIR = DATA_DIR / "summaries"
DOCS_DIR = DATA_DIR / "documents"
MODELS_DIR = PROJECT_ROOT / "models"
ARTIFACTS_DIR = PROJECT_ROOT / "artifacts"
LOGS_DIR = ARTIFACTS_DIR / "logs"
RUNTIME_LOG_DIR = LOGS_DIR / "runtime"
AUDIT_LOG_DIR = LOGS_DIR / "audit"
METRICS_PATH = LOGS_DIR / "metrics" / "system.json"
RESOURCE_LOG_PATH = LOGS_DIR / "resource_log.jsonl"
DRIFT_LOG_PATH = LOGS_DIR / "drift_log.jsonl"
SEMANTIC_BASELINE_PATH = LOGS_DIR / "semantic_baseline.json"
TOPIC_MODEL_PATH = DATA_DIR / "topic_model.joblib"
CORPUS_PATH = DATA_DIR / "corpus.parquet"

__all__ = [
    "PROJECT_ROOT",
    "DATA_DIR",
    "CACHE_DIR",
    "SUMMARIES_DIR",
    "MODELS_DIR",
    "ARTIFACTS_DIR",
    "LOGS_DIR",
    "RUNTIME_LOG_DIR",
    "AUDIT_LOG_DIR",
    "METRICS_PATH",
    "RESOURCE_LOG_PATH",
    "DRIFT_LOG_PATH",
    "SEMANTIC_BASELINE_PATH",
    "TOPIC_MODEL_PATH",
    "CORPUS_PATH",
]
