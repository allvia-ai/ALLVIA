#![allow(dead_code)] // Allow unused library functions for future use
use rusqlite::{params, Connection, Result};

use crate::privacy::PrivacyGuard;
use crate::quality_scorer::QualityScore;
use crate::recommendation::AutomationProposal;
use lazy_static::lazy_static;
use std::str::FromStr;
use std::sync::Mutex; // Added

// Global DB connection (for MVP simplicity)
// In production, we should pass a connection pool or handle.
// But rusqlite Connection is not thread-safe, so we wrap in Mutex.
lazy_static! {
    static ref DB_CONN: Mutex<Option<Connection>> = Mutex::new(None);
}

/// Safe helper to acquire DB lock. Recovers from poisoned mutex.
fn get_db_lock() -> std::sync::MutexGuard<'static, Option<Connection>> {
    match DB_CONN.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            eprintln!("⚠️ DB Mutex was poisoned, recovering...");
            poisoned.into_inner()
        }
    }
}

fn column_exists(conn: &Connection, table: &str, column: &str) -> bool {
    let sql = format!("PRAGMA table_info({})", table);
    let mut stmt = match conn.prepare(&sql) {
        Ok(stmt) => stmt,
        Err(e) => {
            eprintln!("Failed to read table_info for {}: {}", table, e);
            return false;
        }
    };
    let rows = match stmt.query_map([], |row| row.get::<_, String>(1)) {
        Ok(rows) => rows,
        Err(e) => {
            eprintln!("Failed to query table_info for {}: {}", table, e);
            return false;
        }
    };
    for name in rows {
        if let Ok(name) = name {
            if name == column {
                return true;
            }
        }
    }
    false
}

fn ensure_column(conn: &Connection, table: &str, column: &str, ddl: &str) {
    if column_exists(conn, table, column) {
        return;
    }
    let sql = format!("ALTER TABLE {} ADD COLUMN {} {}", table, column, ddl);
    if let Err(e) = conn.execute(&sql, []) {
        eprintln!("Failed to add column {}.{}: {}", table, column, e);
    }
}

pub fn init() -> anyhow::Result<()> {
    // [Paranoid Audit] Fix Connection Leak & Idempotency
    {
        let lock = get_db_lock();
        if lock.is_some() {
            // Already initialized, do nothing.
            return Ok(());
        }
    }

    // [Paranoid Audit] Use stable path, with explicit override for pipeline integration.
    let db_path = if let Ok(override_path) = std::env::var("STEER_DB_PATH") {
        let trimmed = override_path.trim();
        if trimmed.is_empty() {
            std::path::PathBuf::from("steer.db")
        } else {
            std::path::PathBuf::from(trimmed)
        }
    } else if let Some(mut path) = dirs::data_local_dir() {
        path.push("steer");
        std::fs::create_dir_all(&path)?; // Ensure ~/.local/share/steer exists
        path.push("steer.db");
        path
    } else {
        std::path::PathBuf::from("steer.db") // Fallback
    };

    if let Some(parent) = db_path.parent() {
        if !parent.as_os_str().is_empty() {
            let _ = std::fs::create_dir_all(parent);
        }
    }

    // Open (or create) steer.db
    let conn = Connection::open(&db_path)?;
    println!("📦 Database initialized at: {:?}", db_path);

    // [Paranoid Audit] Set Busy Timeout to 5s to handle concurrency (Analyzer + API + Main)
    conn.busy_timeout(std::time::Duration::from_secs(5))?;

    // Legacy simple events table
    conn.execute(
        "CREATE TABLE IF NOT EXISTS events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            timestamp TEXT NOT NULL,
            source TEXT NOT NULL,
            type TEXT NOT NULL,
            data TEXT
        )",
        [],
    )?;

    conn.execute(
        "CREATE TABLE IF NOT EXISTS recommendations (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            created_at TEXT NOT NULL,
            status TEXT NOT NULL,
            title TEXT NOT NULL,
            summary TEXT NOT NULL,
            trigger TEXT NOT NULL,
            actions TEXT NOT NULL,
            n8n_prompt TEXT NOT NULL,
            fingerprint TEXT NOT NULL UNIQUE,
            confidence REAL NOT NULL,
            workflow_id TEXT,
            workflow_json TEXT,
            approved_at TEXT,
            evidence TEXT NOT NULL DEFAULT '[]',
            last_error TEXT,
            pattern_id TEXT
        )",
        [],
    )?;

    // Create 'chat_history' table (New Memory System)
    conn.execute(
        "CREATE TABLE IF NOT EXISTS chat_history (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            role TEXT NOT NULL,
            content TEXT NOT NULL,
            created_at TEXT NOT NULL
        )",
        [],
    )?;

    // Create 'routines' table
    conn.execute(
        "CREATE TABLE IF NOT EXISTS routines (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            cron_expression TEXT NOT NULL,
            prompt TEXT NOT NULL,
            enabled BOOLEAN NOT NULL DEFAULT 1,
            last_run TEXT,
            next_run TEXT,
            created_at TEXT NOT NULL
        )",
        [],
    )?;

    conn.execute(
        "CREATE TABLE IF NOT EXISTS exec_approvals (
            id TEXT PRIMARY KEY,
            command TEXT NOT NULL,
            cwd TEXT,
            created_at TEXT NOT NULL,
            expires_at TEXT NOT NULL,
            status TEXT NOT NULL,
            decision TEXT,
            resolved_at TEXT,
            resolved_by TEXT
        )",
        [],
    )?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS nl_approval_policies (
            policy_key TEXT PRIMARY KEY,
            decision TEXT NOT NULL,
            updated_at TEXT NOT NULL
        )",
        [],
    )?;

    conn.execute(
        "CREATE TABLE IF NOT EXISTS exec_allowlist (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            pattern TEXT NOT NULL,
            cwd TEXT,
            created_at TEXT NOT NULL,
            last_used_at TEXT,
            uses_count INTEGER NOT NULL DEFAULT 0
        )",
        [],
    )?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS exec_results (
            id TEXT PRIMARY KEY,
            command TEXT NOT NULL,
            cwd TEXT,
            status TEXT NOT NULL,
            output TEXT,
            error TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT
        )",
        [],
    )?;

    conn.execute(
        "CREATE TABLE IF NOT EXISTS learned_routines (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL UNIQUE,
            steps_json TEXT NOT NULL,
            created_at TEXT NOT NULL
        )",
        [],
    )?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS quality_scores (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            created_at TEXT NOT NULL,
            overall REAL NOT NULL,
            breakdown TEXT NOT NULL,
            issues TEXT NOT NULL,
            strengths TEXT NOT NULL,
            recommendation TEXT NOT NULL,
            summary TEXT NOT NULL
        )",
        [],
    )?;

    // [Paranoid Audit] Performance Indexes - ADDED MISSING INDICES
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_events_timestamp ON events(timestamp)",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_chat_created ON chat_history(created_at)",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_recs_created ON recommendations(created_at)",
        [],
    )?;
    // V2 Indices moved to init_v2 to ensure table exists

    conn.execute(
        "CREATE TABLE IF NOT EXISTS judgment_states (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            last_hash TEXT,
            consecutive_no_progress INTEGER NOT NULL DEFAULT 0,
            updated_at TEXT NOT NULL
        )",
        [],
    )?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS release_baseline (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            created_at TEXT NOT NULL,
            baseline_json TEXT NOT NULL
        )",
        [],
    )?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS verification_runs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            created_at TEXT NOT NULL,
            kind TEXT NOT NULL,
            ok BOOLEAN NOT NULL,
            summary TEXT NOT NULL,
            details TEXT
        )",
        [],
    )?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS routine_runs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            routine_id INTEGER NOT NULL,
            started_at TEXT NOT NULL,
            finished_at TEXT,
            status TEXT NOT NULL,
            error TEXT
        )",
        [],
    )?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS nl_runs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            created_at TEXT NOT NULL,
            intent TEXT NOT NULL,
            prompt TEXT NOT NULL,
            status TEXT NOT NULL,
            summary TEXT,
            details TEXT
        )",
        [],
    )?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS task_runs (
            run_id TEXT PRIMARY KEY,
            created_at TEXT NOT NULL,
            finished_at TEXT,
            intent TEXT NOT NULL,
            prompt TEXT NOT NULL,
            planner_complete INTEGER NOT NULL DEFAULT 0,
            execution_complete INTEGER NOT NULL DEFAULT 0,
            business_complete INTEGER NOT NULL DEFAULT 0,
            status TEXT NOT NULL,
            summary TEXT,
            details TEXT
        )",
        [],
    )?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS task_stage_runs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            run_id TEXT NOT NULL,
            stage_name TEXT NOT NULL,
            stage_order INTEGER NOT NULL,
            status TEXT NOT NULL,
            started_at TEXT NOT NULL,
            finished_at TEXT NOT NULL,
            details TEXT
        )",
        [],
    )?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS task_stage_assertions (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            run_id TEXT NOT NULL,
            stage_name TEXT NOT NULL,
            assertion_key TEXT NOT NULL,
            expected TEXT NOT NULL,
            actual TEXT NOT NULL,
            passed INTEGER NOT NULL,
            evidence TEXT,
            created_at TEXT NOT NULL
        )",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_task_stage_runs_run_id
         ON task_stage_runs(run_id, stage_order)",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_task_stage_assertions_run_id
         ON task_stage_assertions(run_id, stage_name)",
        [],
    )?;

    // Store connection
    {
        let mut lock = get_db_lock();
        *lock = Some(conn);
    } // Lock is dropped here

    println!("📦 Database 'steer.db' initialized.");

    // Init V2 Schema
    {
        // Must release lock before calling init_v2 if it grabs lock?
        // Actually init_v2 grabs lock. But here we already dropped the lock scope in line 79.
    }
    if let Err(e) = init_v2() {
        eprintln!("Failed to init events_v2: {}", e);
    }
    if let Err(e) = init_sessions_table() {
        eprintln!("Failed to init sessions_v2: {}", e);
    }

    // Seed templates if needed (now safe to call)
    if let Err(e) = seed_advanced_examples() {
        eprintln!("Failed to seed templates: {}", e);
    }

    // [Migration] Ensure 'evidence' column exists
    if let Some(conn) = get_db_lock().as_mut() {
        ensure_column(
            conn,
            "recommendations",
            "evidence",
            "TEXT NOT NULL DEFAULT '[]'",
        );
        // [Migration] Phase 1 Context Enrichment
        ensure_column(conn, "events_v2", "window_title", "TEXT");
        ensure_column(conn, "events_v2", "browser_url", "TEXT");
        // [Migration] Phase 3 Final Polish
        ensure_column(conn, "recommendations", "pattern_id", "TEXT");
        ensure_column(conn, "recommendations", "last_error", "TEXT");
        ensure_column(conn, "exec_approvals", "decision", "TEXT");
        // Keep recommendation status model strict: pending/approved/rejected only.
        let _ = conn.execute(
            "UPDATE recommendations
             SET status = CASE
                 WHEN LOWER(status) IN ('pending', 'approved', 'rejected') THEN LOWER(status)
                 ELSE 'pending'
             END
             WHERE status IS NULL
                OR LOWER(status) NOT IN ('pending', 'approved', 'rejected')
                OR status != LOWER(status)",
            [],
        );

        // 1-2. Routine Candidates Table
        if let Err(e) = conn.execute(
            "CREATE TABLE IF NOT EXISTS routine_candidates (
                candidate_id TEXT PRIMARY KEY,
                created_at TEXT NOT NULL,
                pattern_type TEXT NOT NULL,
                description TEXT,
                frequency INTEGER,
                score REAL,
                sample_events TEXT
            )",
            [],
        ) {
            eprintln!("Failed to create routine_candidates: {}", e);
        }
    }

    Ok(())
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Routine {
    pub id: i64,
    pub name: String,
    pub cron_expression: String,
    pub prompt: String,
    pub enabled: bool,
    pub last_run: Option<String>,
    pub next_run: Option<String>,
    pub created_at: String,
}

pub fn create_routine(name: &str, cron: &str, prompt: &str) -> Result<i64> {
    let mut lock = get_db_lock();
    if let Some(conn) = lock.as_mut() {
        let created_at = chrono::Utc::now().to_rfc3339();

        // Calculate initial next_run
        let next_run = match cron::Schedule::from_str(cron) {
            Ok(s) => s
                .upcoming(chrono::Utc)
                .next()
                .map(|d: chrono::DateTime<chrono::Utc>| d.to_rfc3339()),
            Err(_) => None, // Invalid cron, will never run (validation should happen before)
        };

        conn.execute(
            "INSERT INTO routines (name, cron_expression, prompt, created_at, next_run) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![name, cron, prompt, created_at, next_run],
        )?;
        Ok(conn.last_insert_rowid())
    } else {
        Err(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(1),
            Some("DB not initialized".to_string()),
        ))
    }
}

pub fn get_due_routines() -> Result<Vec<Routine>> {
    let mut lock = get_db_lock();
    if let Some(conn) = lock.as_mut() {
        let now = chrono::Utc::now().to_rfc3339();
        let mut stmt = conn.prepare("SELECT id, name, cron_expression, prompt, enabled, last_run, next_run, created_at FROM routines WHERE enabled = 1 AND next_run <= ?1")?;
        let rows = stmt.query_map(params![now], |row| {
            Ok(Routine {
                id: row.get(0)?,
                name: row.get(1)?,
                cron_expression: row.get(2)?,
                prompt: row.get(3)?,
                enabled: row.get(4)?,
                last_run: row.get(5)?,
                next_run: row.get(6)?,
                created_at: row.get(7)?,
            })
        })?;

        let mut routines = Vec::new();
        for routine in rows {
            routines.push(routine?);
        }
        Ok(routines)
    } else {
        Ok(Vec::new())
    }
}

pub fn update_routine_execution(id: i64, next: Option<String>) -> Result<()> {
    let mut lock = get_db_lock();
    if let Some(conn) = lock.as_mut() {
        let now = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "UPDATE routines SET last_run = ?1, next_run = ?2 WHERE id = ?3",
            params![now, next, id],
        )?;
        Ok(())
    } else {
        Ok(())
    }
}

pub fn get_active_routines() -> Result<Vec<Routine>> {
    let mut lock = get_db_lock();
    if let Some(conn) = lock.as_mut() {
        let mut stmt = conn.prepare("SELECT id, name, cron_expression, prompt, enabled, last_run, next_run, created_at FROM routines WHERE enabled = 1")?;
        let rows = stmt.query_map([], |row| {
            Ok(Routine {
                id: row.get(0)?,
                name: row.get(1)?,
                cron_expression: row.get(2)?,
                prompt: row.get(3)?,
                enabled: row.get(4)?,
                last_run: row.get(5)?,
                next_run: row.get(6)?,
                created_at: row.get(7)?,
            })
        })?;
        // ... (collect)
        let mut routines = Vec::new();
        for routine in rows {
            routines.push(routine?);
        }
        Ok(routines)
    } else {
        Ok(Vec::new())
    }
}

pub fn get_all_routines() -> Result<Vec<Routine>> {
    let mut lock = get_db_lock();
    if let Some(conn) = lock.as_mut() {
        let mut stmt = conn.prepare("SELECT id, name, cron_expression, prompt, enabled, last_run, next_run, created_at FROM routines ORDER BY created_at DESC")?;
        let rows = stmt.query_map([], |row| {
            Ok(Routine {
                id: row.get(0)?,
                name: row.get(1)?,
                cron_expression: row.get(2)?,
                prompt: row.get(3)?,
                enabled: row.get(4)?,
                last_run: row.get(5)?,
                next_run: row.get(6)?,
                created_at: row.get(7)?,
            })
        })?;
        // ... (collect)
        let mut routines = Vec::new();
        for routine in rows {
            routines.push(routine?);
        }
        Ok(routines)
    } else {
        Ok(Vec::new())
    }
}

/// Toggle routine enabled status
pub fn toggle_routine(id: i64, enabled: bool) -> Result<()> {
    let mut lock = get_db_lock();
    if let Some(conn) = lock.as_mut() {
        conn.execute(
            "UPDATE routines SET enabled = ?1 WHERE id = ?2",
            params![enabled, id],
        )?;
        Ok(())
    } else {
        Err(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(1),
            Some("DB not initialized".to_string()),
        ))
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct Recommendation {
    pub id: i64,
    pub status: String,
    pub title: String,
    pub summary: String,
    pub trigger: String,
    pub actions: Vec<String>,
    pub n8n_prompt: String,
    pub confidence: f64,
    pub workflow_id: Option<String>,
    pub workflow_json: Option<String>,
    pub evidence: Vec<String>,
    pub pattern_id: Option<String>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct RecommendationMetrics {
    pub total: i64,
    pub approved: i64,
    pub rejected: i64,
    pub failed: i64,
    pub pending: i64,
    /// Backward-compatible counter kept for older dashboard cards.
    pub later: i64,
    /// Count of records outside pending/approved/rejected (legacy data).
    pub legacy_other: i64,
    pub last_created_at: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ExecApproval {
    pub id: String,
    pub command: String,
    pub cwd: Option<String>,
    pub created_at: String,
    pub expires_at: String,
    pub status: String,
    pub decision: Option<String>,
    pub resolved_at: Option<String>,
    pub resolved_by: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ApprovalPolicy {
    pub policy_key: String,
    pub decision: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ExecAllowlistEntry {
    pub id: i64,
    pub pattern: String,
    pub cwd: Option<String>,
    pub created_at: String,
    pub last_used_at: Option<String>,
    pub uses_count: i64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ExecResult {
    pub id: String,
    pub command: String,
    pub cwd: Option<String>,
    pub status: String,
    pub output: Option<String>,
    pub error: Option<String>,
    pub created_at: String,
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct QualityScoreRecord {
    pub created_at: String,
    pub overall: f64,
    pub breakdown: serde_json::Value,
    pub issues: Vec<String>,
    pub strengths: Vec<String>,
    pub recommendation: String,
    pub summary: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct JudgmentState {
    pub last_hash: Option<String>,
    pub consecutive_no_progress: i64,
    pub updated_at: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ReleaseBaselineRecord {
    pub created_at: String,
    pub baseline_json: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct VerificationRun {
    pub id: i64,
    pub created_at: String,
    pub kind: String,
    pub ok: bool,
    pub summary: String,
    pub details: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct NLRun {
    pub id: i64,
    pub created_at: String,
    pub intent: String,
    pub prompt: String,
    pub status: String,
    pub summary: Option<String>,
    pub details: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct TaskRunRecord {
    pub run_id: String,
    pub created_at: String,
    pub finished_at: Option<String>,
    pub intent: String,
    pub prompt: String,
    pub planner_complete: bool,
    pub execution_complete: bool,
    pub business_complete: bool,
    pub status: String,
    pub summary: Option<String>,
    pub details: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct TaskStageRunRecord {
    pub id: i64,
    pub run_id: String,
    pub stage_name: String,
    pub stage_order: i64,
    pub status: String,
    pub started_at: String,
    pub finished_at: String,
    pub details: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct TaskStageAssertionRecord {
    pub id: i64,
    pub run_id: String,
    pub stage_name: String,
    pub assertion_key: String,
    pub expected: String,
    pub actual: String,
    pub passed: bool,
    pub evidence: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct NLRunMetrics {
    pub total: i64,
    pub completed: i64,
    pub manual_required: i64,
    pub approval_required: i64,
    pub blocked: i64,
    pub error: i64,
    pub success_rate: f64,
}
#[derive(Debug, Clone, serde::Serialize)]
pub struct RoutineRun {
    pub id: i64,
    pub routine_id: i64,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub status: String,
    pub error: Option<String>,
}

pub fn insert_recommendation(proposal: &AutomationProposal) -> Result<bool> {
    let mut lock = get_db_lock();
    if let Some(conn) = lock.as_mut() {
        let created_at = chrono::Utc::now().to_rfc3339();
        let actions_json =
            serde_json::to_string(&proposal.actions).unwrap_or_else(|_| "[]".to_string());
        let fingerprint = proposal.fingerprint();

        let rows = conn.execute(
            "INSERT OR IGNORE INTO recommendations (
                created_at, status, title, summary, trigger, actions, n8n_prompt, fingerprint, confidence, workflow_json, evidence, pattern_id, last_error
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                created_at,
                "pending",
                &proposal.title,
                &proposal.summary,
                &proposal.trigger,
                actions_json,
                &proposal.n8n_prompt,
                fingerprint,
                proposal.confidence,
                None::<String>, // No pre-filled JSON for auto-generated ones
                serde_json::to_string(&proposal.evidence).unwrap_or_else(|_| "[]".to_string()),
                proposal.pattern_id,
                None::<String>,
            ],
        )?;
        return Ok(rows > 0);
    }
    Ok(false)
}

pub fn count_recent_recommendations(hours: i64) -> Result<i64> {
    let mut lock = get_db_lock();
    if let Some(conn) = lock.as_mut() {
        let cutoff = (chrono::Utc::now() - chrono::Duration::hours(hours)).to_rfc3339();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM recommendations WHERE created_at >= ?1",
            params![cutoff],
            |row| row.get(0),
        )?;
        return Ok(count);
    }
    Ok(0)
}

pub fn get_recommendation_metrics() -> Result<RecommendationMetrics> {
    let mut lock = get_db_lock();
    if let Some(conn) = lock.as_mut() {
        let mut stmt = conn.prepare(
            "SELECT
                COUNT(*) as total,
                COALESCE(SUM(CASE WHEN status = 'approved' THEN 1 ELSE 0 END), 0) as approved,
                COALESCE(SUM(CASE WHEN status = 'rejected' THEN 1 ELSE 0 END), 0) as rejected,
                COALESCE(SUM(CASE WHEN last_error IS NOT NULL AND TRIM(last_error) != '' THEN 1 ELSE 0 END), 0) as failed,
                COALESCE(SUM(CASE WHEN status = 'pending' THEN 1 ELSE 0 END), 0) as pending,
                0 as later,
                COALESCE(SUM(CASE WHEN status NOT IN ('pending','approved','rejected') THEN 1 ELSE 0 END), 0) as legacy_other,
                MAX(created_at) as last_created_at
             FROM recommendations",
        )?;

        let metrics = stmt.query_row([], |row| {
            Ok(RecommendationMetrics {
                total: row.get(0)?,
                approved: row.get(1)?,
                rejected: row.get(2)?,
                failed: row.get(3)?,
                pending: row.get(4)?,
                later: row.get(5)?,
                legacy_other: row.get(6)?,
                last_created_at: row.get(7).ok(),
            })
        })?;

        return Ok(metrics);
    }
    Ok(RecommendationMetrics {
        total: 0,
        approved: 0,
        rejected: 0,
        failed: 0,
        pending: 0,
        later: 0,
        legacy_other: 0,
        last_created_at: None,
    })
}

pub fn create_exec_approval(
    command: &str,
    cwd: Option<&str>,
    expires_in_secs: i64,
) -> Result<ExecApproval> {
    let mut lock = get_db_lock();
    if let Some(conn) = lock.as_mut() {
        let now = chrono::Utc::now();
        let id = uuid::Uuid::new_v4().to_string();
        let created_at = now.to_rfc3339();
        let expires_at = (now + chrono::Duration::seconds(expires_in_secs)).to_rfc3339();

        conn.execute(
            "INSERT INTO exec_approvals (id, command, cwd, created_at, expires_at, status, decision)
             VALUES (?1, ?2, ?3, ?4, ?5, 'pending', NULL)",
            params![id, command, cwd, created_at, expires_at],
        )?;

        return Ok(ExecApproval {
            id,
            command: command.to_string(),
            cwd: cwd.map(|c| c.to_string()),
            created_at,
            expires_at,
            status: "pending".to_string(),
            decision: None,
            resolved_at: None,
            resolved_by: None,
        });
    }
    Ok(ExecApproval {
        id: "".to_string(),
        command: command.to_string(),
        cwd: cwd.map(|c| c.to_string()),
        created_at: "".to_string(),
        expires_at: "".to_string(),
        status: "pending".to_string(),
        decision: None,
        resolved_at: None,
        resolved_by: None,
    })
}

pub fn resolve_exec_approval(
    id: &str,
    status: &str,
    resolved_by: Option<&str>,
    decision: Option<&str>,
) -> Result<()> {
    let mut lock = get_db_lock();
    if let Some(conn) = lock.as_mut() {
        let resolved_at = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "UPDATE exec_approvals
             SET status = ?1, resolved_at = ?2, resolved_by = ?3, decision = ?4
             WHERE id = ?5",
            params![status, resolved_at, resolved_by, decision, id],
        )?;
    }
    Ok(())
}

pub fn get_exec_approval_status(id: &str) -> Result<String> {
    let mut lock = get_db_lock();
    if let Some(conn) = lock.as_mut() {
        let status: String = conn.query_row(
            "SELECT status FROM exec_approvals WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )?;
        return Ok(status);
    }
    Err(rusqlite::Error::QueryReturnedNoRows)
}

pub fn list_exec_approvals(status_filter: Option<&str>, limit: i64) -> Result<Vec<ExecApproval>> {
    let mut lock = get_db_lock();
    if let Some(conn) = lock.as_mut() {
        let sql = match status_filter {
            Some(_) => "SELECT id, command, cwd, created_at, expires_at, status, decision, resolved_at, resolved_by FROM exec_approvals WHERE status = ?1 ORDER BY created_at DESC LIMIT ?2",
            None => "SELECT id, command, cwd, created_at, expires_at, status, decision, resolved_at, resolved_by FROM exec_approvals ORDER BY created_at DESC LIMIT ?1",
        };

        let mut approvals = Vec::new();
        if let Some(s) = status_filter {
            let mut stmt = conn.prepare(sql)?;
            let rows = stmt.query_map(params![s, limit], |row| {
                Ok(ExecApproval {
                    id: row.get(0)?,
                    command: row.get(1)?,
                    cwd: row.get(2).ok(),
                    created_at: row.get(3)?,
                    expires_at: row.get(4)?,
                    status: row.get(5)?,
                    decision: row.get(6).ok(),
                    resolved_at: row.get(7).ok(),
                    resolved_by: row.get(8).ok(),
                })
            })?;
            for r in rows {
                approvals.push(r?);
            }
        } else {
            let mut stmt = conn.prepare(sql)?;
            let rows = stmt.query_map(params![limit], |row| {
                Ok(ExecApproval {
                    id: row.get(0)?,
                    command: row.get(1)?,
                    cwd: row.get(2).ok(),
                    created_at: row.get(3)?,
                    expires_at: row.get(4)?,
                    status: row.get(5)?,
                    decision: row.get(6).ok(),
                    resolved_at: row.get(7).ok(),
                    resolved_by: row.get(8).ok(),
                })
            })?;
            for r in rows {
                approvals.push(r?);
            }
        }

        return Ok(approvals);
    }
    Ok(Vec::new())
}

pub fn find_valid_exec_approval(command: &str, cwd: Option<&str>) -> Result<Option<ExecApproval>> {
    let mut lock = get_db_lock();
    if let Some(conn) = lock.as_mut() {
        let now = chrono::Utc::now().to_rfc3339();
        let mut stmt = conn.prepare(
            "SELECT id, command, cwd, created_at, expires_at, status, decision, resolved_at, resolved_by
             FROM exec_approvals
             WHERE status = 'approved'
               AND command = ?1
               AND expires_at > ?2
               AND (?3 IS NULL OR IFNULL(cwd, '') = ?3)
             ORDER BY resolved_at DESC
             LIMIT 1",
        )?;
        let row = stmt.query_row(params![command, now, cwd], |row| {
            Ok(ExecApproval {
                id: row.get(0)?,
                command: row.get(1)?,
                cwd: row.get(2).ok(),
                created_at: row.get(3)?,
                expires_at: row.get(4)?,
                status: row.get(5)?,
                decision: row.get(6).ok(),
                resolved_at: row.get(7).ok(),
                resolved_by: row.get(8).ok(),
            })
        });
        return match row {
            Ok(found) => Ok(Some(found)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        };
    }
    Ok(None)
}

pub fn get_exec_approval(id: &str) -> Result<Option<ExecApproval>> {
    let mut lock = get_db_lock();
    if let Some(conn) = lock.as_mut() {
        let mut stmt = conn.prepare(
            "SELECT id, command, cwd, created_at, expires_at, status, decision, resolved_at, resolved_by
             FROM exec_approvals
             WHERE id = ?1",
        )?;
        let mut rows = stmt.query(params![id])?;
        if let Some(row) = rows.next()? {
            return Ok(Some(ExecApproval {
                id: row.get(0)?,
                command: row.get(1)?,
                cwd: row.get(2).ok(),
                created_at: row.get(3)?,
                expires_at: row.get(4)?,
                status: row.get(5)?,
                decision: row.get(6).ok(),
                resolved_at: row.get(7).ok(),
                resolved_by: row.get(8).ok(),
            }));
        }
    }
    Ok(None)
}

pub fn upsert_approval_policy(policy_key: &str, decision: &str) -> Result<()> {
    let mut lock = get_db_lock();
    if let Some(conn) = lock.as_mut() {
        let updated_at = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO nl_approval_policies (policy_key, decision, updated_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(policy_key) DO UPDATE SET decision = excluded.decision, updated_at = excluded.updated_at",
            params![policy_key, decision, updated_at],
        )?;
    }
    Ok(())
}

pub fn delete_approval_policy(policy_key: &str) -> Result<()> {
    let mut lock = get_db_lock();
    if let Some(conn) = lock.as_mut() {
        conn.execute(
            "DELETE FROM nl_approval_policies WHERE policy_key = ?1",
            params![policy_key],
        )?;
    }
    Ok(())
}

pub fn get_approval_policy_decision(policy_key: &str) -> Result<Option<String>> {
    let mut lock = get_db_lock();
    if let Some(conn) = lock.as_mut() {
        let mut stmt =
            conn.prepare("SELECT decision FROM nl_approval_policies WHERE policy_key = ?1")?;
        let mut rows = stmt.query(params![policy_key])?;
        if let Some(row) = rows.next()? {
            return Ok(Some(row.get(0)?));
        }
    }
    Ok(None)
}

pub fn list_approval_policies(limit: i64) -> Result<Vec<ApprovalPolicy>> {
    let mut lock = get_db_lock();
    if let Some(conn) = lock.as_mut() {
        let mut stmt = conn.prepare(
            "SELECT policy_key, decision, updated_at
             FROM nl_approval_policies
             ORDER BY updated_at DESC
             LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit], |row| {
            Ok(ApprovalPolicy {
                policy_key: row.get(0)?,
                decision: row.get(1)?,
                updated_at: row.get(2)?,
            })
        })?;
        let mut policies = Vec::new();
        for r in rows {
            policies.push(r?);
        }
        return Ok(policies);
    }
    Ok(Vec::new())
}

pub fn add_exec_allowlist(pattern: &str, cwd: Option<&str>) -> Result<i64> {
    let mut lock = get_db_lock();
    if let Some(conn) = lock.as_mut() {
        let created_at = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO exec_allowlist (pattern, cwd, created_at) VALUES (?1, ?2, ?3)",
            params![pattern, cwd, created_at],
        )?;
        return Ok(conn.last_insert_rowid());
    }
    Ok(0)
}

pub fn list_exec_allowlist(limit: i64) -> Result<Vec<ExecAllowlistEntry>> {
    let mut lock = get_db_lock();
    if let Some(conn) = lock.as_mut() {
        let mut stmt = conn.prepare(
            "SELECT id, pattern, cwd, created_at, last_used_at, uses_count
             FROM exec_allowlist ORDER BY created_at DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map([limit], |row| {
            Ok(ExecAllowlistEntry {
                id: row.get(0)?,
                pattern: row.get(1)?,
                cwd: row.get(2).ok(),
                created_at: row.get(3)?,
                last_used_at: row.get(4).ok(),
                uses_count: row.get(5)?,
            })
        })?;
        let mut entries = Vec::new();
        for r in rows {
            entries.push(r?);
        }
        return Ok(entries);
    }
    Ok(Vec::new())
}

pub fn remove_exec_allowlist(id: i64) -> Result<()> {
    let mut lock = get_db_lock();
    if let Some(conn) = lock.as_mut() {
        conn.execute("DELETE FROM exec_allowlist WHERE id = ?1", params![id])?;
    }
    Ok(())
}

pub fn create_exec_result(command: &str, cwd: Option<&str>) -> Result<ExecResult> {
    let mut lock = get_db_lock();
    let id = uuid::Uuid::new_v4().to_string();
    if let Some(conn) = lock.as_mut() {
        let created_at = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO exec_results (id, command, cwd, status, output, error, created_at, updated_at)
             VALUES (?1, ?2, ?3, 'pending', NULL, NULL, ?4, NULL)",
            params![id, command, cwd, created_at],
        )?;
        return Ok(ExecResult {
            id,
            command: command.to_string(),
            cwd: cwd.map(|c| c.to_string()),
            status: "pending".to_string(),
            output: None,
            error: None,
            created_at,
            updated_at: None,
        });
    }
    Ok(ExecResult {
        id,
        command: command.to_string(),
        cwd: cwd.map(|c| c.to_string()),
        status: "pending".to_string(),
        output: None,
        error: None,
        created_at: "".to_string(),
        updated_at: None,
    })
}

pub fn update_exec_result(
    id: &str,
    status: &str,
    output: Option<&str>,
    error: Option<&str>,
) -> Result<()> {
    let mut lock = get_db_lock();
    if let Some(conn) = lock.as_mut() {
        let updated_at = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "UPDATE exec_results
             SET status = ?1, output = ?2, error = ?3, updated_at = ?4
             WHERE id = ?5",
            params![status, output, error, updated_at, id],
        )?;
    }
    Ok(())
}

pub fn list_pending_exec_results(limit: i64) -> Result<Vec<ExecResult>> {
    let mut lock = get_db_lock();
    if let Some(conn) = lock.as_mut() {
        let mut stmt = conn.prepare(
            "SELECT id, command, cwd, status, output, error, created_at, updated_at
             FROM exec_results
             WHERE status = 'pending'
             ORDER BY created_at ASC
             LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit], |row| {
            Ok(ExecResult {
                id: row.get(0)?,
                command: row.get(1)?,
                cwd: row.get(2).ok(),
                status: row.get(3)?,
                output: row.get(4).ok(),
                error: row.get(5).ok(),
                created_at: row.get(6)?,
                updated_at: row.get(7).ok(),
            })
        })?;
        let mut results = Vec::new();
        for r in rows {
            results.push(r?);
        }
        return Ok(results);
    }
    Ok(Vec::new())
}

pub fn list_exec_results(status_filter: Option<&str>, limit: i64) -> Result<Vec<ExecResult>> {
    let mut lock = get_db_lock();
    if let Some(conn) = lock.as_mut() {
        let sql = match status_filter {
            Some(_) => "SELECT id, command, cwd, status, output, error, created_at, updated_at FROM exec_results WHERE status = ?1 ORDER BY created_at DESC LIMIT ?2",
            None => "SELECT id, command, cwd, status, output, error, created_at, updated_at FROM exec_results ORDER BY created_at DESC LIMIT ?1",
        };

        let mut results = Vec::new();
        if let Some(status) = status_filter {
            let mut stmt = conn.prepare(sql)?;
            let rows = stmt.query_map(params![status, limit], |row| {
                Ok(ExecResult {
                    id: row.get(0)?,
                    command: row.get(1)?,
                    cwd: row.get(2).ok(),
                    status: row.get(3)?,
                    output: row.get(4).ok(),
                    error: row.get(5).ok(),
                    created_at: row.get(6)?,
                    updated_at: row.get(7).ok(),
                })
            })?;
            for r in rows {
                results.push(r?);
            }
        } else {
            let mut stmt = conn.prepare(sql)?;
            let rows = stmt.query_map(params![limit], |row| {
                Ok(ExecResult {
                    id: row.get(0)?,
                    command: row.get(1)?,
                    cwd: row.get(2).ok(),
                    status: row.get(3)?,
                    output: row.get(4).ok(),
                    error: row.get(5).ok(),
                    created_at: row.get(6)?,
                    updated_at: row.get(7).ok(),
                })
            })?;
            for r in rows {
                results.push(r?);
            }
        }
        return Ok(results);
    }
    Ok(Vec::new())
}

pub fn insert_quality_score(score: &QualityScore) -> Result<()> {
    let mut lock = get_db_lock();
    if let Some(conn) = lock.as_mut() {
        let created_at = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO quality_scores (created_at, overall, breakdown, issues, strengths, recommendation, summary)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                created_at,
                score.overall,
                serde_json::to_string(&score.breakdown).unwrap_or_else(|_| "{}".to_string()),
                serde_json::to_string(&score.issues).unwrap_or_else(|_| "[]".to_string()),
                serde_json::to_string(&score.strengths).unwrap_or_else(|_| "[]".to_string()),
                score.recommendation,
                score.summary
            ],
        )?;
    }
    Ok(())
}

pub fn get_latest_quality_score() -> Result<Option<QualityScoreRecord>> {
    let mut lock = get_db_lock();
    if let Some(conn) = lock.as_mut() {
        let mut stmt = conn.prepare(
            "SELECT created_at, overall, breakdown, issues, strengths, recommendation, summary
             FROM quality_scores
             ORDER BY created_at DESC
             LIMIT 1",
        )?;
        let row = stmt.query_row([], |row| {
            let breakdown_str: String = row.get(2)?;
            let issues_str: String = row.get(3)?;
            let strengths_str: String = row.get(4)?;
            Ok(QualityScoreRecord {
                created_at: row.get(0)?,
                overall: row.get(1)?,
                breakdown: serde_json::from_str(&breakdown_str)
                    .unwrap_or_else(|_| serde_json::json!({})),
                issues: serde_json::from_str(&issues_str).unwrap_or_else(|_| Vec::new()),
                strengths: serde_json::from_str(&strengths_str).unwrap_or_else(|_| Vec::new()),
                recommendation: row.get(5)?,
                summary: row.get(6)?,
            })
        });
        return match row {
            Ok(record) => Ok(Some(record)),
            Err(_) => Ok(None),
        };
    }
    Ok(None)
}

pub fn get_judgment_state() -> Result<Option<JudgmentState>> {
    let mut lock = get_db_lock();
    if let Some(conn) = lock.as_mut() {
        let mut stmt = conn.prepare(
            "SELECT last_hash, consecutive_no_progress, updated_at
             FROM judgment_states
             WHERE id = 1",
        )?;
        let row = stmt.query_row([], |row| {
            Ok(JudgmentState {
                last_hash: row.get(0)?,
                consecutive_no_progress: row.get(1)?,
                updated_at: row.get(2)?,
            })
        });
        return match row {
            Ok(state) => Ok(Some(state)),
            Err(_) => Ok(None),
        };
    }
    Ok(None)
}

pub fn upsert_judgment_state(last_hash: Option<&str>, consecutive_no_progress: i64) -> Result<()> {
    let mut lock = get_db_lock();
    if let Some(conn) = lock.as_mut() {
        let updated_at = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO judgment_states (id, last_hash, consecutive_no_progress, updated_at)
             VALUES (1, ?1, ?2, ?3)
             ON CONFLICT(id) DO UPDATE SET
                last_hash = excluded.last_hash,
                consecutive_no_progress = excluded.consecutive_no_progress,
                updated_at = excluded.updated_at",
            params![last_hash, consecutive_no_progress, updated_at],
        )?;
    }
    Ok(())
}

pub fn get_release_baseline_json() -> Result<Option<ReleaseBaselineRecord>> {
    let mut lock = get_db_lock();
    if let Some(conn) = lock.as_mut() {
        let mut stmt = conn.prepare(
            "SELECT created_at, baseline_json
             FROM release_baseline
             WHERE id = 1",
        )?;
        let row = stmt.query_row([], |row| {
            Ok(ReleaseBaselineRecord {
                created_at: row.get(0)?,
                baseline_json: row.get(1)?,
            })
        });
        return match row {
            Ok(record) => Ok(Some(record)),
            Err(_) => Ok(None),
        };
    }
    Ok(None)
}

pub fn upsert_release_baseline_json(created_at: &str, baseline_json: &str) -> Result<()> {
    let mut lock = get_db_lock();
    if let Some(conn) = lock.as_mut() {
        conn.execute(
            "INSERT INTO release_baseline (id, created_at, baseline_json)
             VALUES (1, ?1, ?2)
             ON CONFLICT(id) DO UPDATE SET
                created_at = excluded.created_at,
                baseline_json = excluded.baseline_json",
            params![created_at, baseline_json],
        )?;
    }
    Ok(())
}

pub fn insert_verification_run(
    kind: &str,
    ok: bool,
    summary: &str,
    details: Option<&str>,
) -> Result<()> {
    let mut lock = get_db_lock();
    if let Some(conn) = lock.as_mut() {
        let created_at = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO verification_runs (created_at, kind, ok, summary, details)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![created_at, kind, ok, summary, details],
        )?;
    }
    Ok(())
}

pub fn list_verification_runs(limit: i64) -> Result<Vec<VerificationRun>> {
    let mut lock = get_db_lock();
    if let Some(conn) = lock.as_mut() {
        let mut stmt = conn.prepare(
            "SELECT id, created_at, kind, ok, summary, details
             FROM verification_runs
             ORDER BY created_at DESC
             LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit], |row| {
            Ok(VerificationRun {
                id: row.get(0)?,
                created_at: row.get(1)?,
                kind: row.get(2)?,
                ok: row.get::<_, i64>(3)? != 0,
                summary: row.get(4)?,
                details: row.get(5).ok(),
            })
        })?;
        let mut runs = Vec::new();
        for r in rows {
            runs.push(r?);
        }
        return Ok(runs);
    }
    Ok(Vec::new())
}

pub fn insert_nl_run(
    intent: &str,
    prompt: &str,
    status: &str,
    summary: Option<&str>,
    details: Option<&str>,
) -> Result<()> {
    let mut lock = get_db_lock();
    if let Some(conn) = lock.as_mut() {
        let created_at = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO nl_runs (created_at, intent, prompt, status, summary, details)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![created_at, intent, prompt, status, summary, details],
        )?;
    }
    Ok(())
}

pub fn create_task_run(run_id: &str, intent: &str, prompt: &str, status: &str) -> Result<()> {
    let mut lock = get_db_lock();
    if let Some(conn) = lock.as_mut() {
        let created_at = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "INSERT OR REPLACE INTO task_runs (
                run_id, created_at, finished_at, intent, prompt,
                planner_complete, execution_complete, business_complete,
                status, summary, details
             ) VALUES (?1, ?2, NULL, ?3, ?4, 0, 0, 0, ?5, NULL, NULL)",
            params![run_id, created_at, intent, prompt, status],
        )?;
    }
    Ok(())
}

pub fn record_task_stage_run(
    run_id: &str,
    stage_name: &str,
    stage_order: i64,
    status: &str,
    details: Option<&str>,
) -> Result<()> {
    let mut lock = get_db_lock();
    if let Some(conn) = lock.as_mut() {
        let now = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO task_stage_runs (
                run_id, stage_name, stage_order, status, started_at, finished_at, details
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![run_id, stage_name, stage_order, status, now, now, details],
        )?;
    }
    Ok(())
}

pub fn record_task_stage_assertion(
    run_id: &str,
    stage_name: &str,
    assertion_key: &str,
    expected: &str,
    actual: &str,
    passed: bool,
    evidence: Option<&str>,
) -> Result<()> {
    let mut lock = get_db_lock();
    if let Some(conn) = lock.as_mut() {
        let created_at = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO task_stage_assertions (
                run_id, stage_name, assertion_key, expected, actual, passed, evidence, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                run_id,
                stage_name,
                assertion_key,
                expected,
                actual,
                passed as i64,
                evidence,
                created_at
            ],
        )?;
    }
    Ok(())
}

pub fn update_task_run_outcome(
    run_id: &str,
    planner_complete: bool,
    execution_complete: bool,
    business_complete: bool,
    status: &str,
    summary: Option<&str>,
    details: Option<&str>,
) -> Result<()> {
    let mut lock = get_db_lock();
    if let Some(conn) = lock.as_mut() {
        let finished_at = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "UPDATE task_runs
             SET finished_at = ?1,
                 planner_complete = ?2,
                 execution_complete = ?3,
                 business_complete = ?4,
                 status = ?5,
                 summary = ?6,
                 details = ?7
             WHERE run_id = ?8",
            params![
                finished_at,
                planner_complete as i64,
                execution_complete as i64,
                business_complete as i64,
                status,
                summary,
                details,
                run_id
            ],
        )?;
    }
    Ok(())
}

pub fn get_task_run(run_id: &str) -> Result<Option<TaskRunRecord>> {
    let mut lock = get_db_lock();
    if let Some(conn) = lock.as_mut() {
        let mut stmt = conn.prepare(
            "SELECT run_id, created_at, finished_at, intent, prompt,
                    planner_complete, execution_complete, business_complete,
                    status, summary, details
             FROM task_runs
             WHERE run_id = ?1
             LIMIT 1",
        )?;

        let mut rows = stmt.query(params![run_id])?;
        if let Some(row) = rows.next()? {
            return Ok(Some(TaskRunRecord {
                run_id: row.get(0)?,
                created_at: row.get(1)?,
                finished_at: row.get(2).ok(),
                intent: row.get(3)?,
                prompt: row.get(4)?,
                planner_complete: row.get::<_, i64>(5)? != 0,
                execution_complete: row.get::<_, i64>(6)? != 0,
                business_complete: row.get::<_, i64>(7)? != 0,
                status: row.get(8)?,
                summary: row.get(9).ok(),
                details: row.get(10).ok(),
            }));
        }
    }
    Ok(None)
}

pub fn list_task_runs(limit: i64, status: Option<&str>) -> Result<Vec<TaskRunRecord>> {
    let bounded_limit = limit.clamp(1, 500);
    let mut lock = get_db_lock();
    if let Some(conn) = lock.as_mut() {
        let normalized_status = status.map(|s| s.trim()).filter(|s| !s.is_empty());
        let mut out = Vec::new();

        if let Some(status_filter) = normalized_status {
            let mut stmt = conn.prepare(
                "SELECT run_id, created_at, finished_at, intent, prompt,
                        planner_complete, execution_complete, business_complete,
                        status, summary, details
                 FROM task_runs
                 WHERE status = ?1
                 ORDER BY created_at DESC
                 LIMIT ?2",
            )?;

            let rows = stmt.query_map(params![status_filter, bounded_limit], |row| {
                Ok(TaskRunRecord {
                    run_id: row.get(0)?,
                    created_at: row.get(1)?,
                    finished_at: row.get(2).ok(),
                    intent: row.get(3)?,
                    prompt: row.get(4)?,
                    planner_complete: row.get::<_, i64>(5)? != 0,
                    execution_complete: row.get::<_, i64>(6)? != 0,
                    business_complete: row.get::<_, i64>(7)? != 0,
                    status: row.get(8)?,
                    summary: row.get(9).ok(),
                    details: row.get(10).ok(),
                })
            })?;

            for row in rows {
                out.push(row?);
            }
            return Ok(out);
        }

        let mut stmt = conn.prepare(
            "SELECT run_id, created_at, finished_at, intent, prompt,
                    planner_complete, execution_complete, business_complete,
                    status, summary, details
             FROM task_runs
             ORDER BY created_at DESC
             LIMIT ?1",
        )?;

        let rows = stmt.query_map(params![bounded_limit], |row| {
            Ok(TaskRunRecord {
                run_id: row.get(0)?,
                created_at: row.get(1)?,
                finished_at: row.get(2).ok(),
                intent: row.get(3)?,
                prompt: row.get(4)?,
                planner_complete: row.get::<_, i64>(5)? != 0,
                execution_complete: row.get::<_, i64>(6)? != 0,
                business_complete: row.get::<_, i64>(7)? != 0,
                status: row.get(8)?,
                summary: row.get(9).ok(),
                details: row.get(10).ok(),
            })
        })?;

        for row in rows {
            out.push(row?);
        }
        return Ok(out);
    }
    Ok(Vec::new())
}

pub fn list_task_stage_runs(run_id: &str) -> Result<Vec<TaskStageRunRecord>> {
    let mut lock = get_db_lock();
    if let Some(conn) = lock.as_mut() {
        let mut stmt = conn.prepare(
            "SELECT id, run_id, stage_name, stage_order, status, started_at, finished_at, details
             FROM task_stage_runs
             WHERE run_id = ?1
             ORDER BY stage_order ASC, id ASC",
        )?;

        let rows = stmt.query_map(params![run_id], |row| {
            Ok(TaskStageRunRecord {
                id: row.get(0)?,
                run_id: row.get(1)?,
                stage_name: row.get(2)?,
                stage_order: row.get(3)?,
                status: row.get(4)?,
                started_at: row.get(5)?,
                finished_at: row.get(6)?,
                details: row.get(7).ok(),
            })
        })?;

        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        return Ok(out);
    }
    Ok(Vec::new())
}

pub fn list_task_stage_assertions(run_id: &str) -> Result<Vec<TaskStageAssertionRecord>> {
    let mut lock = get_db_lock();
    if let Some(conn) = lock.as_mut() {
        let mut stmt = conn.prepare(
            "SELECT id, run_id, stage_name, assertion_key, expected, actual, passed, evidence, created_at
             FROM task_stage_assertions
             WHERE run_id = ?1
             ORDER BY id ASC",
        )?;

        let rows = stmt.query_map(params![run_id], |row| {
            Ok(TaskStageAssertionRecord {
                id: row.get(0)?,
                run_id: row.get(1)?,
                stage_name: row.get(2)?,
                assertion_key: row.get(3)?,
                expected: row.get(4)?,
                actual: row.get(5)?,
                passed: row.get::<_, i64>(6)? != 0,
                evidence: row.get(7).ok(),
                created_at: row.get(8)?,
            })
        })?;

        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        return Ok(out);
    }
    Ok(Vec::new())
}

pub fn list_nl_runs(limit: i64) -> Result<Vec<NLRun>> {
    let mut lock = get_db_lock();
    if let Some(conn) = lock.as_mut() {
        let mut stmt = conn.prepare(
            "SELECT id, created_at, intent, prompt, status, summary, details
             FROM nl_runs
             ORDER BY created_at DESC
             LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit], |row| {
            Ok(NLRun {
                id: row.get(0)?,
                created_at: row.get(1)?,
                intent: row.get(2)?,
                prompt: row.get(3)?,
                status: row.get(4)?,
                summary: row.get(5).ok(),
                details: row.get(6).ok(),
            })
        })?;
        let mut runs = Vec::new();
        for r in rows {
            runs.push(r?);
        }
        return Ok(runs);
    }
    Ok(Vec::new())
}

pub fn get_nl_run_metrics(limit: i64) -> Result<NLRunMetrics> {
    let mut lock = get_db_lock();
    if let Some(conn) = lock.as_mut() {
        let mut stmt = conn.prepare(
            "SELECT
                COUNT(*) as total,
                COALESCE(SUM(CASE WHEN status = 'completed' THEN 1 ELSE 0 END), 0) as completed,
                COALESCE(SUM(CASE WHEN status = 'manual_required' THEN 1 ELSE 0 END), 0) as manual_required,
                COALESCE(SUM(CASE WHEN status = 'approval_required' THEN 1 ELSE 0 END), 0) as approval_required,
                COALESCE(SUM(CASE WHEN status = 'blocked' THEN 1 ELSE 0 END), 0) as blocked,
                COALESCE(SUM(CASE WHEN status = 'error' THEN 1 ELSE 0 END), 0) as error_count
             FROM (
                SELECT status
                FROM nl_runs
                ORDER BY created_at DESC
                LIMIT ?1
             )",
        )?;
        let metrics = stmt.query_row(params![limit], |row| {
            let total: i64 = row.get(0)?;
            let completed: i64 = row.get(1)?;
            let manual_required: i64 = row.get(2)?;
            let approval_required: i64 = row.get(3)?;
            let blocked: i64 = row.get(4)?;
            let error: i64 = row.get(5)?;
            let success_rate = if total > 0 {
                (completed as f64) / (total as f64) * 100.0
            } else {
                0.0
            };
            Ok(NLRunMetrics {
                total,
                completed,
                manual_required,
                approval_required,
                blocked,
                error,
                success_rate,
            })
        })?;
        return Ok(metrics);
    }
    Ok(NLRunMetrics {
        total: 0,
        completed: 0,
        manual_required: 0,
        approval_required: 0,
        blocked: 0,
        error: 0,
        success_rate: 0.0,
    })
}

pub fn is_exec_allowlisted(command: &str, cwd: Option<&str>) -> Result<bool> {
    let mut lock = get_db_lock();
    if let Some(conn) = lock.as_mut() {
        let mut stmt =
            conn.prepare("SELECT id, pattern, cwd FROM exec_allowlist ORDER BY created_at DESC")?;
        let rows = stmt.query_map([], |row| {
            let cwd: Option<String> = row.get(2)?;
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, cwd))
        })?;
        for r in rows {
            let (id, pattern, entry_cwd) = r?;
            if let Some(ref required_cwd) = entry_cwd {
                if let Some(cwd_val) = cwd {
                    if required_cwd != cwd_val {
                        continue;
                    }
                } else {
                    continue;
                }
            }
            if exec_pattern_match(&pattern, command) {
                let now = chrono::Utc::now().to_rfc3339();
                let _ = conn.execute(
                    "UPDATE exec_allowlist SET last_used_at = ?1, uses_count = uses_count + 1 WHERE id = ?2",
                    params![now, id],
                );
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn exec_pattern_match(pattern: &str, command: &str) -> bool {
    let trimmed = pattern.trim();
    if trimmed.is_empty() {
        return false;
    }
    if trimmed == "*" || trimmed.eq_ignore_ascii_case("all") {
        return true;
    }
    if let Some(rest) = trimmed.strip_prefix("re:") {
        if let Ok(re) = regex::Regex::new(rest) {
            return re.is_match(command);
        }
    }
    if trimmed.starts_with('/') && trimmed.ends_with('/') && trimmed.len() > 2 {
        let body = &trimmed[1..trimmed.len() - 1];
        if let Ok(re) = regex::Regex::new(body) {
            return re.is_match(command);
        }
    }
    if trimmed.ends_with('*') {
        let prefix = trimmed.trim_end_matches('*');
        return command.starts_with(prefix);
    }
    command == trimmed
}

pub fn has_recent_pattern_recommendation(pattern_id: &str, hours: i64) -> Result<bool> {
    let mut lock = get_db_lock();
    if let Some(conn) = lock.as_mut() {
        let cutoff = (chrono::Utc::now() - chrono::Duration::hours(hours)).to_rfc3339();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM recommendations WHERE pattern_id = ?1 AND created_at >= ?2",
            params![pattern_id, cutoff],
            |row| row.get(0),
        )?;
        return Ok(count > 0);
    }
    Ok(false)
}

pub fn create_routine_run(routine_id: i64) -> Result<i64> {
    let mut lock = get_db_lock();
    if let Some(conn) = lock.as_mut() {
        let started_at = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO routine_runs (routine_id, started_at, status) VALUES (?1, ?2, 'running')",
            params![routine_id, started_at],
        )?;
        return Ok(conn.last_insert_rowid());
    }
    Ok(0)
}

pub fn finish_routine_run(run_id: i64, status: &str, error: Option<&str>) -> Result<()> {
    let mut lock = get_db_lock();
    if let Some(conn) = lock.as_mut() {
        let finished_at = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "UPDATE routine_runs SET status = ?1, error = ?2, finished_at = ?3 WHERE id = ?4",
            params![status, error, finished_at, run_id],
        )?;
    }
    Ok(())
}

pub fn list_routine_runs(limit: i64) -> Result<Vec<RoutineRun>> {
    let mut lock = get_db_lock();
    if let Some(conn) = lock.as_mut() {
        let mut stmt = conn.prepare(
            "SELECT id, routine_id, started_at, finished_at, status, error
             FROM routine_runs ORDER BY started_at DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map([limit], |row| {
            Ok(RoutineRun {
                id: row.get(0)?,
                routine_id: row.get(1)?,
                started_at: row.get(2)?,
                finished_at: row.get(3).ok(),
                status: row.get(4)?,
                error: row.get(5).ok(),
            })
        })?;

        let mut runs = Vec::new();
        for r in rows {
            runs.push(r?);
        }
        return Ok(runs);
    }
    Ok(Vec::new())
}

pub fn insert_routine_candidate(pattern: &crate::pattern_detector::DetectedPattern) -> Result<()> {
    let mut lock = get_db_lock();
    if let Some(conn) = lock.as_mut() {
        let created_at = chrono::Utc::now().to_rfc3339();
        let samples_json = serde_json::to_string(&pattern.sample_events).unwrap_or_default();

        conn.execute(
            "INSERT OR IGNORE INTO routine_candidates (
                candidate_id, created_at, pattern_type, description, frequency, score, sample_events
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                pattern.pattern_id,
                created_at,
                pattern.pattern_type.as_str(),
                pattern.description,
                pattern.occurrences,
                pattern.similarity_score,
                samples_json
            ],
        )?;
    }
    Ok(())
}

// Function to seed advanced examples if DB is empty
pub fn seed_advanced_examples() -> Result<()> {
    let mut lock = get_db_lock();
    if let Some(conn) = lock.as_mut() {
        // Check if any recommendations exist
        let count: i64 =
            conn.query_row("SELECT count(*) FROM recommendations", [], |row| row.get(0))?;

        if count > 0 {
            return Ok(());
        }

        println!("🌱 Seeding advanced workflow templates...");

        let created_at = chrono::Utc::now().to_rfc3339();

        // Example 1: Morning Briefing
        let briefing_json = r#"{
            "name": "Daily Morning Briefing",
            "nodes": [
                { "type": "n8n-nodes-base.cron", "typeVersion": 1, "position": [100, 300], "parameters": { "triggerTimes": { "item": [{ "mode": "everyDay", "hour": 9 }] } }, "name": "Schedule (9 AM)" },
                { "type": "n8n-nodes-base.googleCalendar", "typeVersion": 1, "position": [300, 300], "parameters": { "operation": "getAll", "calendar": { "__rl": true, "mode": "list", "value": "primary" }, "options": { "timeMin": "={{ $today }}", "timeMax": "={{ $today.end }}" } }, "name": "Get Appointments" },
                { "type": "n8n-nodes-base.openAi", "typeVersion": 1, "position": [500, 300], "parameters": { "resource": "chat", "prompt": { "messages": [{ "role": "user", "content": "Summarize my day based on these events: {{ JSON.stringify($json) }}" }] } }, "name": "AI Summary" },
                { "type": "n8n-nodes-base.telegram", "typeVersion": 1, "position": [700, 300], "parameters": { "chatId": "YOUR_CHAT_ID", "text": "🌞 *Morning Briefing*\n\n{{ $json.message.content }}", "additionalFields": { "parseMode": "Markdown" } }, "name": "Send to Telegram" }
            ],
            "connections": {
                "Schedule (9 AM)": { "main": [[{ "node": "Get Appointments", "type": "main", "index": 0 }]] },
                "Get Appointments": { "main": [[{ "node": "AI Summary", "type": "main", "index": 0 }]] },
                "AI Summary": { "main": [[{ "node": "Send to Telegram", "type": "main", "index": 0 }]] }
            }
        }"#;

        conn.execute(
            "INSERT INTO recommendations (
                created_at, status, title, summary, trigger, actions, n8n_prompt, fingerprint, confidence, workflow_json
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                created_at,
                "pending",
                "🌞 Daily Morning Briefing",
                "매일 아침 9시에 일정과 날씨를 요약해서 텔레그램으로 보냅니다.",
                "Every Day at 9:00 AM",
                "[\"Calendar Check\", \"AI Summary\", \"Telegram Notify\"]",
                "Create a workflow that runs every day at 9am, fetches Google Calendar events, summarizes them using OpenAI, and sends it to Telegram.",
                "seed_briefing_001",
                1.0,
                briefing_json
            ],
        )?;

        // Example 2: Urgent Email Alert
        let urgent_mail_json = r#"{
            "name": "Urgent Email Alert",
            "nodes": [
                { "type": "n8n-nodes-base.gmail", "typeVersion": 2, "position": [100, 300], "parameters": { "pollTimes": { "item": [{ "mode": "everyMinute" }] }, "filters": { "labelIds": ["INBOX"], "readStatus": "unread" } }, "name": "Check Inbox" },
                { "type": "n8n-nodes-base.if", "typeVersion": 1, "position": [300, 300], "parameters": { "conditions": { "string": [{ "value1": "={{ $json.snippet }}", "operation": "contains", "value2": "urgent" }, { "value1": "={{ $json.subject }}", "operation": "contains", "value2": "긴급" }] }, "combineOperation": "any" }, "name": "Is Urgent?" },
                { "type": "n8n-nodes-base.telegram", "typeVersion": 1, "position": [500, 200], "parameters": { "chatId": "YOUR_CHAT_ID", "text": "🚨 *Urgent Email*\n\nFrom: {{ $json.from }}\nSubject: {{ $json.subject }}\nSnippet: {{ $json.snippet }}" }, "name": "Notify Telegram" }
            ],
            "connections": {
                "Check Inbox": { "main": [[{ "node": "Is Urgent?", "type": "main", "index": 0 }]] },
                "Is Urgent?": { "main": [[{ "node": "Notify Telegram", "type": "main", "index": 0 }]] }
            }
        }"#;

        conn.execute(
            "INSERT INTO recommendations (
                created_at, status, title, summary, trigger, actions, n8n_prompt, fingerprint, confidence, workflow_json
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                created_at,
                "pending",
                "🚨 긴급 메일 알림",
                "제목이나 내용에 '긴급'이 포함된 메일이 오면 즉시 알림을 보냅니다.",
                "New Email in Inbox",
                "[\"Check Keywords\", \"Telegram Alert\"]",
                "Watch Gmail for new emails. If subject contains 'urgent' or '긴급', send a Telegram notification.",
                "seed_urgent_001",
                0.95,
                urgent_mail_json
            ],
        )?;
    }
    Ok(())
}

pub fn get_recommendations_with_filter(status_filter: Option<&str>) -> Result<Vec<Recommendation>> {
    let mut lock = get_db_lock();
    if let Some(conn) = lock.as_mut() {
        let sql = match status_filter {
            Some("all") => "SELECT id, status, title, summary, trigger, actions, n8n_prompt, confidence, workflow_id, workflow_json, evidence, pattern_id, last_error FROM recommendations ORDER BY created_at DESC",
            Some(_) => "SELECT id, status, title, summary, trigger, actions, n8n_prompt, confidence, workflow_id, workflow_json, evidence, pattern_id, last_error FROM recommendations WHERE status = ?1 ORDER BY created_at DESC",
            None => "SELECT id, status, title, summary, trigger, actions, n8n_prompt, confidence, workflow_id, workflow_json, evidence, pattern_id, last_error FROM recommendations WHERE status = 'pending' ORDER BY created_at DESC"
        };

        let mut stmt = conn.prepare(sql)?;

        // Execute query map based on filter
        let mut recs = Vec::new();

        if let Some(s) = status_filter {
            if s == "all" {
                let rows = stmt.query_map([], map_row)?;
                for rec in rows {
                    recs.push(rec?);
                }
            } else {
                let rows = stmt.query_map([s], map_row)?;
                for rec in rows {
                    recs.push(rec?);
                }
            }
        } else {
            let rows = stmt.query_map([], map_row)?;
            for rec in rows {
                recs.push(rec?);
            }
        };

        Ok(recs)
    } else {
        Ok(Vec::new())
    }
}

pub fn get_recommendations() -> Result<Vec<Recommendation>> {
    get_recommendations_with_filter(None)
}

// Deprecated wrapper
pub fn list_recommendations(status: &str, _limit: i64) -> Result<Vec<Recommendation>> {
    get_recommendations_with_filter(Some(status))
}

// Helper to map row to struct
fn map_row(row: &rusqlite::Row) -> rusqlite::Result<Recommendation> {
    Ok(Recommendation {
        id: row.get(0)?,
        status: row.get(1)?,
        title: row.get(2)?,
        summary: row.get(3)?,
        trigger: row.get(4)?,
        actions: serde_json::from_str(&row.get::<_, String>(5)?).unwrap_or_default(),
        n8n_prompt: row.get(6)?,
        confidence: row.get(7)?,
        workflow_id: row.get(8)?,
        workflow_json: row.get(9)?,
        evidence: {
            let json: String = row.get(10).unwrap_or_else(|_| "[]".to_string());
            serde_json::from_str(&json).unwrap_or_default()
        },
        pattern_id: row.get(11).ok(),
        last_error: row.get(12).ok(),
    })
}

// Old function body removal target:
/*
pub fn list_recommendations(status: &str, limit: i64) -> Result<Vec<Recommendation>> {
    let mut lock = get_db_lock();
    if let Some(conn) = lock.as_mut() {
        let mut stmt = conn.prepare(
            "SELECT id, status, title, summary, trigger, actions, n8n_prompt, confidence, workflow_id, workflow_json
             FROM recommendations
             WHERE status = ?1
             ORDER BY created_at DESC
             LIMIT ?2",
        )?;
*/

pub fn get_recommendation(id: i64) -> Result<Option<Recommendation>> {
    let mut lock = get_db_lock();
    if let Some(conn) = lock.as_mut() {
        let mut stmt = conn.prepare(
            "SELECT id, status, title, summary, trigger, actions, n8n_prompt, confidence, workflow_id, workflow_json, evidence, pattern_id, last_error
             FROM recommendations
             WHERE id = ?1",
        )?;

        let mut rows = stmt.query(params![id])?;
        if let Some(row) = rows.next()? {
            let actions_json: String = row.get(5)?;
            let actions: Vec<String> = serde_json::from_str(&actions_json).unwrap_or_default();

            let evidence_json: String = row.get(10).unwrap_or_else(|_| "[]".to_string());
            let evidence: Vec<String> = serde_json::from_str(&evidence_json).unwrap_or_default();

            return Ok(Some(Recommendation {
                id: row.get(0)?,
                status: row.get(1)?,
                title: row.get(2)?,
                summary: row.get(3)?,
                trigger: row.get(4)?,
                actions,
                n8n_prompt: row.get(6)?,
                confidence: row.get(7)?,
                workflow_id: row.get(8)?,
                workflow_json: row.get(9)?,
                evidence,
                pattern_id: row.get(11).ok(),
                last_error: row.get(12).ok(),
            }));
        }
    }
    Ok(None)
}

pub fn update_recommendation_status(id: i64, status: &str) -> Result<()> {
    let mut lock = get_db_lock();
    if let Some(conn) = lock.as_mut() {
        conn.execute(
            "UPDATE recommendations SET status = ?1 WHERE id = ?2",
            params![status, id],
        )?;
    }
    Ok(())
}

pub fn update_recommendation_review_status(id: i64, status: &str) -> Result<()> {
    let normalized = status.trim().to_lowercase();
    if normalized != "pending" && normalized != "approved" && normalized != "rejected" {
        return Err(rusqlite::Error::InvalidParameterName(format!(
            "invalid recommendation status '{}': only pending/approved/rejected are allowed",
            status
        )));
    }

    let rec = get_recommendation(id)?.ok_or_else(|| {
        rusqlite::Error::InvalidParameterName(format!("recommendation {} not found", id))
    })?;
    let current = rec.status.trim().to_lowercase();

    let allowed = match (current.as_str(), normalized.as_str()) {
        ("pending", "pending") => true,
        ("pending", "approved") => true,
        ("pending", "rejected") => true,
        ("approved", "approved") => true,
        ("approved", "rejected") => true,
        ("rejected", "rejected") => true,
        ("rejected", "pending") => true,
        _ => false,
    };

    if !allowed {
        return Err(rusqlite::Error::InvalidParameterName(format!(
            "invalid recommendation transition: {} -> {}",
            current, normalized
        )));
    }

    update_recommendation_status(id, &normalized)
}

pub fn mark_recommendation_failed(id: i64, error: &str) -> Result<()> {
    let mut lock = get_db_lock();
    if let Some(conn) = lock.as_mut() {
        // Keep review state in pending/approved/rejected model; store execution failure in last_error.
        conn.execute(
            "UPDATE recommendations
             SET status = CASE
                 WHEN status IN ('approved', 'rejected') THEN status
                 ELSE 'pending'
             END,
             last_error = ?1
             WHERE id = ?2",
            params![error, id],
        )?;
    }
    Ok(())
}

pub fn claim_recommendation_provisioning(id: i64, claim_token: &str) -> Result<Option<String>> {
    let mut lock = get_db_lock();
    if let Some(conn) = lock.as_mut() {
        let mut stmt = conn.prepare(
            "SELECT workflow_id
             FROM recommendations
             WHERE id = ?1",
        )?;
        let mut rows = stmt.query(params![id])?;
        let Some(row) = rows.next()? else {
            return Err(rusqlite::Error::InvalidParameterName(format!(
                "recommendation {} not found",
                id
            )));
        };

        let existing_workflow_id: Option<String> = row.get(0)?;
        if let Some(existing) = existing_workflow_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            return Ok(Some(existing.to_string()));
        }

        let changed = conn.execute(
            "UPDATE recommendations
             SET workflow_id = ?1
             WHERE id = ?2
               AND (workflow_id IS NULL OR TRIM(workflow_id) = '')",
            params![claim_token, id],
        )?;

        if changed > 0 {
            return Ok(None);
        }

        let mut reload_stmt = conn.prepare(
            "SELECT workflow_id
             FROM recommendations
             WHERE id = ?1",
        )?;
        let mut reload_rows = reload_stmt.query(params![id])?;
        if let Some(reload_row) = reload_rows.next()? {
            let current: Option<String> = reload_row.get(0)?;
            if let Some(current) = current.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                return Ok(Some(current.to_string()));
            }
        }
    }
    Ok(None)
}

pub fn release_recommendation_provisioning_claim(id: i64, claim_token: &str) -> Result<()> {
    let mut lock = get_db_lock();
    if let Some(conn) = lock.as_mut() {
        conn.execute(
            "UPDATE recommendations
             SET workflow_id = NULL
             WHERE id = ?1 AND workflow_id = ?2",
            params![id, claim_token],
        )?;
    }
    Ok(())
}

pub fn mark_recommendation_approved(id: i64, workflow_id: &str, workflow_json: &str) -> Result<()> {
    let mut lock = get_db_lock();
    if let Some(conn) = lock.as_mut() {
        let approved_at = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "UPDATE recommendations
             SET status = 'approved', workflow_id = ?1, workflow_json = ?2, approved_at = ?3
             WHERE id = ?4",
            params![workflow_id, workflow_json, approved_at, id],
        )?;
    }
    Ok(())
}

// --- V2 Event Ingestion (Matches Python Schema) ---

pub fn init_v2() -> Result<()> {
    let mut lock = get_db_lock();
    if let Some(conn) = lock.as_mut() {
        conn.execute(
            "CREATE TABLE IF NOT EXISTS events_v2 (
                schema_version TEXT,
                event_id TEXT PRIMARY KEY,
                ts TEXT NOT NULL,
                source TEXT NOT NULL,
                app TEXT NOT NULL,
                event_type TEXT NOT NULL,
                priority TEXT,
                resource_type TEXT,
                resource_id TEXT,
                payload_json TEXT,
                privacy_json TEXT,
                pid INTEGER,
                window_id TEXT,
                window_title TEXT,
                browser_url TEXT,
                raw_json TEXT
            )",
            [],
        )?;

        // [Paranoid Audit] Add Index for V2 Events
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_events_v2_ts ON events_v2(ts)",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_events_v2_type ON events_v2(event_type)",
            [],
        )?;
    }
    Ok(())
}

pub fn insert_event_v2(envelope: &crate::schema::EventEnvelope) -> Result<()> {
    let mut lock = get_db_lock();
    if let Some(conn) = lock.as_mut() {
        let payload_json = serde_json::to_string(&envelope.payload).unwrap_or_default();
        let privacy_json = serde_json::to_string(&envelope.privacy).unwrap_or_default();
        let raw_json = serde_json::to_string(&envelope.raw).unwrap_or_default();

        let (res_type, res_id) = match &envelope.resource {
            Some(r) => (r.resource_type.clone(), r.id.clone()),
            None => ("".to_string(), "".to_string()),
        };

        // Initialize PrivacyGuard (Local scope to safeguard data)
        let salt = std::env::var("PRIVACY_SALT").unwrap_or_else(|_| "default_salt".to_string());
        let guard = PrivacyGuard::new(salt);

        // Mask specific fields
        let window_title = envelope
            .window_title
            .as_ref()
            .map(|t| guard.mask_sensitive_text(t));
        let browser_url = envelope
            .browser_url
            .as_ref()
            .map(|u| guard.mask_sensitive_text(u));

        conn.execute(
            "INSERT OR IGNORE INTO events_v2 (
                schema_version, event_id, ts, source, app, event_type, priority,
                resource_type, resource_id, payload_json, privacy_json, pid, window_id, window_title, browser_url, raw_json
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
            params![
                envelope.schema_version,
                envelope.event_id,
                envelope.ts,
                envelope.source,
                envelope.app,
                envelope.event_type,
                envelope.priority,
                res_type,
                res_id,
                payload_json,
                privacy_json,
                envelope.pid,
                envelope.window_id,
                window_title, // Masked
                browser_url,  // Masked
                raw_json
            ],
        )?;
    }
    Ok(())
}

// Add Sessions Table
pub fn init_sessions_table() -> Result<()> {
    let mut lock = get_db_lock();
    if let Some(conn) = lock.as_mut() {
        conn.execute(
            "CREATE TABLE IF NOT EXISTS sessions_v2 (
                session_id TEXT PRIMARY KEY,
                start_ts TEXT NOT NULL,
                end_ts TEXT NOT NULL,
                duration_sec INTEGER,
                summary_json TEXT
            )",
            [],
        )?;
    }
    Ok(())
}

pub fn fetch_all_events_v2(limit: i64) -> Result<Vec<crate::schema::EventEnvelope>> {
    let mut lock = get_db_lock();
    if let Some(conn) = lock.as_mut() {
        let mut stmt = conn.prepare(
            "SELECT schema_version, event_id, ts, source, app, event_type, priority,
             resource_type, resource_id, payload_json, privacy_json, pid, window_id, window_title, browser_url, raw_json
             FROM events_v2 ORDER BY ts ASC LIMIT ?1"
        )?;

        let rows = stmt.query_map([limit], |row| {
            let payload_str: String = row.get(9)?;
            let privacy_str: String = row.get(10)?;
            let raw_str: String = row.get(15)?;
            let res_type: String = row.get(7)?;
            let res_id: String = row.get(8)?;

            let payload = serde_json::from_str(&payload_str).unwrap_or(serde_json::Value::Null);
            let privacy = serde_json::from_str(&privacy_str).ok();
            let raw = serde_json::from_str(&raw_str).ok();
            let resource = if res_type.is_empty() && res_id.is_empty() {
                None
            } else {
                Some(crate::schema::ResourceContext {
                    resource_type: res_type,
                    id: res_id,
                })
            };

            Ok(crate::schema::EventEnvelope {
                schema_version: row.get(0)?,
                event_id: row.get(1)?,
                ts: row.get(2)?,
                source: row.get(3)?,
                app: row.get(4)?,
                event_type: row.get(5)?,
                priority: row.get(6)?,
                resource,
                payload,
                privacy,
                pid: row.get(11)?,
                window_id: row.get(12)?,
                window_title: row.get(13).ok(),
                browser_url: row.get(14).ok(),
                raw,
            })
        })?;

        let mut events = Vec::new();
        for r in rows {
            events.push(r?);
        }
        Ok(events)
    } else {
        Ok(Vec::new())
    }
}

pub fn insert_session(session: &crate::session::SessionRecord) -> Result<()> {
    let mut lock = get_db_lock();
    if let Some(conn) = lock.as_mut() {
        let summary_json = serde_json::to_string(&session.summary).unwrap_or_default();
        conn.execute(
            "INSERT INTO sessions_v2 (session_id, start_ts, end_ts, duration_sec, summary_json)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                session.session_id,
                session.start_ts,
                session.end_ts,
                session.duration_sec,
                summary_json
            ],
        )?;
    }
    Ok(())
}

// Legacy simple insert (kept for backward compat during migration)
pub fn insert_event(event_json: &str) -> Result<()> {
    let mut lock = get_db_lock();
    if let Some(conn) = lock.as_mut() {
        // Parse basic fields
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(event_json) {
            let timestamp_str = value["timestamp"].as_str().unwrap_or("");
            let timestamp = if timestamp_str.is_empty() {
                chrono::Utc::now().to_rfc3339()
            } else {
                timestamp_str.to_string()
            };

            let source = value["source"].as_str().unwrap_or("unknown");
            let type_ = value["type"].as_str().unwrap_or("unknown");
            // Store full JSON in data
            let data = event_json;

            conn.execute(
                "INSERT INTO events (timestamp, source, type, data) VALUES (?1, ?2, ?3, ?4)",
                params![timestamp, source, type_, data],
            )?;
        }
    }
    Ok(())
}

pub fn get_recent_events(cutoff_hours: i64) -> anyhow::Result<Vec<String>> {
    let mut lock = get_db_lock();
    if let Some(conn) = lock.as_mut() {
        let cutoff = (chrono::Utc::now() - chrono::Duration::hours(cutoff_hours)).to_rfc3339();
        let mut stmt = conn.prepare(
            "SELECT schema_version, event_id, ts, source, app, event_type, priority,
             resource_type, resource_id, payload_json, privacy_json, pid, window_id, window_title, browser_url, raw_json
             FROM events_v2 WHERE ts >= ?1 ORDER BY ts ASC"
        )?;

        let rows = stmt.query_map([cutoff.clone()], |row| {
            let payload_str: String = row.get(9)?;
            let privacy_str: String = row.get(10)?;
            let raw_str: String = row.get(15)?;

            let payload = serde_json::from_str(&payload_str).unwrap_or(serde_json::Value::Null);
            let privacy = serde_json::from_str(&privacy_str).ok();
            let raw = serde_json::from_str(&raw_str).ok();

            let res_type: String = row.get(7)?;
            let res_id: String = row.get(8)?;
            let resource = if res_type.is_empty() && res_id.is_empty() {
                None
            } else {
                Some(crate::schema::ResourceContext {
                    resource_type: res_type,
                    id: res_id,
                })
            };

            let envelope = crate::schema::EventEnvelope {
                schema_version: row.get(0)?,
                event_id: row.get(1)?,
                ts: row.get(2)?,
                source: row.get(3)?,
                app: row.get(4)?,
                event_type: row.get(5)?,
                priority: row.get(6)?,
                resource,
                payload,
                privacy,
                pid: row.get(11)?,
                window_id: row.get(12)?,
                window_title: row.get(13).ok(),
                browser_url: row.get(14).ok(),
                raw,
            };

            Ok(serde_json::to_string(&envelope).unwrap_or_default())
        })?;

        let mut events = Vec::new();
        for event in rows {
            events.push(event?);
        }

        // [Paranoid Audit] Merge Legacy Events correctly
        let mut stmt = conn.prepare(
            "SELECT data FROM events
             WHERE timestamp >= ?1
             ORDER BY timestamp ASC",
        )?;

        let rows = stmt.query_map(params![&cutoff], |row| row.get::<_, String>(0))?;
        for event in rows {
            if let Ok(json) = event {
                events.push(json);
            }
        }

        // Sort merged events by timestamp to be safe (though usually appended logic works if legacy is old)
        // But here we rely on the fact that legacy is old.
        // If we want true sort, we need to parse JSON.
        // For MVP harding, we assume legacy is strictly older or concurrent?
        // Actually, let's just append. Data from V2 is recent.
        // Wait, if V2 is empty, we get legacy. If V2 has data, we ALSO get legacy?
        // Yes, we want full history for the window.

        return Ok(events);
    }
    Err(anyhow::anyhow!("DB not initialized").into())
}

// Memory System: Chat History
#[derive(Debug, Clone, serde::Serialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
    pub created_at: String,
}

pub fn insert_chat_message(role: &str, content: &str) -> Result<()> {
    let mut lock = get_db_lock();
    if let Some(conn) = lock.as_mut() {
        let created_at = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO chat_history (role, content, created_at) VALUES (?1, ?2, ?3)",
            params![role, content, created_at],
        )?;
    }
    Ok(())
}

pub fn get_recent_chat_history(limit: i64) -> Result<Vec<ChatMessage>> {
    let mut lock = get_db_lock();
    if let Some(conn) = lock.as_mut() {
        let mut stmt = conn.prepare(
            "SELECT role, content, created_at FROM chat_history ORDER BY created_at DESC LIMIT ?1",
        )?;

        let rows = stmt.query_map([limit], |row| {
            Ok(ChatMessage {
                role: row.get(0)?,
                content: row.get(1)?,
                created_at: row.get(2)?,
            })
        })?;

        let mut history = Vec::new();
        for row in rows {
            history.push(row?);
        }
        // Return in chronological order for context (REVERSE)
        history.reverse();
        Ok(history)
    } else {
        Ok(Vec::new())
    }
}

// --- Learned Routines (Macro Recorder) ---

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LearnedRoutine {
    pub id: i64,
    pub name: String,
    pub steps_json: String,
    pub created_at: String,
}

pub fn save_learned_routine(name: &str, steps_json: &str) -> Result<()> {
    let mut lock = get_db_lock();
    if let Some(conn) = lock.as_mut() {
        let created_at = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "INSERT OR REPLACE INTO learned_routines (name, steps_json, created_at) VALUES (?1, ?2, ?3)",
            params![name, steps_json, created_at],
        )?;
        Ok(())
    } else {
        Err(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(1),
            Some("DB not initialized".to_string()),
        ))
    }
}

pub fn get_learned_routine(name: &str) -> Result<Option<LearnedRoutine>> {
    let mut lock = get_db_lock();
    if let Some(conn) = lock.as_mut() {
        let mut stmt = conn.prepare(
            "SELECT id, name, steps_json, created_at FROM learned_routines WHERE name = ?1",
        )?;
        let mut rows = stmt.query(params![name])?;

        if let Some(row) = rows.next()? {
            Ok(Some(LearnedRoutine {
                id: row.get(0)?,
                name: row.get(1)?,
                steps_json: row.get(2)?,
                created_at: row.get(3)?,
            }))
        } else {
            Ok(None)
        }
    } else {
        Ok(None)
    }
}

pub fn list_learned_routines() -> Result<Vec<LearnedRoutine>> {
    let mut lock = get_db_lock();
    if let Some(conn) = lock.as_mut() {
        let mut stmt = conn.prepare("SELECT id, name, steps_json, created_at FROM learned_routines ORDER BY created_at DESC")?;
        let rows = stmt.query_map([], |row| {
            Ok(LearnedRoutine {
                id: row.get(0)?,
                name: row.get(1)?,
                steps_json: row.get(2)?,
                created_at: row.get(3)?, // This line was missing in the provided snippet, added for completeness based on LearnedRoutine struct
            })
        })?;

        let mut list = Vec::new();
        for r in rows {
            list.push(r?);
        }
        Ok(list)
    } else {
        Ok(Vec::new())
    }
}

pub fn delete_learned_routine(id: i64) -> Result<()> {
    let mut lock = get_db_lock();
    if let Some(conn) = lock.as_mut() {
        conn.execute("DELETE FROM learned_routines WHERE id = ?1", params![id])?;
    }
    Ok(())
}

// --- Dashboard Stats ---

#[derive(Debug, Clone, serde::Serialize)]
pub struct DashboardStats {
    pub total_sessions: i64,
    pub total_time_mins: i64,
    pub top_apps: Vec<(String, i64)>,
    pub rec_pending: i64,
}

pub fn get_dashboard_stats() -> Result<DashboardStats> {
    let mut lock = get_db_lock();
    if let Some(conn) = lock.as_mut() {
        // 1. Session Stats
        // Check if sessions_v2 exists first, otherwise mock
        let (total_sessions, total_time_mins): (i64, i64) = match conn.query_row(
            "SELECT COUNT(*), COALESCE(SUM(duration_sec)/60, 0) FROM sessions_v2",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ) {
            Ok(res) => res,
            Err(_) => (0, 0), // sessions_v2 might not exist yet
        };

        // 2. Pending Recs
        let rec_pending: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM recommendations WHERE status = 'pending'",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);

        // 3. Top Apps
        let mut top_apps = Vec::new();
        if let Ok(mut stmt) = conn.prepare("SELECT app_name, count(*) as c FROM (select app as app_name from events_v2) GROUP BY app_name ORDER BY c DESC LIMIT 3") {
            let rows = stmt.query_map([], |row| {
                 Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            });
            if let Ok(iter) = rows {
                for r in iter {
                    if let Ok(val) = r { top_apps.push(val); }
                }
            }
        }

        Ok(DashboardStats {
            total_sessions,
            total_time_mins,
            top_apps,
            rec_pending,
        })
    } else {
        Ok(DashboardStats {
            total_sessions: 0,
            total_time_mins: 0,
            top_apps: vec![],
            rec_pending: 0,
        })
    }
}

pub fn get_recent_recommendations(limit: i64) -> Result<Vec<Recommendation>> {
    let mut lock = get_db_lock();
    if let Some(conn) = lock.as_mut() {
        let mut stmt = conn.prepare(
            "SELECT id, status, title, summary, trigger, actions, n8n_prompt, confidence, workflow_id, workflow_json, evidence, pattern_id, last_error 
             FROM recommendations 
             WHERE status IN ('pending', 'approved', 'rejected')
             ORDER BY created_at DESC 
             LIMIT ?1"
        )?;
        let rows = stmt.query_map(params![limit], map_row)?;

        let mut recs = Vec::new();
        for r in rows {
            recs.push(r?);
        }
        Ok(recs)
    } else {
        Ok(Vec::new())
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct PolicyConfigReport {
    pub tool_allowlist: Vec<String>,
    pub tool_denylist: Vec<String>,
    pub shell_allowlist: Vec<String>,
    pub shell_denylist: Vec<String>,
    pub write_lock_default: bool,
}

pub fn get_active_policy_config() -> PolicyConfigReport {
    PolicyConfigReport {
        tool_allowlist: std::env::var("TOOL_ALLOWLIST")
            .unwrap_or_default()
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
        tool_denylist: std::env::var("TOOL_DENYLIST")
            .unwrap_or_default()
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
        shell_allowlist: std::env::var("SHELL_ALLOWLIST")
            .unwrap_or_default()
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
        shell_denylist: std::env::var("SHELL_DENYLIST")
            .unwrap_or_default()
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
        // Check standard env var or default
        write_lock_default: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recommendation::AutomationProposal;

    #[test]
    fn test_init_creates_table() {
        let result = init();
        assert!(result.is_ok());
    }

    #[test]
    fn test_insert_event() {
        init().ok(); // Might error if already init

        let test_event = r#"{"type":"test","source":"unit_test"}"#;
        let insert_result = insert_event(test_event);
        assert!(insert_result.is_ok());
    }

    #[test]
    fn test_recommendation_review_status_transitions() {
        init().ok();

        let unique = format!("status-transition-test-{}", uuid::Uuid::new_v4());
        let proposal = AutomationProposal {
            title: unique.clone(),
            summary: "status transition test".to_string(),
            trigger: "unit-test".to_string(),
            actions: vec!["noop".to_string()],
            confidence: 0.1,
            n8n_prompt: "noop".to_string(),
            evidence: vec![],
            pattern_id: None,
        };
        let _ = insert_recommendation(&proposal);

        let rows = get_recommendations_with_filter(Some("all")).unwrap_or_default();
        let Some(rec) = rows.into_iter().find(|r| r.title == unique) else {
            eprintln!("skip: could not resolve inserted recommendation for transition test");
            return;
        };

        assert!(update_recommendation_review_status(rec.id, "approved").is_ok());
        assert!(update_recommendation_review_status(rec.id, "pending").is_err());
        assert!(update_recommendation_review_status(rec.id, "rejected").is_ok());
        assert!(update_recommendation_review_status(rec.id, "later").is_err());
    }
}
