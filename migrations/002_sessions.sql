CREATE TABLE IF NOT EXISTS sessions (
    session_id TEXT PRIMARY KEY,
    start_ts TEXT NOT NULL,
    end_ts TEXT NOT NULL,
    duration_sec INTEGER NOT NULL,
    summary_json TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_sessions_start_ts ON sessions (start_ts);
CREATE INDEX IF NOT EXISTS idx_sessions_end_ts ON sessions (end_ts);
