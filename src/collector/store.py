from __future__ import annotations

import json
import sqlite3
import threading
import time
from pathlib import Path
from typing import Optional

from .models import EventEnvelope


class SQLiteStore:
    def __init__(
        self,
        db_path: Path,
        wal_mode: bool = True,
        busy_timeout_ms: int = 5000,
    ) -> None:
        self.db_path = Path(db_path)
        self.wal_mode = wal_mode
        self.busy_timeout_ms = max(0, int(busy_timeout_ms))
        self._conn: Optional[sqlite3.Connection] = None
        self._lock = threading.Lock()

    def connect(self) -> None:
        self.db_path.parent.mkdir(parents=True, exist_ok=True)
        self._conn = sqlite3.connect(self.db_path, check_same_thread=False)
        self._conn.execute("PRAGMA foreign_keys = ON;")
        if self.busy_timeout_ms:
            self._conn.execute(f"PRAGMA busy_timeout = {self.busy_timeout_ms};")
        if self.wal_mode:
            self._conn.execute("PRAGMA journal_mode = WAL;")

    def migrate(self, migrations_path: Path) -> None:
        if self._conn is None:
            raise RuntimeError("database is not connected")
        migrations_path = Path(migrations_path)
        sql_files = sorted(migrations_path.glob("*.sql"))
        if not sql_files:
            raise FileNotFoundError(f"no migrations found in {migrations_path}")
        with self._lock:
            for sql_path in sql_files:
                sql = sql_path.read_text()
                self._conn.executescript(sql)
            self._conn.commit()

    def insert_event(self, envelope: EventEnvelope) -> None:
        self.insert_events([envelope])

    def insert_events(
        self,
        envelopes: list[EventEnvelope],
        *,
        retry_attempts: int = 3,
        retry_backoff_ms: int = 50,
    ) -> None:
        if self._conn is None:
            raise RuntimeError("database is not connected")
        if not envelopes:
            return
        rows = []
        for envelope in envelopes:
            payload_json = json.dumps(envelope.payload, separators=(",", ":"))
            privacy_json = json.dumps(envelope.privacy.__dict__, separators=(",", ":"))
            raw_json = json.dumps(envelope.raw or {}, separators=(",", ":"))
            rows.append(
                (
                    envelope.schema_version,
                    envelope.event_id,
                    envelope.ts,
                    envelope.source,
                    envelope.app,
                    envelope.event_type,
                    envelope.priority,
                    envelope.resource.type,
                    envelope.resource.id,
                    payload_json,
                    privacy_json,
                    envelope.pid,
                    envelope.window_id,
                    raw_json,
                )
            )
        attempts = max(0, int(retry_attempts))
        backoff_ms = max(0, int(retry_backoff_ms))
        for attempt in range(attempts + 1):
            try:
                with self._lock:
                    self._conn.executemany(
                        """
                        INSERT INTO events (
                            schema_version,
                            event_id,
                            ts,
                            source,
                            app,
                            event_type,
                            priority,
                            resource_type,
                            resource_id,
                            payload_json,
                            privacy_json,
                            pid,
                            window_id,
                            raw_json
                        )
                        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                        """,
                        rows,
                    )
                    self._conn.commit()
                return
            except sqlite3.OperationalError as exc:
                if "database is locked" not in str(exc).lower():
                    raise
                if attempt >= attempts:
                    raise
                sleep_for = (backoff_ms / 1000.0) * (2**attempt)
                time.sleep(sleep_for)

    def insert_session(
        self,
        session_id: str,
        start_ts: str,
        end_ts: str,
        duration_sec: int,
        summary_json: str,
    ) -> None:
        if self._conn is None:
            raise RuntimeError("database is not connected")
        with self._lock:
            self._conn.execute(
                """
                INSERT INTO sessions (
                    session_id,
                    start_ts,
                    end_ts,
                    duration_sec,
                    summary_json
                )
                VALUES (?, ?, ?, ?, ?)
                """,
                (session_id, start_ts, end_ts, duration_sec, summary_json),
            )
            self._conn.commit()

    def clear_routine_candidates(self) -> None:
        if self._conn is None:
            raise RuntimeError("database is not connected")
        with self._lock:
            self._conn.execute("DELETE FROM routine_candidates")
            self._conn.commit()

    def insert_routine_candidate(
        self,
        pattern_id: str,
        pattern_json: str,
        support: int,
        confidence: float,
        last_seen_ts: str,
        evidence_session_ids: str,
    ) -> None:
        if self._conn is None:
            raise RuntimeError("database is not connected")
        with self._lock:
            self._conn.execute(
                """
                INSERT INTO routine_candidates (
                    pattern_id,
                    pattern_json,
                    support,
                    confidence,
                    last_seen_ts,
                    evidence_session_ids
                )
                VALUES (?, ?, ?, ?, ?, ?)
                """,
                (
                    pattern_id,
                    pattern_json,
                    support,
                    confidence,
                    last_seen_ts,
                    evidence_session_ids,
                ),
            )
            self._conn.commit()

    def fetch_events(
        self,
        start_ts: Optional[str] = None,
        end_ts: Optional[str] = None,
    ) -> list[tuple]:
        if self._conn is None:
            raise RuntimeError("database is not connected")
        query = (
            "SELECT ts, event_type, priority, app, resource_type, resource_id, payload_json "
            "FROM events"
        )
        params: list[str] = []
        clauses: list[str] = []
        if start_ts:
            clauses.append("ts >= ?")
            params.append(start_ts)
        if end_ts:
            clauses.append("ts <= ?")
            params.append(end_ts)
        if clauses:
            query += " WHERE " + " AND ".join(clauses)
        query += " ORDER BY ts ASC"
        with self._lock:
            return list(self._conn.execute(query, params))

    def fetch_sessions(
        self,
        start_ts: Optional[str] = None,
        end_ts: Optional[str] = None,
    ) -> list[tuple]:
        if self._conn is None:
            raise RuntimeError("database is not connected")
        query = "SELECT session_id, start_ts, end_ts, summary_json FROM sessions"
        params: list[str] = []
        clauses: list[str] = []
        if start_ts:
            clauses.append("start_ts >= ?")
            params.append(start_ts)
        if end_ts:
            clauses.append("end_ts <= ?")
            params.append(end_ts)
        if clauses:
            query += " WHERE " + " AND ".join(clauses)
        query += " ORDER BY start_ts ASC"
        with self._lock:
            return list(self._conn.execute(query, params))

    def fetch_recent_sessions(self, limit: int = 3) -> list[tuple]:
        if self._conn is None:
            raise RuntimeError("database is not connected")
        query = (
            "SELECT session_id, start_ts, end_ts, duration_sec, summary_json "
            "FROM sessions ORDER BY start_ts DESC LIMIT ?"
        )
        with self._lock:
            return list(self._conn.execute(query, (limit,)))

    def fetch_routine_candidates(self, limit: int = 10) -> list[tuple]:
        if self._conn is None:
            raise RuntimeError("database is not connected")
        query = (
            "SELECT pattern_id, pattern_json, support, confidence, last_seen_ts, "
            "evidence_session_ids FROM routine_candidates "
            "ORDER BY support DESC, confidence DESC LIMIT ?"
        )
        with self._lock:
            return list(self._conn.execute(query, (limit,)))

    def fetch_latest_event(self) -> Optional[tuple]:
        if self._conn is None:
            raise RuntimeError("database is not connected")
        query = (
            "SELECT ts, event_type, priority, app, payload_json "
            "FROM events ORDER BY ts DESC LIMIT 1"
        )
        with self._lock:
            return self._conn.execute(query).fetchone()

    def fetch_latest_session_end_ts(self) -> Optional[str]:
        if self._conn is None:
            raise RuntimeError("database is not connected")
        query = "SELECT end_ts FROM sessions ORDER BY end_ts DESC LIMIT 1"
        with self._lock:
            row = self._conn.execute(query).fetchone()
        return row[0] if row else None

    def fetch_recent_privacy(self, limit: int = 200) -> list[tuple]:
        if self._conn is None:
            raise RuntimeError("database is not connected")
        query = "SELECT privacy_json FROM events ORDER BY ts DESC LIMIT ?"
        with self._lock:
            return list(self._conn.execute(query, (limit,)))

    def has_recent_p0(self, since_ts: str) -> bool:
        if self._conn is None:
            raise RuntimeError("database is not connected")
        query = "SELECT 1 FROM events WHERE priority = 'P0' AND ts >= ? LIMIT 1"
        with self._lock:
            return self._conn.execute(query, (since_ts,)).fetchone() is not None

    def enqueue_handoff(
        self,
        package_id: str,
        created_at: str,
        status: str,
        payload_json: str,
        payload_size: int,
        expires_at: Optional[str] = None,
        error: Optional[str] = None,
    ) -> None:
        if self._conn is None:
            raise RuntimeError("database is not connected")
        with self._lock:
            self._conn.execute(
                """
                INSERT INTO handoff_queue (
                    package_id,
                    created_at,
                    status,
                    payload_json,
                    payload_size,
                    expires_at,
                    error
                )
                VALUES (?, ?, ?, ?, ?, ?, ?)
                """,
                (
                    package_id,
                    created_at,
                    status,
                    payload_json,
                    payload_size,
                    expires_at,
                    error,
                ),
            )
            self._conn.commit()

    def fetch_latest_handoff(self, status: str = "pending") -> Optional[tuple]:
        if self._conn is None:
            raise RuntimeError("database is not connected")
        query = (
            "SELECT id, payload_json FROM handoff_queue "
            "WHERE status = ? ORDER BY created_at DESC LIMIT 1"
        )
        with self._lock:
            return self._conn.execute(query, (status,)).fetchone()

    def clear_pending_handoff(self) -> None:
        if self._conn is None:
            raise RuntimeError("database is not connected")
        with self._lock:
            self._conn.execute("DELETE FROM handoff_queue WHERE status = 'pending'")
            self._conn.commit()

    def mark_handoff_status(
        self, handoff_id: int, status: str, error: Optional[str] = None
    ) -> None:
        if self._conn is None:
            raise RuntimeError("database is not connected")
        with self._lock:
            self._conn.execute(
                "UPDATE handoff_queue SET status = ?, error = ? WHERE id = ?",
                (status, error, handoff_id),
            )
            self._conn.commit()

    def get_state(self, key: str) -> Optional[str]:
        if self._conn is None:
            raise RuntimeError("database is not connected")
        with self._lock:
            row = self._conn.execute(
                "SELECT value FROM state WHERE key = ?",
                (key,),
            ).fetchone()
        return row[0] if row else None

    def set_state(self, key: str, value: str) -> None:
        if self._conn is None:
            raise RuntimeError("database is not connected")
        with self._lock:
            self._conn.execute(
                """
                INSERT INTO state (key, value, updated_at)
                VALUES (?, ?, datetime('now'))
                ON CONFLICT(key) DO UPDATE SET
                    value = excluded.value,
                    updated_at = excluded.updated_at
                """,
                (key, value),
            )
            self._conn.commit()

    def close(self) -> None:
        if self._conn is None:
            return
        with self._lock:
            self._conn.close()
        self._conn = None

    def get_db_size(self) -> int:
        return int(self.db_path.stat().st_size) if self.db_path.exists() else 0

    def checkpoint_wal(self) -> None:
        if self._conn is None:
            raise RuntimeError("database is not connected")
        with self._lock:
            self._conn.execute("PRAGMA wal_checkpoint(TRUNCATE);")

    def vacuum(self) -> None:
        if self._conn is None:
            raise RuntimeError("database is not connected")
        with self._lock:
            self._conn.execute("VACUUM;")

    def delete_old_events(self, cutoff_ts: str, batch_size: int = 0) -> int:
        return self._delete_by_cutoff(
            "events", "ts", cutoff_ts, batch_size=batch_size
        )

    def delete_old_sessions(self, cutoff_ts: str, batch_size: int = 0) -> int:
        return self._delete_by_cutoff(
            "sessions", "end_ts", cutoff_ts, batch_size=batch_size
        )

    def delete_old_routines(self, cutoff_ts: str, batch_size: int = 0) -> int:
        return self._delete_by_cutoff(
            "routine_candidates", "last_seen_ts", cutoff_ts, batch_size=batch_size
        )

    def expire_pending_handoff(self, cutoff_ts: str) -> int:
        if self._conn is None:
            raise RuntimeError("database is not connected")
        with self._lock:
            self._conn.execute(
                "UPDATE handoff_queue SET status = 'expired' "
                "WHERE status = 'pending' AND created_at < ?",
                (cutoff_ts,),
            )
            changes = self._conn.execute("SELECT changes()").fetchone()[0]
            self._conn.commit()
            return changes

    def delete_old_handoff(self, cutoff_ts: str, batch_size: int = 0) -> int:
        return self._delete_by_cutoff(
            "handoff_queue", "created_at", cutoff_ts, batch_size=batch_size
        )

    def _delete_by_cutoff(
        self, table: str, ts_column: str, cutoff_ts: str, batch_size: int = 0
    ) -> int:
        if self._conn is None:
            raise RuntimeError("database is not connected")
        total = 0
        with self._lock:
            if batch_size and batch_size > 0:
                while True:
                    self._conn.execute(
                        f"DELETE FROM {table} WHERE rowid IN ("
                        f"SELECT rowid FROM {table} WHERE {ts_column} < ? LIMIT ?"
                        f")",
                        (cutoff_ts, batch_size),
                    )
                    removed = self._conn.execute("SELECT changes()").fetchone()[0]
                    total += removed
                    if removed < batch_size:
                        break
            else:
                self._conn.execute(
                    f"DELETE FROM {table} WHERE {ts_column} < ?",
                    (cutoff_ts,),
                )
                total = self._conn.execute("SELECT changes()").fetchone()[0]
            self._conn.commit()
        return total
