from __future__ import annotations

from dataclasses import dataclass, field
import os
from pathlib import Path
from typing import Any, Dict

import yaml

PROJECT_ROOT = Path(__file__).resolve().parents[2]


@dataclass
class IngestConfig:
    enabled: bool = True
    host: str = "127.0.0.1"
    port: int = 8080
    token: str = ""


@dataclass
class QueueConfig:
    max_size: int = 1000
    shutdown_drain_seconds: int = 3


@dataclass
class PrivacyConfig:
    hash_salt: str = "dev-salt"


@dataclass
class StoreConfig:
    busy_timeout_ms: int = 5000
    insert_batch_size: int = 100
    insert_flush_ms: int = 1000
    insert_retry_attempts: int = 3
    insert_retry_backoff_ms: int = 50


@dataclass
class PriorityConfig:
    debounce_seconds: float = 2.0
    focus_event_types: list[str] = field(default_factory=lambda: ["os.foreground_changed"])
    focus_block_event_type: str = "os.app_focus_block"
    drop_p2_when_queue_over: float = 0.8


@dataclass
class RetentionConfig:
    enabled: bool = True
    interval_minutes: int = 60
    raw_events_days: int = 7
    sessions_days: int = 30
    routine_candidates_days: int = 90
    handoff_queue_days: int = 7
    max_db_mb: int = 500
    batch_size: int = 5000
    vacuum_hours: int = 24


@dataclass
class ObservabilityConfig:
    log_interval_sec: int = 60
    activity_log: bool = True
    activity_top_n: int = 3
    activity_min_duration_sec: int = 5
    activity_include_title: bool = False


@dataclass
class LoggingConfig:
    dir: Path = PROJECT_ROOT / "logs"
    file_name: str = "collector.log"
    max_mb: int = 20
    backup_count: int = 10
    json: bool = True
    to_console: bool = True


@dataclass
class Config:
    db_path: Path
    migrations_path: Path
    validation_level: str = "lenient"
    wal_mode: bool = True
    log_level: str = "INFO"
    privacy_rules_path: Path = PROJECT_ROOT / "configs" / "privacy_rules.yaml"
    ingest: IngestConfig = field(default_factory=IngestConfig)
    queue: QueueConfig = field(default_factory=QueueConfig)
    privacy: PrivacyConfig = field(default_factory=PrivacyConfig)
    store: StoreConfig = field(default_factory=StoreConfig)
    priority: PriorityConfig = field(default_factory=PriorityConfig)
    retention: RetentionConfig = field(default_factory=RetentionConfig)
    observability: ObservabilityConfig = field(default_factory=ObservabilityConfig)
    logging: LoggingConfig = field(default_factory=LoggingConfig)


def load_config(path: str | Path) -> Config:
    config_path = Path(path)
    if not config_path.exists():
        raise FileNotFoundError(f"config file not found: {config_path}")

    raw = yaml.safe_load(config_path.read_text()) or {}
    if not isinstance(raw, dict):
        raise ValueError("config root must be a mapping")

    db_path = _resolve_db_path(raw.get("db_path", "collector.db"))
    migrations_path = _resolve_path(raw.get("migrations_path", "migrations"))
    privacy_rules_path = _resolve_path(
        raw.get("privacy_rules_path", "configs/privacy_rules.yaml")
    )

    ingest_raw = _as_dict(raw.get("ingest"))
    token_value = ingest_raw.get("token", "")
    ingest = IngestConfig(
        enabled=bool(ingest_raw.get("enabled", True)),
        host=str(ingest_raw.get("host", "127.0.0.1")),
        port=int(ingest_raw.get("port", 8080)),
        token=str(token_value) if token_value is not None else "",
    )

    queue_raw = _as_dict(raw.get("queue"))
    queue = QueueConfig(
        max_size=int(queue_raw.get("max_size", 1000)),
        shutdown_drain_seconds=int(queue_raw.get("shutdown_drain_seconds", 3)),
    )

    privacy_raw = _as_dict(raw.get("privacy"))
    privacy = PrivacyConfig(hash_salt=str(privacy_raw.get("hash_salt", "dev-salt")))

    store_raw = _as_dict(raw.get("store"))
    store = StoreConfig(
        busy_timeout_ms=int(store_raw.get("busy_timeout_ms", 5000)),
        insert_batch_size=int(store_raw.get("insert_batch_size", 100)),
        insert_flush_ms=int(store_raw.get("insert_flush_ms", 1000)),
        insert_retry_attempts=int(store_raw.get("insert_retry_attempts", 3)),
        insert_retry_backoff_ms=int(store_raw.get("insert_retry_backoff_ms", 50)),
    )

    priority_raw = _as_dict(raw.get("priority"))
    focus_event_types = priority_raw.get("focus_event_types", ["os.foreground_changed"])
    if not isinstance(focus_event_types, list):
        focus_event_types = [str(focus_event_types)]
    priority = PriorityConfig(
        debounce_seconds=float(priority_raw.get("debounce_seconds", 2.0)),
        focus_event_types=[str(item) for item in focus_event_types],
        focus_block_event_type=str(
            priority_raw.get("focus_block_event_type", "os.app_focus_block")
        ),
        drop_p2_when_queue_over=float(
            priority_raw.get("drop_p2_when_queue_over", 0.8)
        ),
    )

    retention_raw = _as_dict(raw.get("retention"))
    retention = RetentionConfig(
        enabled=bool(retention_raw.get("enabled", True)),
        interval_minutes=int(retention_raw.get("interval_minutes", 60)),
        raw_events_days=int(retention_raw.get("raw_events_days", 7)),
        sessions_days=int(retention_raw.get("sessions_days", 30)),
        routine_candidates_days=int(retention_raw.get("routine_candidates_days", 90)),
        handoff_queue_days=int(retention_raw.get("handoff_queue_days", 7)),
        max_db_mb=int(retention_raw.get("max_db_mb", 500)),
        batch_size=int(retention_raw.get("batch_size", 5000)),
        vacuum_hours=int(retention_raw.get("vacuum_hours", 24)),
    )

    observability_raw = _as_dict(raw.get("observability"))
    observability = ObservabilityConfig(
        log_interval_sec=int(observability_raw.get("log_interval_sec", 60)),
        activity_log=bool(observability_raw.get("activity_log", True)),
        activity_top_n=int(observability_raw.get("activity_top_n", 3)),
        activity_min_duration_sec=int(
            observability_raw.get("activity_min_duration_sec", 5)
        ),
        activity_include_title=bool(
            observability_raw.get("activity_include_title", False)
        ),
    )

    logging_raw = _as_dict(raw.get("logging"))
    logging_config = LoggingConfig(
        dir=_resolve_path(logging_raw.get("dir", "logs")),
        file_name=str(logging_raw.get("file_name", "collector.log")),
        max_mb=int(logging_raw.get("max_mb", 20)),
        backup_count=int(logging_raw.get("backup_count", 10)),
        json=bool(logging_raw.get("json", True)),
        to_console=bool(logging_raw.get("to_console", True)),
    )

    return Config(
        db_path=db_path,
        migrations_path=migrations_path,
        validation_level=str(raw.get("validation_level", "lenient")),
        wal_mode=bool(raw.get("wal_mode", True)),
        log_level=str(raw.get("log_level", "INFO")),
        privacy_rules_path=privacy_rules_path,
        ingest=ingest,
        queue=queue,
        privacy=privacy,
        store=store,
        priority=priority,
        retention=retention,
        observability=observability,
        logging=logging_config,
    )


def _resolve_path(value: str) -> Path:
    path = Path(value)
    if path.is_absolute():
        return path
    return PROJECT_ROOT / path


def _resolve_db_path(value: Any) -> Path:
    raw_value = str(value).strip() if value is not None else "collector.db"
    if not raw_value:
        raw_value = "collector.db"

    def _env_bool(key: str, default: bool) -> bool:
        raw = os.environ.get(key)
        if raw is None:
            return default
        return raw.strip().lower() in {"1", "true", "yes", "on"}

    # Explicit integration mode:
    # Use shared DB only when env override is explicitly provided.
    env_override = os.environ.get("STEER_COLLECTOR_DB_PATH") or os.environ.get("STEER_DB_PATH")
    if env_override and raw_value in {"collector.db", "./collector.db"}:
        return _resolve_path(env_override)

    # Explicit separation mode:
    # keep collector.db isolated even when a shared steer.db exists.
    if _env_bool("STEER_COLLECTOR_SEPARATE_DB", False):
        return _resolve_path(raw_value)

    # Auto-link mode defaults to ON so collector follows core DB by default.
    # Set STEER_COLLECTOR_AUTO_LINK=0 to disable auto-linking.
    auto_link = _env_bool("STEER_COLLECTOR_AUTO_LINK", True)
    if auto_link and raw_value in {"collector.db", "./collector.db"}:
        shared_path = _discover_shared_steer_db()
        if shared_path is not None:
            return shared_path

    return _resolve_path(raw_value)


def _discover_shared_steer_db() -> Path | None:
    candidates: list[Path] = []

    home = Path.home()
    # macOS default for dirs::data_local_dir()
    candidates.append(home / "Library" / "Application Support" / "steer" / "steer.db")
    # Linux default for dirs::data_local_dir()
    candidates.append(home / ".local" / "share" / "steer" / "steer.db")

    # Repo-local fallbacks (when DB is forced into workspace paths).
    here = Path(__file__).resolve()
    for idx in range(2, min(7, len(here.parents))):
        base = here.parents[idx]
        candidates.append(base / "steer.db")
        candidates.append(base / "core" / "steer.db")

    for path in candidates:
        try:
            if path.exists() and path.is_file():
                return path
        except OSError:
            continue

    return None


def _as_dict(value: Any) -> Dict[str, Any]:
    if isinstance(value, dict):
        return value
    return {}
