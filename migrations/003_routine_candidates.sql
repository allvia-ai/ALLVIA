CREATE TABLE IF NOT EXISTS routine_candidates (
    pattern_id TEXT PRIMARY KEY,
    pattern_json TEXT NOT NULL,
    support INTEGER NOT NULL,
    confidence REAL NOT NULL,
    last_seen_ts TEXT NOT NULL,
    evidence_session_ids TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_routines_support ON routine_candidates (support);
CREATE INDEX IF NOT EXISTS idx_routines_last_seen ON routine_candidates (last_seen_ts);
