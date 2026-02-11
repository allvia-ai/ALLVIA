import json
import logging
import uuid
from datetime import datetime, timezone
from logging.handlers import RotatingFileHandler
from pathlib import Path
from typing import Any, Dict, Optional


class JsonFormatter(logging.Formatter):
    def __init__(self, run_id: str) -> None:
        super().__init__()
        self._run_id = run_id

    def format(self, record: logging.LogRecord) -> str:
        payload: Dict[str, Any] = {
            "ts": datetime.fromtimestamp(record.created)
            .astimezone()
            .strftime("%Y-%m-%d %H:%M:%S"),
            "level": record.levelname,
            "component": record.name,
            "run_id": self._run_id,
        }

        message = record.getMessage()
        parsed = _parse_json(message)
        if parsed is not None:
            event = parsed.get("event")
            if event:
                payload["event"] = event
            payload["meta"] = parsed
        else:
            payload["event"] = getattr(record, "event", None) or message
            meta = getattr(record, "meta", None)
            if meta is not None:
                payload["meta"] = meta

        if record.exc_info:
            payload["error"] = self.formatException(record.exc_info)

        return json.dumps(payload, separators=(",", ":"))


def setup_logging(
    level: str = "INFO",
    *,
    log_dir: Optional[Path] = None,
    log_file: str = "collector.log",
    max_mb: int = 20,
    backup_count: int = 10,
    use_json: bool = True,
    to_console: bool = True,
    run_id: Optional[str] = None,
) -> str:
    run_id = run_id or uuid.uuid4().hex
    root = logging.getLogger()
    root.setLevel(getattr(logging, level.upper(), logging.INFO))
    for handler in list(root.handlers):
        root.removeHandler(handler)

    if use_json:
        formatter: logging.Formatter = JsonFormatter(run_id)
    else:
        formatter = logging.Formatter("%(asctime)s %(levelname)s %(name)s %(message)s")

    if log_dir:
        log_dir.mkdir(parents=True, exist_ok=True)
        log_path = log_dir / log_file
        max_bytes = max(1, int(max_mb)) * 1024 * 1024
        file_handler = RotatingFileHandler(
            log_path, maxBytes=max_bytes, backupCount=max(1, int(backup_count))
        )
        file_handler.setFormatter(formatter)
        root.addHandler(file_handler)

    if to_console or not log_dir:
        console_handler = logging.StreamHandler()
        console_handler.setFormatter(formatter)
        root.addHandler(console_handler)

    return run_id


def _parse_json(message: str) -> Optional[Dict[str, Any]]:
    if not message:
        return None
    try:
        parsed = json.loads(message)
    except (TypeError, ValueError, json.JSONDecodeError):
        return None
    if isinstance(parsed, dict):
        return parsed
    return None
