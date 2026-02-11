CREATE TABLE IF NOT EXISTS handoff_queue (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    package_id TEXT NOT NULL UNIQUE,
    created_at TEXT NOT NULL,
    status TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    payload_size INTEGER NOT NULL,
    expires_at TEXT,
    error TEXT
);

CREATE INDEX IF NOT EXISTS idx_handoff_status ON handoff_queue (status);
CREATE INDEX IF NOT EXISTS idx_handoff_created_at ON handoff_queue (created_at);
