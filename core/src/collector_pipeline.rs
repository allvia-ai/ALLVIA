use anyhow::{Context, Result};
use chrono::{DateTime, Datelike, Duration, SecondsFormat, Utc};
use regex::Regex;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

const KEY_P1_TYPES: &[&str] = &[
    "outlook.compose_started",
    "outlook.attachment_added_meta",
    "excel.refresh_pivot",
];

const DEFAULT_WINDOW_HINT_LIMIT: usize = 64;
const DEFAULT_MAX_RESOURCES: usize = 20;

#[derive(Debug, Deserialize, Default)]
struct ConfigYaml {
    db_path: Option<String>,
    privacy_rules_path: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SessionEventRow {
    pub ts: DateTime<Utc>,
    pub event_type: String,
    pub priority: String,
    pub app: String,
    pub resource_type: String,
    pub resource_id: String,
    pub payload: Value,
}

#[derive(Debug, Clone)]
pub struct SessionRecord {
    pub session_id: String,
    pub start_ts: String,
    pub end_ts: String,
    pub duration_sec: i64,
    pub summary: Value,
}

#[derive(Debug, Clone)]
pub struct RoutineSession {
    pub session_id: String,
    pub start_ts: DateTime<Utc>,
    pub end_ts: DateTime<Utc>,
    pub key_events: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct RoutineCandidate {
    pub pattern_id: String,
    pub pattern_json: String,
    pub support: i64,
    pub confidence: f64,
    pub last_seen_ts: String,
    pub evidence_session_ids: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct HandoffBuildOptions {
    pub max_size_bytes: usize,
    pub recent_sessions: usize,
    pub recent_routines: usize,
    pub max_resources: usize,
    pub max_evidence: usize,
    pub redaction_scan_limit: usize,
}

#[derive(Debug, Clone)]
pub struct HandoffPayload {
    pub payload: Value,
    pub size_bytes: usize,
}

#[derive(Debug, Clone)]
pub struct HandoffPrivacyRules {
    pub denylist_apps: HashSet<String>,
    pub redaction_patterns: Vec<Regex>,
    pub window_title_limit: usize,
}

impl Default for HandoffPrivacyRules {
    fn default() -> Self {
        Self {
            denylist_apps: HashSet::new(),
            redaction_patterns: Vec::new(),
            window_title_limit: DEFAULT_WINDOW_HINT_LIMIT,
        }
    }
}

pub fn resolve_db_path(config_path: Option<&Path>) -> PathBuf {
    if let Ok(value) = std::env::var("STEER_DB_PATH") {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }

    if let Some(path) = config_db_path(config_path) {
        return path;
    }

    if let Some(mut path) = dirs::data_local_dir() {
        path.push("steer");
        let _ = fs::create_dir_all(&path);
        path.push("steer.db");
        return path;
    }

    PathBuf::from("steer.db")
}

pub fn resolve_privacy_rules_path(config_path: Option<&Path>) -> PathBuf {
    if let Ok(value) = std::env::var("STEER_PRIVACY_RULES_PATH") {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }

    if let Some(path) = config_privacy_path(config_path) {
        return path;
    }

    PathBuf::from("configs/privacy_rules.yaml")
}

pub fn open_connection(path: &Path) -> Result<Connection> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed creating db dir {}", parent.display()))?;
        }
    }
    let conn = Connection::open(path)
        .with_context(|| format!("failed opening sqlite db {}", path.display()))?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    Ok(conn)
}

pub fn ensure_pipeline_tables(conn: &Connection) -> Result<()> {
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
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_events_v2_ts ON events_v2(ts)",
        [],
    )?;

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

    conn.execute(
        "CREATE TABLE IF NOT EXISTS collector_state (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )",
        [],
    )?;

    conn.execute(
        "CREATE TABLE IF NOT EXISTS collector_routine_candidates (
            pattern_id TEXT PRIMARY KEY,
            pattern_json TEXT NOT NULL,
            support INTEGER NOT NULL,
            confidence REAL NOT NULL,
            last_seen_ts TEXT NOT NULL,
            evidence_session_ids TEXT NOT NULL,
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        )",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_collector_routines_support
         ON collector_routine_candidates(support)",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_collector_routines_last_seen
         ON collector_routine_candidates(last_seen_ts)",
        [],
    )?;

    conn.execute(
        "CREATE TABLE IF NOT EXISTS collector_handoff_queue (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            package_id TEXT NOT NULL UNIQUE,
            created_at TEXT NOT NULL,
            status TEXT NOT NULL,
            payload_json TEXT NOT NULL,
            payload_size INTEGER NOT NULL,
            expires_at TEXT,
            error TEXT
        )",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_collector_handoff_status
         ON collector_handoff_queue(status)",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_collector_handoff_created
         ON collector_handoff_queue(created_at)",
        [],
    )?;

    Ok(())
}

pub fn get_state(conn: &Connection, key: &str) -> Result<Option<String>> {
    conn.query_row(
        "SELECT value FROM collector_state WHERE key = ?1",
        [key],
        |row| row.get::<_, String>(0),
    )
    .optional()
    .map_err(Into::into)
}

pub fn set_state(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO collector_state (key, value, updated_at)
         VALUES (?1, ?2, datetime('now'))
         ON CONFLICT(key) DO UPDATE SET
             value = excluded.value,
             updated_at = excluded.updated_at",
        params![key, value],
    )?;
    Ok(())
}

pub fn fetch_latest_session_end_ts(conn: &Connection) -> Result<Option<String>> {
    conn.query_row(
        "SELECT end_ts FROM sessions_v2 ORDER BY end_ts DESC LIMIT 1",
        [],
        |row| row.get::<_, String>(0),
    )
    .optional()
    .map_err(Into::into)
}

pub fn fetch_events(
    conn: &Connection,
    start_ts: Option<&str>,
    end_ts: Option<&str>,
) -> Result<Vec<SessionEventRow>> {
    let mut sql = String::from(
        "SELECT ts, event_type, priority, app, resource_type, resource_id, payload_json FROM events_v2",
    );
    let mut clauses: Vec<&str> = Vec::new();
    let mut params_vec: Vec<String> = Vec::new();

    if let Some(start) = start_ts {
        clauses.push("ts >= ?");
        params_vec.push(start.to_string());
    }
    if let Some(end) = end_ts {
        clauses.push("ts <= ?");
        params_vec.push(end.to_string());
    }

    if !clauses.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&clauses.join(" AND "));
    }
    sql.push_str(" ORDER BY ts ASC");

    let mut stmt = conn.prepare(&sql)?;

    let rows = if params_vec.is_empty() {
        stmt.query_map([], |row| {
            map_event_row(
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
            )
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?
    } else if params_vec.len() == 1 {
        stmt.query_map([params_vec[0].as_str()], |row| {
            map_event_row(
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
            )
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?
    } else {
        stmt.query_map(
            params![params_vec[0].as_str(), params_vec[1].as_str()],
            |row| {
                map_event_row(
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                )
            },
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?
    };

    Ok(rows)
}

pub fn sessionize_events(
    events: &[SessionEventRow],
    gap_seconds: i64,
) -> Vec<Vec<SessionEventRow>> {
    let mut sessions: Vec<Vec<SessionEventRow>> = Vec::new();
    let mut current: Vec<SessionEventRow> = Vec::new();
    let mut last_ts: Option<DateTime<Utc>> = None;

    for event in events {
        if let Some(prev) = last_ts {
            if gap_seconds > 0 {
                let gap = (event.ts - prev).num_seconds();
                if gap >= gap_seconds {
                    flush_session(&mut current, &mut sessions);
                }
            }
        }

        if event.event_type.eq_ignore_ascii_case("os.idle_start") {
            flush_session(&mut current, &mut sessions);
            last_ts = None;
            continue;
        }

        current.push(event.clone());

        if event.priority.eq_ignore_ascii_case("P0") {
            flush_session(&mut current, &mut sessions);
            last_ts = None;
            continue;
        }

        last_ts = Some(event.ts);
    }

    flush_session(&mut current, &mut sessions);
    sessions
}

pub fn build_session_records(sessions: &[Vec<SessionEventRow>]) -> Vec<SessionRecord> {
    let mut records = Vec::new();

    for session in sessions {
        if session.is_empty() {
            continue;
        }
        let start = session.first().expect("non-empty session").ts;
        let end = session.last().expect("non-empty session").ts;
        let duration_sec = (end - start).num_seconds().max(0);

        let summary = build_session_summary(session);
        records.push(SessionRecord {
            session_id: uuid::Uuid::new_v4().to_string(),
            start_ts: format_utc_ts(start),
            end_ts: format_utc_ts(end),
            duration_sec,
            summary,
        });
    }

    records
}

pub fn insert_session_record(conn: &Connection, record: &SessionRecord) -> Result<()> {
    conn.execute(
        "INSERT INTO sessions_v2 (session_id, start_ts, end_ts, duration_sec, summary_json)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            record.session_id,
            record.start_ts,
            record.end_ts,
            record.duration_sec,
            serde_json::to_string(&record.summary)?
        ],
    )?;
    Ok(())
}

pub fn fetch_sessions(
    conn: &Connection,
    start_ts: Option<&str>,
    end_ts: Option<&str>,
) -> Result<Vec<RoutineSession>> {
    let mut sql =
        String::from("SELECT session_id, start_ts, end_ts, summary_json FROM sessions_v2");
    let mut clauses: Vec<&str> = Vec::new();
    let mut params_vec: Vec<String> = Vec::new();

    if let Some(start) = start_ts {
        clauses.push("start_ts >= ?");
        params_vec.push(start.to_string());
    }
    if let Some(end) = end_ts {
        clauses.push("end_ts <= ?");
        params_vec.push(end.to_string());
    }

    if !clauses.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&clauses.join(" AND "));
    }
    sql.push_str(" ORDER BY start_ts ASC");

    let mut stmt = conn.prepare(&sql)?;

    let rows = if params_vec.is_empty() {
        stmt.query_map([], |row| {
            map_routine_session_row(row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?
    } else if params_vec.len() == 1 {
        stmt.query_map([params_vec[0].as_str()], |row| {
            map_routine_session_row(row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?
    } else {
        stmt.query_map(
            params![params_vec[0].as_str(), params_vec[1].as_str()],
            |row| map_routine_session_row(row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?),
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?
    };

    Ok(rows)
}

pub fn build_routine_candidates(
    sessions: &[RoutineSession],
    n_min: usize,
    n_max: usize,
    min_support: i64,
    max_patterns: usize,
    max_evidence: usize,
) -> Vec<RoutineCandidate> {
    if max_patterns == 0 {
        return Vec::new();
    }

    #[derive(Default)]
    struct PatternStats {
        support: i64,
        session_ids: Vec<String>,
        session_set: HashSet<String>,
        weekday_counts: HashMap<u32, i64>,
        last_seen: Option<DateTime<Utc>>,
    }

    let mut stats: HashMap<Vec<String>, PatternStats> = HashMap::new();

    for session in sessions {
        if session.key_events.len() < n_min {
            continue;
        }

        let patterns = unique_ngrams(&session.key_events, n_min, n_max);
        if patterns.is_empty() {
            continue;
        }

        let weekday = session.start_ts.weekday().num_days_from_monday();
        for pattern in patterns {
            let entry = stats.entry(pattern).or_default();
            if entry.session_set.contains(&session.session_id) {
                continue;
            }

            entry.session_set.insert(session.session_id.clone());
            entry.session_ids.push(session.session_id.clone());
            entry.support += 1;
            *entry.weekday_counts.entry(weekday).or_insert(0) += 1;
            if entry.last_seen.map(|v| session.end_ts > v).unwrap_or(true) {
                entry.last_seen = Some(session.end_ts);
            }
        }
    }

    let now = Utc::now();
    let mut out = Vec::new();

    for (events, entry) in stats {
        if entry.support < min_support {
            continue;
        }

        let last_seen = entry.last_seen.unwrap_or(now);
        let confidence = compute_confidence(entry.support, &entry.weekday_counts, last_seen, now);
        let pattern_json = json!({
            "type": "ngram",
            "events": events,
            "n": events.len()
        })
        .to_string();

        let mut hasher = Sha256::new();
        hasher.update(pattern_json.as_bytes());
        let pattern_id = format!("{:x}", hasher.finalize());

        let evidence = if max_evidence == 0 {
            Vec::new()
        } else {
            let ids = entry.session_ids;
            if ids.len() <= max_evidence {
                ids
            } else {
                let keep_from = ids.len() - max_evidence;
                ids.into_iter().skip(keep_from).collect()
            }
        };

        out.push(RoutineCandidate {
            pattern_id,
            pattern_json,
            support: entry.support,
            confidence,
            last_seen_ts: format_utc_ts(last_seen),
            evidence_session_ids: evidence,
        });
    }

    out.sort_by(|a, b| {
        b.support
            .cmp(&a.support)
            .then_with(|| b.confidence.total_cmp(&a.confidence))
    });
    out.truncate(max_patterns);
    out
}

pub fn clear_routine_candidates(conn: &Connection) -> Result<()> {
    conn.execute("DELETE FROM collector_routine_candidates", [])?;
    Ok(())
}

pub fn insert_routine_candidate(conn: &Connection, candidate: &RoutineCandidate) -> Result<()> {
    conn.execute(
        "INSERT INTO collector_routine_candidates (
            pattern_id, pattern_json, support, confidence, last_seen_ts, evidence_session_ids
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            candidate.pattern_id,
            candidate.pattern_json,
            candidate.support,
            candidate.confidence,
            candidate.last_seen_ts,
            serde_json::to_string(&candidate.evidence_session_ids)?
        ],
    )?;
    Ok(())
}

pub fn load_handoff_privacy_rules(path: &Path) -> HandoffPrivacyRules {
    let raw = match fs::read_to_string(path) {
        Ok(v) => v,
        Err(_) => return HandoffPrivacyRules::default(),
    };
    let yaml: serde_yaml::Value = match serde_yaml::from_str(&raw) {
        Ok(v) => v,
        Err(_) => return HandoffPrivacyRules::default(),
    };

    let mut rules = HandoffPrivacyRules::default();

    if let Some(list) = yaml.get("denylist_apps").and_then(|v| v.as_sequence()) {
        for app in list {
            if let Some(s) = app.as_str() {
                rules.denylist_apps.insert(s.to_lowercase());
            }
        }
    }

    if let Some(limit) = yaml
        .get("length_limits")
        .and_then(|v| v.as_mapping())
        .and_then(|m| m.get(serde_yaml::Value::String("window_title".to_string())))
        .and_then(|v| v.as_i64())
    {
        if limit > 0 {
            rules.window_title_limit = limit as usize;
        }
    }

    if let Some(patterns) = yaml.get("redaction_patterns").and_then(|v| v.as_sequence()) {
        for item in patterns {
            let regex_str = if let Some(m) = item.as_mapping() {
                m.get(serde_yaml::Value::String("regex".to_string()))
                    .and_then(|v| v.as_str())
            } else {
                item.as_str()
            };

            if let Some(s) = regex_str {
                if let Ok(re) = Regex::new(s) {
                    rules.redaction_patterns.push(re);
                }
            }
        }
    }

    rules
}

pub fn build_handoff_with_size_guard(
    conn: &Connection,
    rules: &HandoffPrivacyRules,
    options: &HandoffBuildOptions,
) -> Result<HandoffPayload> {
    let package_id = uuid::Uuid::new_v4().to_string();
    let created_at = format_utc_ts(Utc::now());

    let profiles = [
        (
            options.recent_sessions,
            options.recent_routines,
            options.max_resources,
        ),
        (
            2usize.min(options.recent_sessions),
            options.recent_routines,
            options.max_resources,
        ),
        (
            1,
            5usize.min(options.recent_routines),
            5usize.min(options.max_resources),
        ),
        (
            1,
            3usize.min(options.recent_routines),
            3usize.min(options.max_resources),
        ),
        (1, 1, 1),
    ];

    let mut last_payload = json!({});
    let mut last_size = 0usize;

    for (session_limit, routine_limit, resource_limit) in profiles {
        let payload = build_handoff_payload(
            conn,
            rules,
            &package_id,
            &created_at,
            session_limit,
            routine_limit,
            resource_limit,
            options.max_evidence,
            options.redaction_scan_limit,
        )?;

        let scrubbed = scrub_value(payload);
        let bytes = serde_json::to_vec(&scrubbed)?.len();
        last_payload = scrubbed;
        last_size = bytes;

        if bytes <= options.max_size_bytes {
            return Ok(HandoffPayload {
                payload: last_payload,
                size_bytes: last_size,
            });
        }
    }

    Ok(HandoffPayload {
        payload: last_payload,
        size_bytes: last_size,
    })
}

pub fn fetch_latest_pending_handoff_payload(conn: &Connection) -> Result<Option<Value>> {
    let row = conn
        .query_row(
            "SELECT payload_json FROM collector_handoff_queue WHERE status = 'pending' ORDER BY created_at DESC LIMIT 1",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()?;

    Ok(row.and_then(|text| serde_json::from_str(&text).ok()))
}

pub fn clear_pending_handoff(conn: &Connection) -> Result<()> {
    conn.execute(
        "DELETE FROM collector_handoff_queue WHERE status = 'pending'",
        [],
    )?;
    Ok(())
}

pub fn enqueue_handoff(conn: &Connection, payload: &HandoffPayload) -> Result<()> {
    let package_id = payload
        .payload
        .get("package_id")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let created_at = payload
        .payload
        .get("created_at")
        .and_then(|v| v.as_str())
        .unwrap_or_default();

    conn.execute(
        "INSERT INTO collector_handoff_queue (
            package_id, created_at, status, payload_json, payload_size, expires_at, error
         ) VALUES (?1, ?2, 'pending', ?3, ?4, NULL, NULL)",
        params![
            package_id,
            created_at,
            serde_json::to_string(&payload.payload)?,
            payload.size_bytes as i64
        ],
    )?;

    Ok(())
}

pub fn iso_now_minus_hours(hours: f64) -> String {
    let seconds = (hours.max(0.0) * 3600.0).round() as i64;
    format_utc_ts(Utc::now() - Duration::seconds(seconds))
}

pub fn iso_now_minus_days(days: f64) -> String {
    let seconds = (days.max(0.0) * 86400.0).round() as i64;
    format_utc_ts(Utc::now() - Duration::seconds(seconds))
}

pub fn parse_iso_ts(value: &str) -> Option<DateTime<Utc>> {
    let normalized = value.trim().replace(' ', "T");
    chrono::DateTime::parse_from_rfc3339(&normalized)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

pub fn plus_one_microsecond_iso(value: &str) -> Option<String> {
    let ts = parse_iso_ts(value)?;
    Some(format_utc_ts(ts + Duration::microseconds(1)))
}

pub fn format_utc_ts(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn config_db_path(config_path: Option<&Path>) -> Option<PathBuf> {
    let cfg_path = config_path?;
    let cfg = read_config(cfg_path)?;
    let db_raw = cfg.db_path?.trim().to_string();
    if db_raw.is_empty() {
        return None;
    }
    let candidate = PathBuf::from(&db_raw);
    if candidate.is_absolute() {
        Some(candidate)
    } else {
        cfg_path
            .parent()
            .map(|p| p.join(&candidate))
            .or(Some(candidate))
    }
}

fn config_privacy_path(config_path: Option<&Path>) -> Option<PathBuf> {
    let cfg_path = config_path?;
    let cfg = read_config(cfg_path)?;
    let raw = cfg.privacy_rules_path?.trim().to_string();
    if raw.is_empty() {
        return None;
    }
    let candidate = PathBuf::from(&raw);
    if candidate.is_absolute() {
        Some(candidate)
    } else {
        cfg_path
            .parent()
            .map(|p| p.join(&candidate))
            .or(Some(candidate))
    }
}

fn read_config(path: &Path) -> Option<ConfigYaml> {
    let text = fs::read_to_string(path).ok()?;
    serde_yaml::from_str::<ConfigYaml>(&text).ok()
}

fn map_event_row(
    ts_raw: String,
    event_type: String,
    priority: String,
    app: String,
    resource_type: String,
    resource_id: String,
    payload_json: String,
) -> rusqlite::Result<SessionEventRow> {
    let ts = parse_iso_ts(&ts_raw).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("invalid ts: {ts_raw}"),
            )),
        )
    })?;

    let payload: Value = serde_json::from_str(&payload_json).unwrap_or_else(|_| json!({}));

    Ok(SessionEventRow {
        ts,
        event_type,
        priority,
        app,
        resource_type,
        resource_id,
        payload,
    })
}

fn flush_session(current: &mut Vec<SessionEventRow>, sessions: &mut Vec<Vec<SessionEventRow>>) {
    if !current.is_empty() {
        sessions.push(std::mem::take(current));
    }
}

fn build_session_summary(events: &[SessionEventRow]) -> Value {
    let mut app_secs: HashMap<String, i64> = HashMap::new();
    let mut key_seen = HashSet::new();
    let mut key_events: Vec<String> = Vec::new();
    let mut resource_seen = HashSet::new();
    let mut resources: Vec<Value> = Vec::new();
    let mut p0 = 0i64;
    let mut p1 = 0i64;
    let mut p2 = 0i64;

    for event in events {
        if event.event_type.eq_ignore_ascii_case("os.app_focus_block") {
            let duration = event
                .payload
                .get("duration_sec")
                .and_then(value_to_i64)
                .unwrap_or(0);
            if duration > 0 {
                *app_secs.entry(event.app.clone()).or_insert(0) += duration;
            }
        }

        let event_type = event.event_type.to_lowercase();
        let priority = event.priority.to_uppercase();

        if priority == "P0" {
            p0 += 1;
        } else if priority == "P1" {
            p1 += 1;
        } else if priority == "P2" {
            p2 += 1;
        }

        let include_key = priority == "P0" || KEY_P1_TYPES.contains(&event_type.as_str());
        if include_key && !event_type.is_empty() && key_seen.insert(event_type.clone()) {
            key_events.push(event_type);
        }

        let key = format!("{}::{}", event.resource_type, event.resource_id);
        if resource_seen.insert(key) {
            resources.push(json!({
                "type": event.resource_type,
                "id": event.resource_id
            }));
            if resources.len() >= DEFAULT_MAX_RESOURCES {
                // Keep scanning for counts/key-events but no more resource append.
            }
        }
    }

    if resources.len() > DEFAULT_MAX_RESOURCES {
        resources.truncate(DEFAULT_MAX_RESOURCES);
    }

    let mut timeline: Vec<Value> = app_secs
        .into_iter()
        .map(|(app, sec)| json!({"app": app, "sec": sec}))
        .collect();
    timeline.sort_by(|a, b| {
        let a_sec = a.get("sec").and_then(|v| v.as_i64()).unwrap_or(0);
        let b_sec = b.get("sec").and_then(|v| v.as_i64()).unwrap_or(0);
        b_sec.cmp(&a_sec)
    });

    json!({
        "apps_timeline": timeline,
        "key_events": key_events,
        "resources": resources,
        "counts": {
            "total": events.len(),
            "p0": p0,
            "p1": p1,
            "p2": p2
        }
    })
}

fn map_routine_session_row(
    session_id: String,
    start_ts_raw: String,
    end_ts_raw: String,
    summary_json: String,
) -> rusqlite::Result<RoutineSession> {
    let start_ts = parse_iso_ts(&start_ts_raw).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            1,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("invalid start ts: {start_ts_raw}"),
            )),
        )
    })?;
    let end_ts = parse_iso_ts(&end_ts_raw).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            2,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("invalid end ts: {end_ts_raw}"),
            )),
        )
    })?;

    let summary: Value = serde_json::from_str(&summary_json).unwrap_or_else(|_| json!({}));
    let mut key_events = Vec::new();
    if let Some(list) = summary.get("key_events").and_then(|v| v.as_array()) {
        for item in list {
            if let Some(s) = item.as_str() {
                let lower = s.to_lowercase();
                if !lower.is_empty() {
                    key_events.push(lower);
                }
            }
        }
    }

    Ok(RoutineSession {
        session_id,
        start_ts,
        end_ts,
        key_events,
    })
}

fn unique_ngrams(events: &[String], n_min: usize, n_max: usize) -> HashSet<Vec<String>> {
    if n_min == 0 || n_max < n_min {
        return HashSet::new();
    }

    let limit = n_max.min(events.len());
    let mut out = HashSet::new();

    for n in n_min..=limit {
        for idx in 0..=(events.len() - n) {
            out.insert(events[idx..idx + n].to_vec());
        }
    }

    out
}

fn compute_confidence(
    support: i64,
    weekday_counts: &HashMap<u32, i64>,
    last_seen: DateTime<Utc>,
    now: DateTime<Utc>,
) -> f64 {
    let days_ago = (now - last_seen).num_days();
    let recency_bonus = if days_ago <= 1 {
        0.3
    } else if days_ago <= 7 {
        0.1
    } else {
        0.0
    };

    let periodicity_bonus = if weekday_counts.values().any(|count| *count >= 2) {
        0.1
    } else {
        0.0
    };

    support as f64 * (1.0 + recency_bonus) * (1.0 + periodicity_bonus)
}

fn build_handoff_payload(
    conn: &Connection,
    rules: &HandoffPrivacyRules,
    package_id: &str,
    created_at: &str,
    sessions_limit: usize,
    routines_limit: usize,
    resources_limit: usize,
    max_evidence: usize,
    redaction_scan_limit: usize,
) -> Result<Value> {
    let device_context = build_device_context(conn, rules)?;
    let last_event_ts = device_context
        .get("last_event_ts")
        .and_then(|v| v.as_str())
        .map(|v| v.to_string());

    let recent_sessions = build_recent_sessions(conn, sessions_limit, resources_limit)?;
    let routines = build_routine_candidates_section(conn, routines_limit, max_evidence)?;
    let signals = build_signals(conn, last_event_ts.as_deref())?;
    let privacy_state = build_privacy_state(conn, rules, redaction_scan_limit)?;

    Ok(json!({
        "package_id": package_id,
        "created_at": created_at,
        "version": "1.0",
        "device_context": device_context,
        "recent_sessions": recent_sessions,
        "routine_candidates": routines,
        "signals": signals,
        "privacy_state": privacy_state
    }))
}

fn build_device_context(conn: &Connection, rules: &HandoffPrivacyRules) -> Result<Value> {
    let row = conn
        .query_row(
            "SELECT ts, event_type, app, payload_json FROM events_v2 ORDER BY ts DESC LIMIT 1",
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )
        .optional()?;

    let Some((ts, event_type, app, payload_json)) = row else {
        return Ok(json!({
            "active_app": Value::Null,
            "active_window_hint": Value::Null,
            "last_event_ts": Value::Null
        }));
    };

    let payload: Value = serde_json::from_str(&payload_json).unwrap_or_else(|_| json!({}));
    let window_title = payload
        .get("window_title")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let hint = if window_title.is_empty() {
        Value::Null
    } else {
        Value::String(sanitize_hint(window_title, rules))
    };

    Ok(json!({
        "active_app": app,
        "active_window_hint": hint,
        "last_event_ts": ts,
        "last_event_type": event_type,
    }))
}

fn build_recent_sessions(
    conn: &Connection,
    limit: usize,
    max_resources: usize,
) -> Result<Vec<Value>> {
    let mut stmt = conn.prepare(
        "SELECT session_id, start_ts, end_ts, duration_sec, summary_json
         FROM sessions_v2
         ORDER BY start_ts DESC
         LIMIT ?1",
    )?;

    let rows = stmt.query_map([limit as i64], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, Option<i64>>(3)?.unwrap_or(0),
            row.get::<_, Option<String>>(4)?
                .unwrap_or_else(|| "{}".to_string()),
        ))
    })?;

    let mut out = Vec::new();
    for row in rows {
        let (session_id, start_ts, end_ts, duration_sec, summary_json) = row?;
        let mut summary: Value = serde_json::from_str(&summary_json).unwrap_or_else(|_| json!({}));

        let resources = summary
            .get_mut("resources")
            .and_then(|v| v.as_array_mut())
            .map(|items| {
                if items.len() > max_resources {
                    items.truncate(max_resources);
                }
                items.clone()
            })
            .unwrap_or_default();

        let apps_timeline = summary
            .get("apps_timeline")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let key_events = summary
            .get("key_events")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let counts = summary.get("counts").cloned().unwrap_or_else(|| json!({}));

        out.push(json!({
            "session_id": session_id,
            "start_ts": start_ts,
            "end_ts": end_ts,
            "duration_sec": duration_sec,
            "apps_timeline": apps_timeline,
            "key_events": key_events,
            "resources": resources,
            "counts": counts
        }));
    }

    Ok(out)
}

fn build_routine_candidates_section(
    conn: &Connection,
    limit: usize,
    max_evidence: usize,
) -> Result<Vec<Value>> {
    let mut stmt = conn.prepare(
        "SELECT pattern_id, pattern_json, support, confidence, last_seen_ts, evidence_session_ids
         FROM collector_routine_candidates
         ORDER BY support DESC, confidence DESC
         LIMIT ?1",
    )?;

    let rows = stmt.query_map([limit as i64], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, f64>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, String>(5)?,
        ))
    })?;

    let mut out = Vec::new();
    for row in rows {
        let (pattern_id, pattern_json, support, confidence, last_seen_ts, evidence_json) = row?;
        let pattern: Value = serde_json::from_str(&pattern_json).unwrap_or_else(|_| json!({}));
        let mut evidence: Vec<Value> = serde_json::from_str(&evidence_json).unwrap_or_default();
        if evidence.len() > max_evidence {
            evidence.truncate(max_evidence);
        }

        out.push(json!({
            "pattern_id": pattern_id,
            "pattern": pattern,
            "support": support,
            "confidence": confidence,
            "last_seen_ts": last_seen_ts,
            "evidence_session_ids": evidence,
        }));
    }

    Ok(out)
}

fn build_signals(conn: &Connection, _last_event_ts: Option<&str>) -> Result<Value> {
    let since = format_utc_ts(Utc::now() - Duration::minutes(5));
    let p0_recent = conn
        .query_row(
            "SELECT 1 FROM events_v2 WHERE priority = 'P0' AND ts >= ?1 LIMIT 1",
            [since],
            |_row| Ok(true),
        )
        .optional()?
        .unwrap_or(false);

    let latest_type = conn
        .query_row(
            "SELECT event_type FROM events_v2 ORDER BY ts DESC LIMIT 1",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()?;

    let idle_state = match latest_type.unwrap_or_default().to_lowercase().as_str() {
        "os.idle_start" => Value::Bool(true),
        "os.idle_end" => Value::Bool(false),
        _ => Value::Null,
    };

    Ok(json!({
        "p0_recent": p0_recent,
        "idle_state": idle_state,
    }))
}

fn build_privacy_state(
    conn: &Connection,
    rules: &HandoffPrivacyRules,
    scan_limit: usize,
) -> Result<Value> {
    let mut stmt = conn.prepare("SELECT privacy_json FROM events_v2 ORDER BY ts DESC LIMIT ?1")?;
    let rows = stmt.query_map([scan_limit as i64], |row| row.get::<_, String>(0))?;

    let mut total = 0i64;
    let mut items: HashMap<String, i64> = HashMap::new();

    for row in rows {
        let privacy_json = row?;
        let parsed: Value = serde_json::from_str(&privacy_json).unwrap_or_else(|_| json!({}));
        if let Some(list) = parsed.get("redaction").and_then(|v| v.as_array()) {
            for item in list {
                if let Some(s) = item.as_str() {
                    total += 1;
                    *items.entry(s.to_string()).or_insert(0) += 1;
                }
            }
        }
    }

    let mut ranked: Vec<(String, i64)> = items.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1));
    ranked.truncate(10);

    let top_map: serde_json::Map<String, Value> = ranked
        .into_iter()
        .map(|(k, v)| (k, Value::Number(v.into())))
        .collect();

    Ok(json!({
        "content_collection": false,
        "denylist_active": !rules.denylist_apps.is_empty(),
        "redaction_summary": {
            "total": total,
            "items": Value::Object(top_map)
        }
    }))
}

fn sanitize_hint(input: &str, rules: &HandoffPrivacyRules) -> String {
    let mut value = input.to_string();

    for re in &rules.redaction_patterns {
        value = re.replace_all(&value, "[REDACTED]").to_string();
    }

    if value.chars().count() > rules.window_title_limit {
        value = value.chars().take(rules.window_title_limit).collect();
    }

    scrub_string(&value)
}

fn scrub_value(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let mapped = map
                .into_iter()
                .map(|(k, v)| (k, scrub_value(v)))
                .collect::<serde_json::Map<String, Value>>();
            Value::Object(mapped)
        }
        Value::Array(list) => Value::Array(list.into_iter().map(scrub_value).collect()),
        Value::String(s) => Value::String(scrub_string(&s)),
        other => other,
    }
}

fn scrub_string(value: &str) -> String {
    static EMAIL_RE_STR: &str = r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}";
    static PATH_RE_STR: &str = r"([A-Za-z]:\\\\|/Users/|/home/|\\.xlsx|\\.docx|\\.pptx)";
    static LONG_DIGITS_RE_STR: &str = r"\b\d{12,}\b";
    static HEX64_RE_STR: &str = r"^[a-f0-9]{64}$";

    let email_re = Regex::new(EMAIL_RE_STR).expect("valid email regex");
    let path_re = Regex::new(PATH_RE_STR).expect("valid path regex");
    let long_digits_re = Regex::new(LONG_DIGITS_RE_STR).expect("valid long digits regex");
    let hex64_re = Regex::new(HEX64_RE_STR).expect("valid hex64 regex");

    if hex64_re.is_match(value) {
        return value.to_string();
    }

    if email_re.is_match(value) || path_re.is_match(value) || long_digits_re.is_match(value) {
        return "[REDACTED]".to_string();
    }

    value.to_string()
}

fn value_to_i64(value: &Value) -> Option<i64> {
    if let Some(v) = value.as_i64() {
        return Some(v);
    }
    if let Some(v) = value.as_u64() {
        return i64::try_from(v).ok();
    }
    if let Some(s) = value.as_str() {
        return s.parse::<i64>().ok();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sessionize_splits_on_idle_and_gap() {
        let base = Utc::now();
        let events = vec![
            SessionEventRow {
                ts: base,
                event_type: "app.open".to_string(),
                priority: "P1".to_string(),
                app: "A".to_string(),
                resource_type: "doc".to_string(),
                resource_id: "1".to_string(),
                payload: json!({}),
            },
            SessionEventRow {
                ts: base + Duration::seconds(10),
                event_type: "os.idle_start".to_string(),
                priority: "P1".to_string(),
                app: "A".to_string(),
                resource_type: "doc".to_string(),
                resource_id: "1".to_string(),
                payload: json!({}),
            },
            SessionEventRow {
                ts: base + Duration::seconds(1000),
                event_type: "app.open".to_string(),
                priority: "P1".to_string(),
                app: "B".to_string(),
                resource_type: "doc".to_string(),
                resource_id: "2".to_string(),
                payload: json!({}),
            },
        ];

        let sessions = sessionize_events(&events, 900);
        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions[0].len(), 1);
        assert_eq!(sessions[1].len(), 1);
    }

    #[test]
    fn routine_candidates_require_support() {
        let now = Utc::now();
        let sessions = vec![
            RoutineSession {
                session_id: "s1".to_string(),
                start_ts: now,
                end_ts: now,
                key_events: vec!["a".to_string(), "b".to_string(), "c".to_string()],
            },
            RoutineSession {
                session_id: "s2".to_string(),
                start_ts: now,
                end_ts: now,
                key_events: vec!["a".to_string(), "b".to_string(), "d".to_string()],
            },
        ];

        let candidates = build_routine_candidates(&sessions, 2, 3, 2, 100, 10);
        assert!(!candidates.is_empty());
        assert!(candidates
            .iter()
            .any(|c| c.pattern_json.contains("\"a\",\"b\"")));
    }
}
