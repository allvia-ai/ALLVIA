"""Chunk/document cache helpers for incremental training."""

from __future__ import annotations

import json
import time
from dataclasses import dataclass, asdict
from pathlib import Path
import os
from typing import Dict, Iterable, List, Optional, Set, Tuple

import sqlite3

try:
    import pandas as pd
except Exception:  # pragma: no cover - optional
    pd = None  # type: ignore[assignment]


CACHE_MAX_ENTRIES_ENV = "INFOPILOT_CACHE_MAX_ENTRIES"


def _max_cache_entries() -> int:
    try:
        return max(0, int(os.getenv(CACHE_MAX_ENTRIES_ENV, "0") or 0))
    except ValueError:
        return 0


@dataclass
class CacheEntry:
    path: str
    doc_hash: str
    chunk_count: int
    updated_at: float


class ChunkCache:
    """Persist a lightweight mapping of document hashes for reuse/dedup."""

    def __init__(self, cache_path: Path) -> None:
        self.cache_path = cache_path
        self._entries: Dict[str, CacheEntry] = {}
        self._hash_index: Dict[str, str] = {}
        self._dirty = False
        self._max_entries = _max_cache_entries()
        self._load()

    def _load(self) -> None:
        if not self.cache_path.exists():
            return
        try:
            payload = json.loads(self.cache_path.read_text(encoding="utf-8"))
            if isinstance(payload, dict):
                for path, meta in payload.items():
                    if not isinstance(meta, dict):
                        continue
                    entry = CacheEntry(
                        path=path,
                        doc_hash=str(meta.get("doc_hash") or ""),
                        chunk_count=int(meta.get("chunk_count") or 0),
                        updated_at=float(meta.get("updated_at") or 0.0),
                    )
                    self._entries[path] = entry
                    if entry.doc_hash:
                        self._hash_index[entry.doc_hash] = path
        except Exception:
            # best-effort; corrupted cache will be rebuilt
            self._entries = {}
            self._hash_index = {}

    def mark_dirty(self) -> None:
        self._dirty = True

    def save(self) -> None:
        if not self._dirty:
            return
        self.cache_path.parent.mkdir(parents=True, exist_ok=True)
        serialized = {path: asdict(entry) for path, entry in self._entries.items()}
        
        # Atomic write: write to tmp then replace
        tmp_path = self.cache_path.with_suffix(".tmp")
        try:
            tmp_path.write_text(json.dumps(serialized, ensure_ascii=False, indent=2), encoding="utf-8")
            os.replace(tmp_path, self.cache_path)
            self._dirty = False
        except Exception:
            if tmp_path.exists():
                try:
                    tmp_path.unlink()
                except OSError:
                    pass
            raise

    def unchanged_paths(self, df: "pd.DataFrame") -> Set[str]:
        """Return paths whose doc_hash matches cached value."""
        if pd is None or df is None or df.empty or "path" not in df.columns:
            return set()
        if "doc_hash" not in df.columns:
            return set()
        unchanged: Set[str] = set()
        for path, doc_hash in zip(df["path"], df["doc_hash"]):
            key = str(path or "")
            cached = self._entries.get(key)
            if not key or not doc_hash or cached is None:
                continue
            if cached.doc_hash == str(doc_hash):
                unchanged.add(key)
        return unchanged

    def update_from_frame(self, df: "pd.DataFrame") -> None:
        """Refresh cache entries using the final corpus frame."""
        if pd is None or df is None or df.empty or "path" not in df.columns:
            return
        now = time.time()
        grouped = df.groupby("path", dropna=True)
        for path, group in grouped:
            doc_hash = ""
            if "doc_hash" in group.columns:
                doc_hash = str(group["doc_hash"].fillna("").iloc[0])
            if "chunk_count" in group.columns and not group["chunk_count"].isnull().all():
                chunk_count = int(group["chunk_count"].fillna(0).iloc[0])
            else:
                chunk_count = int(len(group))
            entry = CacheEntry(
                path=str(path),
                doc_hash=doc_hash,
                chunk_count=chunk_count,
                updated_at=now,
            )
            self._entries[str(path)] = entry
            if entry.doc_hash:
                self._hash_index[entry.doc_hash] = str(path)
            self._dirty = True
        self._prune_if_needed()

    def drop_paths(self, missing: Iterable[str]) -> None:
        removed = False
        for path in missing:
            entry = self._entries.pop(path, None)
            if entry and entry.doc_hash in self._hash_index:
                self._hash_index.pop(entry.doc_hash, None)
                removed = True
        if removed:
            self._dirty = True

    def known_paths(self) -> Set[str]:
        return set(self._entries.keys())

    def _prune_if_needed(self) -> None:
        limit = self._max_entries
        if limit <= 0 or len(self._entries) <= limit:
            return
        ordered = sorted(self._entries.values(), key=lambda entry: entry.updated_at, reverse=True)
        keep = {entry.path for entry in ordered[:limit]}
        for path in list(self._entries.keys()):
            if path not in keep:
                entry = self._entries.pop(path)
                if entry.doc_hash in self._hash_index:
                    self._hash_index.pop(entry.doc_hash, None)
                self._dirty = True


class SQLiteChunkCache:
    """SQLite-backed chunk cache for larger datasets."""

    def __init__(self, cache_path: Path) -> None:
        self.cache_path = cache_path
        self._conn = sqlite3.connect(str(cache_path))
        self._conn.execute("PRAGMA journal_mode=WAL;")
        self._conn.execute(
            """
            CREATE TABLE IF NOT EXISTS entries (
                path TEXT PRIMARY KEY,
                doc_hash TEXT,
                chunk_count INTEGER,
                updated_at REAL
            )
            """
        )
        self._conn.commit()
        self._max_entries = _max_cache_entries()

    def save(self) -> None:
        self._conn.commit()

    def _row_map(self) -> Dict[str, CacheEntry]:
        cursor = self._conn.execute("SELECT path, doc_hash, chunk_count, updated_at FROM entries")
        rows = cursor.fetchall()
        return {
            path: CacheEntry(path=path, doc_hash=doc_hash or "", chunk_count=chunk_count or 0, updated_at=updated_at or 0.0)
            for path, doc_hash, chunk_count, updated_at in rows
        }

    def unchanged_paths(self, df: "pd.DataFrame") -> Set[str]:
        if pd is None or df is None or df.empty or "path" not in df.columns or "doc_hash" not in df.columns:
            return set()
        existing = self._row_map()
        results: Set[str] = set()
        for path, doc_hash in zip(df["path"], df["doc_hash"]):
            key = str(path or "")
            if not key:
                continue
            cached = existing.get(key)
            if cached and cached.doc_hash and str(doc_hash) == cached.doc_hash:
                results.add(key)
        return results

    def update_from_frame(self, df: "pd.DataFrame") -> None:
        if pd is None or df is None or df.empty or "path" not in df.columns:
            return
        now = time.time()
        grouped = df.groupby("path", dropna=True)
        rows: List[Tuple[str, str, int, float]] = []
        for path, group in grouped:
            doc_hash = ""
            if "doc_hash" in group.columns:
                doc_hash = str(group["doc_hash"].fillna("").iloc[0])
            if "chunk_count" in group.columns and not group["chunk_count"].isnull().all():
                chunk_count = int(group["chunk_count"].fillna(0).iloc[0])
            else:
                chunk_count = int(len(group))
            rows.append((str(path), doc_hash, chunk_count, now))
        if not rows:
            return
        self._conn.executemany(
            """
            INSERT INTO entries(path, doc_hash, chunk_count, updated_at)
            VALUES(?, ?, ?, ?)
            ON CONFLICT(path) DO UPDATE SET
                doc_hash=excluded.doc_hash,
                chunk_count=excluded.chunk_count,
                updated_at=excluded.updated_at
            """,
            rows,
        )
        self._conn.commit()
        self._prune_if_needed()

    def drop_paths(self, missing: Iterable[str]) -> None:
        items = [str(path) for path in missing if path]
        if not items:
            return
        with self._conn:
            for chunk in [items[i : i + 200] for i in range(0, len(items), 200)]:
                placeholders = ", ".join("?" for _ in chunk)
                self._conn.execute(f"DELETE FROM entries WHERE path IN ({placeholders})", chunk)

    def known_paths(self) -> Set[str]:
        cursor = self._conn.execute("SELECT path FROM entries")
        return {row[0] for row in cursor.fetchall()}

    def _prune_if_needed(self) -> None:
        limit = self._max_entries
        if limit <= 0:
            return
        cur = self._conn.execute("SELECT COUNT(1) FROM entries")
        total = cur.fetchone()[0]
        if total <= limit:
            return
        remove = total - limit
        with self._conn:
            self._conn.execute(
                "DELETE FROM entries WHERE path IN (SELECT path FROM entries ORDER BY updated_at ASC LIMIT ?)",
                (remove,),
            )

    def close(self) -> None:
        try:
            self._conn.close()
        except Exception:
            pass

    def __del__(self) -> None:  # pragma: no cover - best-effort cleanup
        self.close()
