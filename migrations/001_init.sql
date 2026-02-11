CREATE TABLE IF NOT EXISTS events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    schema_version TEXT NOT NULL,
    event_id TEXT NOT NULL,
    ts TEXT NOT NULL,
    source TEXT NOT NULL,
    app TEXT NOT NULL,
    event_type TEXT NOT NULL,
    priority TEXT NOT NULL,
    resource_type TEXT NOT NULL,
    resource_id TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    privacy_json TEXT NOT NULL,
    pid INTEGER,
    window_id TEXT,
    raw_json TEXT NOT NULL,
    ingested_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_events_ts ON events (ts);
CREATE INDEX IF NOT EXISTS idx_events_app ON events (app);
CREATE INDEX IF NOT EXISTS idx_events_event_type ON events (event_type);
