from __future__ import annotations

import argparse
import json
import logging
import signal
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Any, Dict, List

from .bus import EventBus
from .config import load_config
from .logging_ import setup_logging
from .observability import Observability
from .privacy import PrivacyGuard, load_privacy_rules
from .priority import PriorityProcessor
from .retention import retention_result_json, run_retention
from .store import SQLiteStore

logger = logging.getLogger(__name__)


class IngestServer(ThreadingHTTPServer):
    allow_reuse_address = True
    daemon_threads = True

    def __init__(
        self,
        server_address,
        handler,
        bus: EventBus,
        ingest_config,
        metrics: Observability,
        db_path,
    ):
        super().__init__(server_address, handler)
        self.bus = bus
        self.ingest_config = ingest_config
        self.metrics = metrics
        self.db_path = db_path


class IngestHandler(BaseHTTPRequestHandler):
    server_version = "CollectorHTTP/0.1"

    def do_GET(self) -> None:
        if self.path != "/health":
            if self.path == "/stats":
                self._handle_stats()
                return
            self._send_json(404, {"error": "not found"})
            return
        self._send_json(200, {"ok": True})

    def do_OPTIONS(self) -> None:
        self.send_response(204)
        self._set_cors_headers()
        self.end_headers()

    def do_POST(self) -> None:
        if self.path != "/events":
            self._send_json(404, {"error": "not found"})
            return
        if not self._check_token():
            return
        metrics = getattr(self.server, "metrics", None)
        content_length = self.headers.get("Content-Length")
        if not content_length:
            if metrics:
                metrics.record_ingest_invalid()
            self._send_json(411, {"error": "missing content-length"})
            return
        try:
            body = self.rfile.read(int(content_length))
            payload = json.loads(body)
        except (ValueError, json.JSONDecodeError):
            if metrics:
                metrics.record_ingest_invalid()
            self._send_json(400, {"error": "invalid json"})
            return

        events = _normalize_payload(payload)
        if events is None:
            if metrics:
                metrics.record_ingest_invalid()
            self._send_json(400, {"error": "payload must be object or list"})
            return

        for event in events:
            if not isinstance(event, dict):
                if metrics:
                    metrics.record_ingest_invalid()
                self._send_json(400, {"error": "event must be object"})
                return

        queued = 0
        if metrics:
            metrics.inc("ingest.received_total", len(events))
        for event in events:
            if not self.server.bus.enqueue(event):
                if metrics:
                    metrics.record_drop("queue_full")
                self._send_json(429, {"error": "queue full", "queued": queued})
                return
            queued += 1

        if metrics:
            metrics.inc("ingest.ok_total", queued)
        self._send_json(200, {"status": "queued", "count": queued})

    def log_message(self, format: str, *args: Any) -> None:
        logger.info("%s - %s", self.address_string(), format % args)

    def _send_json(self, status: int, payload: Dict[str, Any]) -> None:
        body = json.dumps(payload, separators=(",", ":")).encode("utf-8")
        self.send_response(status)
        self._set_cors_headers()
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def _set_cors_headers(self) -> None:
        self.send_header("Access-Control-Allow-Origin", "*")
        self.send_header("Access-Control-Allow-Methods", "POST, OPTIONS")
        self.send_header("Access-Control-Allow-Headers", "Content-Type, X-Collector-Token")

    def _check_token(self) -> bool:
        ingest_config = getattr(self.server, "ingest_config", None)
        token = getattr(ingest_config, "token", "")
        if not token:
            return True
        provided = self.headers.get("X-Collector-Token")
        if provided != token:
            self._send_json(401, {"error": "unauthorized"})
            return False
        return True

    def _handle_stats(self) -> None:
        metrics = getattr(self.server, "metrics", None)
        db_path = getattr(self.server, "db_path", None)
        if metrics is None:
            self._send_json(503, {"error": "metrics disabled"})
            return
        db_size = 0
        if db_path and db_path.exists():
            db_size = db_path.stat().st_size
        payload = metrics.snapshot(db_size)
        self._send_json(200, payload)


def _normalize_payload(payload: Any) -> List[Dict[str, Any]] | None:
    if isinstance(payload, dict):
        return [payload]
    if isinstance(payload, list):
        return payload
    return None


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Data Collector Core")
    parser.add_argument(
        "--config", default="configs/config.yaml", help="path to config file"
    )
    return parser.parse_args()


def run() -> None:
    args = parse_args()
    config = load_config(args.config)
    setup_logging(
        config.log_level,
        log_dir=config.logging.dir,
        log_file=config.logging.file_name,
        max_mb=config.logging.max_mb,
        backup_count=config.logging.backup_count,
        use_json=config.logging.json,
        to_console=config.logging.to_console,
    )

    logger.info("starting collector")

    store = SQLiteStore(
        config.db_path,
        wal_mode=config.wal_mode,
        busy_timeout_ms=config.store.busy_timeout_ms,
    )
    store.connect()
    store.migrate(config.migrations_path)

    metrics = Observability(
        log_interval_sec=config.observability.log_interval_sec,
        activity_log=config.observability.activity_log,
        activity_top_n=config.observability.activity_top_n,
        activity_min_duration_sec=config.observability.activity_min_duration_sec,
        activity_include_title=config.observability.activity_include_title,
    )

    privacy_rules = load_privacy_rules(config.privacy_rules_path)
    privacy_guard = PrivacyGuard(privacy_rules, config.privacy.hash_salt, metrics=metrics)

    priority = PriorityProcessor(
        debounce_seconds=config.priority.debounce_seconds,
        focus_event_types=config.priority.focus_event_types,
        focus_block_event_type=config.priority.focus_block_event_type,
        drop_p2_when_queue_over=config.priority.drop_p2_when_queue_over,
        metrics=metrics,
    )

    bus = EventBus(
        store,
        privacy_guard,
        priority,
        validation_level=config.validation_level,
        queue_size=config.queue.max_size,
        insert_batch_size=config.store.insert_batch_size,
        insert_flush_ms=config.store.insert_flush_ms,
        insert_retry_attempts=config.store.insert_retry_attempts,
        insert_retry_backoff_ms=config.store.insert_retry_backoff_ms,
        metrics=metrics,
    )
    bus.start()

    server: IngestServer | None = None
    server_thread: threading.Thread | None = None

    if config.ingest.enabled:
        server = IngestServer(
            (config.ingest.host, config.ingest.port),
            IngestHandler,
            bus,
            config.ingest,
            metrics,
            config.db_path,
        )
        server_thread = threading.Thread(target=server.serve_forever, daemon=True)
        server_thread.start()
        logger.info(
            "ingest listening on http://%s:%s",
            config.ingest.host,
            config.ingest.port,
        )
    else:
        logger.info("ingest disabled")

    stop_event = threading.Event()
    retention_thread: threading.Thread | None = None

    if config.retention.enabled:
        vacuum_seconds = max(0, config.retention.vacuum_hours) * 3600
        last_vacuum = 0.0

        def _retention_loop() -> None:
            nonlocal last_vacuum
            while not stop_event.is_set():
                try:
                    force_vacuum = False
                    if vacuum_seconds > 0:
                        now_ts = time.time()
                        if last_vacuum <= 0 or (now_ts - last_vacuum) >= vacuum_seconds:
                            force_vacuum = True
                    result = run_retention(
                        store, config.retention, force_vacuum=force_vacuum
                    )
                    if result.vacuumed:
                        last_vacuum = time.time()
                    logger.info(retention_result_json(result))
                except Exception:
                    logger.exception("retention failed")
                stop_event.wait(config.retention.interval_minutes * 60)

        retention_thread = threading.Thread(target=_retention_loop, daemon=True)
        retention_thread.start()

    def _handle_signal(signum, frame):
        logger.info("shutdown requested")
        stop_event.set()
        if server is not None:
            server.shutdown()

    signal.signal(signal.SIGINT, _handle_signal)
    signal.signal(signal.SIGTERM, _handle_signal)

    try:
        heartbeat_interval = max(5, config.observability.log_interval_sec // 2)

        def _metrics_loop() -> None:
            while not stop_event.is_set():
                try:
                    metrics.maybe_log(logger, store.get_db_size())
                except Exception:
                    logger.exception("metrics heartbeat failed")
                stop_event.wait(heartbeat_interval)

        metrics_thread = threading.Thread(target=_metrics_loop, daemon=True)
        metrics_thread.start()

        while not stop_event.is_set():
            time.sleep(0.25)
    finally:
        if server is not None:
            server.shutdown()
            server.server_close()
        if server_thread is not None:
            server_thread.join(timeout=5)
        bus.stop(drain_seconds=config.queue.shutdown_drain_seconds)
        if retention_thread is not None:
            retention_thread.join(timeout=5)
        store.close()
        logger.info("collector stopped")


if __name__ == "__main__":
    run()
