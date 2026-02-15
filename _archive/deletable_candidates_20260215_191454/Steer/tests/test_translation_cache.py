from __future__ import annotations

import sqlite3

from core.conversation.translation_cache import TranslationCache


def test_translation_cache_roundtrip(tmp_path) -> None:
    db_path = tmp_path / "translations.sqlite3"
    cache = TranslationCache(db_path)
    cache.set("docs/a.md", "hello", "en", "hello")
    assert cache.get("docs/a.md", "hello", "en") == "hello"

    with sqlite3.connect(db_path) as conn:
        mode = conn.execute("PRAGMA journal_mode").fetchone()[0]
    assert str(mode).lower() == "wal"

