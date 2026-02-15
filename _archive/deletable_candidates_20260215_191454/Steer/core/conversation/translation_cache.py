"""On-demand translation cache backed by SQLite."""
from __future__ import annotations

import hashlib
import sqlite3
import threading
import time
from pathlib import Path
from typing import Optional


class TranslationCache:
    """Simple persistent cache for translated snippets.

    The cache is keyed by `(source_fingerprint, target_lang)` where
    `source_fingerprint` is a hash of the original text combined with the
    document path. This keeps the schema agnostic to document IDs while
    remaining stable across runs.
    """

    def __init__(self, db_path: Path) -> None:
        self.db_path = Path(db_path)
        self.db_path.parent.mkdir(parents=True, exist_ok=True)
        self._lock = threading.Lock()
        self._init_db()

    def _connect(self) -> sqlite3.Connection:
        conn = sqlite3.connect(self.db_path, timeout=5.0, check_same_thread=False)
        conn.execute("PRAGMA journal_mode=WAL")
        conn.execute("PRAGMA synchronous=NORMAL")
        conn.execute("PRAGMA temp_store=MEMORY")
        conn.execute("PRAGMA busy_timeout=3000")
        return conn

    # ------------------------------------------------------------------
    # Public API
    # ------------------------------------------------------------------
    def get(self, doc_path: str, original_text: str, target_lang: str) -> Optional[str]:
        key = self._fingerprint(doc_path, original_text)
        with self._lock, self._connect() as conn:
            cur = conn.execute(
                "SELECT translated_text FROM translations WHERE source_key=? AND target_lang=?",
                (key, target_lang),
            )
            row = cur.fetchone()
        if row:
            return row[0]
        return None

    def set(self, doc_path: str, original_text: str, target_lang: str, translated_text: str) -> None:
        key = self._fingerprint(doc_path, original_text)
        timestamp = int(time.time())
        with self._lock, self._connect() as conn:
            conn.execute(
                """
                INSERT INTO translations(source_key, target_lang, translated_text, updated_at)
                VALUES (?, ?, ?, ?)
                ON CONFLICT(source_key, target_lang)
                DO UPDATE SET translated_text=excluded.translated_text,
                               updated_at=excluded.updated_at
                """,
                (key, target_lang, translated_text, timestamp),
            )
            conn.commit()

    # ------------------------------------------------------------------
    # Internal helpers
    # ------------------------------------------------------------------
    def _init_db(self) -> None:
        with self._lock, self._connect() as conn:
            conn.execute(
                """
                CREATE TABLE IF NOT EXISTS translations (
                    source_key TEXT NOT NULL,
                    target_lang TEXT NOT NULL,
                    translated_text TEXT NOT NULL,
                    updated_at INTEGER NOT NULL,
                    PRIMARY KEY (source_key, target_lang)
                )
                """,
            )
            conn.commit()

    @staticmethod
    def _fingerprint(doc_path: str, original_text: str) -> str:
        digest = hashlib.sha1()
        digest.update((doc_path or "").encode("utf-8"))
        digest.update(b"\x00")
        digest.update((original_text or "").encode("utf-8"))
        return digest.hexdigest()


__all__ = ["TranslationCache"]
