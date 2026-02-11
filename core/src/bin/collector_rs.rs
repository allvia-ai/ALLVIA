use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::get,
    routing::post,
    Json, Router,
};
use chrono::{DateTime, Duration as ChronoDuration, Timelike, Utc};
use local_os_agent::{db, privacy::PrivacyGuard, schema::EventEnvelope};
use rusqlite::{params, Connection};
use serde::Serialize;
use serde_json::{json, Value};
use tokio::sync::Mutex;

#[derive(Default, Debug)]
struct IngestStats {
    received: u64,
    processed: u64,
    dropped: u64,
    failed: u64,
}

#[derive(Clone)]
struct AppState {
    started_at: Instant,
    stats: Arc<Mutex<IngestStats>>,
    guard: Arc<PrivacyGuard>,
    ingest_token: Option<String>,
}

#[derive(Clone, Debug)]
struct AggregationConfig {
    interval_sec: u64,
    raw_retention_days: i64,
    summary_retention_days: i64,
}

#[derive(Default)]
struct AppAggregate {
    event_count: u64,
    actions: HashMap<String, u64>,
}

#[derive(Default)]
struct PatternAccumulator {
    count: u64,
    examples: Vec<Value>,
}

#[derive(Serialize)]
struct HealthResponse {
    ok: bool,
}

#[derive(Serialize)]
struct StatsResponse {
    uptime_sec: u64,
    received: u64,
    processed: u64,
    dropped: u64,
    failed: u64,
}

impl AggregationConfig {
    fn from_env() -> Self {
        let interval_sec = std::env::var("STEER_COLLECTOR_AGG_INTERVAL_SEC")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|v| *v > 0)
            .unwrap_or(300);

        let raw_retention_days = std::env::var("STEER_COLLECTOR_RAW_RETENTION_DAYS")
            .ok()
            .and_then(|v| v.parse::<i64>().ok())
            .filter(|v| *v >= 1)
            .unwrap_or(7);

        let summary_retention_days = std::env::var("STEER_COLLECTOR_SUMMARY_RETENTION_DAYS")
            .ok()
            .and_then(|v| v.parse::<i64>().ok())
            .filter(|v| *v >= 1)
            .unwrap_or(30);

        Self {
            interval_sec,
            raw_retention_days,
            summary_retention_days,
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    db::init()?;

    let host = std::env::var("STEER_COLLECTOR_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let port = std::env::var("STEER_COLLECTOR_PORT")
        .ok()
        .and_then(|v| v.parse::<u16>().ok())
        .unwrap_or(8080);

    let privacy_salt = std::env::var("PRIVACY_SALT").unwrap_or_else(|_| "default_salt".to_string());
    let ingest_token = std::env::var("STEER_COLLECTOR_TOKEN")
        .ok()
        .or_else(|| std::env::var("COLLECTOR_INGEST_TOKEN").ok())
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());
    let require_ingest_token = std::env::var("STEER_COLLECTOR_REQUIRE_TOKEN")
        .ok()
        .map(|v| matches!(v.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(true);
    if require_ingest_token && ingest_token.is_none() {
        anyhow::bail!(
            "collector_rs token is required. Set STEER_COLLECTOR_TOKEN (or COLLECTOR_INGEST_TOKEN) or set STEER_COLLECTOR_REQUIRE_TOKEN=0 for local-only dev."
        );
    }

    let db_path = resolve_db_path();
    let output_dir = PathBuf::from(
        std::env::var("STEER_WORKFLOW_OUTPUT_DIR").unwrap_or_else(|_| "workflows".to_string()),
    );

    let min_events = std::env::var("STEER_STARTUP_MIN_EVENTS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(100);
    let pattern_threshold = std::env::var("STEER_STARTUP_PATTERN_THRESHOLD")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(3);

    if let Err(err) =
        run_startup_workflow_generation(&db_path, &output_dir, min_events, pattern_threshold)
    {
        eprintln!("⚠️ startup workflow generation failed: {err}");
    }

    let aggregation_config = AggregationConfig::from_env();
    let analytics_db_path = db_path.clone();
    let analytics_cfg = aggregation_config.clone();

    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(analytics_cfg.interval_sec));
        loop {
            ticker.tick().await;
            if let Err(err) = run_analytics_tick(&analytics_db_path, &analytics_cfg) {
                eprintln!("⚠️ analytics tick failed: {err}");
            }
        }
    });

    let state = AppState {
        started_at: Instant::now(),
        stats: Arc::new(Mutex::new(IngestStats::default())),
        guard: Arc::new(PrivacyGuard::new(privacy_salt)),
        ingest_token,
    };

    let app = Router::new()
        .route("/health", get(health_handler))
        .route("/stats", get(stats_handler))
        .route("/events", post(ingest_events_handler))
        .with_state(state);

    let addr: SocketAddr = format!("{host}:{port}").parse()?;
    println!("collector_rs listening on http://{addr}");
    println!("POST /events | GET /health | GET /stats");
    println!(
        "analytics: {}s interval, raw={}d, summary={}d",
        aggregation_config.interval_sec,
        aggregation_config.raw_retention_days,
        aggregation_config.summary_retention_days
    );

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn health_handler() -> Json<HealthResponse> {
    Json(HealthResponse { ok: true })
}

async fn stats_handler(State(state): State<AppState>) -> Json<StatsResponse> {
    let stats = state.stats.lock().await;
    Json(StatsResponse {
        uptime_sec: state.started_at.elapsed().as_secs(),
        received: stats.received,
        processed: stats.processed,
        dropped: stats.dropped,
        failed: stats.failed,
    })
}

async fn ingest_events_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<Value>,
) -> (StatusCode, Json<Value>) {
    if !request_is_authorized(&state, &headers) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({
                "status": "error",
                "error": "unauthorized"
            })),
        );
    }

    let events = match parse_events(payload) {
        Ok(events) => events,
        Err(msg) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "status": "error",
                    "error": msg
                })),
            )
        }
    };

    if events.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "status": "error",
                "error": "no valid events"
            })),
        );
    }

    {
        let mut stats = state.stats.lock().await;
        stats.received += events.len() as u64;
    }

    let mut processed = 0u64;
    let mut dropped = 0u64;
    let mut failed = 0u64;

    for event in events {
        match state.guard.apply(event) {
            Some(masked) => {
                if db::insert_event_v2(&masked).is_ok() {
                    processed += 1;
                } else {
                    failed += 1;
                }
            }
            None => {
                dropped += 1;
            }
        }
    }

    {
        let mut stats = state.stats.lock().await;
        stats.processed += processed;
        stats.dropped += dropped;
        stats.failed += failed;
    }

    let code = if failed > 0 {
        StatusCode::MULTI_STATUS
    } else {
        StatusCode::OK
    };

    (
        code,
        Json(json!({
            "status": "queued",
            "received": processed + dropped + failed,
            "processed": processed,
            "dropped": dropped,
            "failed": failed
        })),
    )
}

fn request_is_authorized(state: &AppState, headers: &HeaderMap) -> bool {
    let Some(expected) = state.ingest_token.as_ref() else {
        return true;
    };
    if let Some(value) = headers
        .get("x-collector-token")
        .and_then(|v| v.to_str().ok())
    {
        if value == expected {
            return true;
        }
    }
    if let Some(value) = headers.get("authorization").and_then(|v| v.to_str().ok()) {
        if let Some(token) = value.strip_prefix("Bearer ") {
            if token.trim() == expected {
                return true;
            }
        }
    }
    false
}

fn parse_events(payload: Value) -> Result<Vec<EventEnvelope>, String> {
    if let Some(arr) = payload.as_array() {
        let mut events = Vec::with_capacity(arr.len());
        for v in arr {
            let event: EventEnvelope = serde_json::from_value(v.clone())
                .map_err(|e| format!("invalid event in array: {e}"))?;
            events.push(event);
        }
        Ok(events)
    } else {
        let single: EventEnvelope =
            serde_json::from_value(payload).map_err(|e| format!("invalid event object: {e}"))?;
        Ok(vec![single])
    }
}

fn run_analytics_tick(db_path: &PathBuf, cfg: &AggregationConfig) -> anyhow::Result<()> {
    let conn = Connection::open(db_path)?;
    conn.busy_timeout(Duration::from_secs(5))?;

    ensure_analytics_tables(&conn)?;
    aggregate_last_five_minute_bucket(&conn, Utc::now())?;
    build_daily_summary_for_yesterday(&conn, Utc::now())?;
    cleanup_old_data(
        &conn,
        cfg.raw_retention_days,
        cfg.summary_retention_days,
        Utc::now(),
    )?;

    Ok(())
}

fn ensure_analytics_tables(conn: &Connection) -> anyhow::Result<()> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS minute_aggregates (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            timestamp TEXT NOT NULL,
            app TEXT NOT NULL,
            event_count INTEGER DEFAULT 0,
            actions_json TEXT,
            created_at TEXT DEFAULT CURRENT_TIMESTAMP,
            UNIQUE(timestamp, app)
        )",
        [],
    )?;

    conn.execute(
        "CREATE TABLE IF NOT EXISTS daily_summaries (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            date TEXT UNIQUE NOT NULL,
            total_events INTEGER DEFAULT 0,
            total_apps INTEGER DEFAULT 0,
            active_hours INTEGER DEFAULT 0,
            app_usage_json TEXT,
            top_actions_json TEXT,
            summary_text TEXT,
            created_at TEXT DEFAULT CURRENT_TIMESTAMP
        )",
        [],
    )?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_ma_ts ON minute_aggregates(timestamp)",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_ds_date ON daily_summaries(date)",
        [],
    )?;

    Ok(())
}

fn aggregate_last_five_minute_bucket(conn: &Connection, now: DateTime<Utc>) -> anyhow::Result<()> {
    let bucket_end = floor_to_five_minute_bucket(now);
    let bucket_start = bucket_end - ChronoDuration::minutes(5);

    let start_iso = format_iso_z(bucket_start);
    let end_iso = format_iso_z(bucket_end);

    let mut stmt = conn.prepare(
        "SELECT app, payload_json
         FROM events_v2
         WHERE datetime(ts) >= datetime(?1)
           AND datetime(ts) < datetime(?2)
         ORDER BY ts",
    )?;

    let rows = stmt.query_map(params![start_iso, end_iso], |row| {
        let app: String = row.get(0)?;
        let payload_json: String = row.get(1)?;
        Ok((app, payload_json))
    })?;

    let mut by_app: HashMap<String, AppAggregate> = HashMap::new();

    for row in rows {
        let (app, payload_json) = row?;
        let aggregate = by_app.entry(app).or_default();
        aggregate.event_count += 1;

        let payload: Value = serde_json::from_str(&payload_json).unwrap_or(Value::Null);
        if let Some(obj) = payload.as_object() {
            let control_type = obj
                .get("control_type")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            let element_name = obj
                .get("element_name")
                .and_then(|v| v.as_str())
                .unwrap_or_default();

            if !control_type.is_empty() && !element_name.is_empty() {
                let action_key = format!("{}:{}", control_type, truncate(element_name, 64));
                *aggregate.actions.entry(action_key).or_insert(0) += 1;
            }
        }
    }

    if by_app.is_empty() {
        return Ok(());
    }

    let bucket_label = bucket_start.format("%Y-%m-%d %H:%M").to_string();

    for (app, mut aggregate) in by_app {
        let mut top_actions: Vec<(String, u64)> = aggregate.actions.drain().collect();
        top_actions.sort_by(|a, b| b.1.cmp(&a.1));
        top_actions.truncate(5);
        let actions_json = serde_json::to_string(&top_actions)?;

        conn.execute(
            "INSERT OR REPLACE INTO minute_aggregates (timestamp, app, event_count, actions_json)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                bucket_label,
                app,
                aggregate.event_count as i64,
                actions_json
            ],
        )?;
    }

    Ok(())
}

fn build_daily_summary_for_yesterday(conn: &Connection, now: DateTime<Utc>) -> anyhow::Result<()> {
    let yesterday = (now - ChronoDuration::days(1))
        .date_naive()
        .format("%Y-%m-%d")
        .to_string();
    build_daily_summary_for_date(conn, &yesterday)
}

fn build_daily_summary_for_date(conn: &Connection, date: &str) -> anyhow::Result<()> {
    let mut stmt = conn.prepare(
        "SELECT app, SUM(event_count) as total
         FROM minute_aggregates
         WHERE date(timestamp) = ?1
         GROUP BY app
         ORDER BY total DESC",
    )?;

    let rows = stmt.query_map(params![date], |row| {
        let app: String = row.get(0)?;
        let total: i64 = row.get(1)?;
        Ok((app, total.max(0) as u64))
    })?;

    let mut app_usage: Vec<(String, u64)> = Vec::new();
    let mut total_events = 0u64;

    for row in rows {
        let (app, total) = row?;
        total_events += total;
        app_usage.push((app, total));
    }

    if app_usage.is_empty() {
        return Ok(());
    }

    let mut actions_stmt = conn.prepare(
        "SELECT actions_json
         FROM minute_aggregates
         WHERE date(timestamp) = ?1",
    )?;

    let action_rows = actions_stmt.query_map(params![date], |row| row.get::<_, String>(0))?;

    let mut all_actions: HashMap<String, u64> = HashMap::new();
    for row in action_rows {
        let actions_json = row?;
        let parsed: Vec<(String, u64)> = serde_json::from_str(&actions_json).unwrap_or_default();
        for (action, count) in parsed {
            *all_actions.entry(action).or_insert(0) += count;
        }
    }

    let active_hours: i64 = conn.query_row(
        "SELECT COUNT(DISTINCT substr(timestamp, 12, 2))
         FROM minute_aggregates
         WHERE date(timestamp) = ?1",
        params![date],
        |row| row.get(0),
    )?;

    app_usage.sort_by(|a, b| b.1.cmp(&a.1));
    let mut top_actions: Vec<(String, u64)> = all_actions.into_iter().collect();
    top_actions.sort_by(|a, b| b.1.cmp(&a.1));
    top_actions.truncate(20);

    let top_apps_display: Vec<(String, u64)> = app_usage.iter().take(5).cloned().collect();

    let summary_text = {
        let mut parts = vec![
            format!("date: {date}"),
            format!("total events: {total_events}"),
            format!("apps used: {}", app_usage.len()),
            format!("active hours: {}", active_hours.max(0)),
        ];
        if !top_apps_display.is_empty() {
            parts.push("top apps:".to_string());
            for (app, count) in &top_apps_display {
                parts.push(format!("- {}: {}", normalize_app_for_display(app), count));
            }
        }
        parts.join("\n")
    };

    let app_usage_json = serde_json::to_string(&app_usage)?;
    let top_actions_json = serde_json::to_string(&top_actions)?;

    conn.execute(
        "INSERT INTO daily_summaries
            (date, total_events, total_apps, active_hours, app_usage_json, top_actions_json, summary_text)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(date) DO UPDATE SET
            total_events = excluded.total_events,
            total_apps = excluded.total_apps,
            active_hours = excluded.active_hours,
            app_usage_json = excluded.app_usage_json,
            top_actions_json = excluded.top_actions_json,
            summary_text = excluded.summary_text",
        params![
            date,
            total_events as i64,
            app_usage.len() as i64,
            active_hours,
            app_usage_json,
            top_actions_json,
            summary_text
        ],
    )?;

    Ok(())
}

fn cleanup_old_data(
    conn: &Connection,
    raw_retention_days: i64,
    summary_retention_days: i64,
    now: DateTime<Utc>,
) -> anyhow::Result<()> {
    let raw_cutoff = (now - ChronoDuration::days(raw_retention_days.max(1)))
        .date_naive()
        .format("%Y-%m-%d")
        .to_string();
    let summary_cutoff = (now - ChronoDuration::days(summary_retention_days.max(1)))
        .date_naive()
        .format("%Y-%m-%d")
        .to_string();

    conn.execute(
        "DELETE FROM events_v2 WHERE date(ts) < ?1",
        params![raw_cutoff],
    )?;
    conn.execute(
        "DELETE FROM minute_aggregates WHERE date(timestamp) < ?1",
        params![summary_cutoff],
    )?;

    Ok(())
}

fn run_startup_workflow_generation(
    db_path: &PathBuf,
    output_dir: &PathBuf,
    min_events: usize,
    pattern_threshold: u64,
) -> anyhow::Result<Option<PathBuf>> {
    fs::create_dir_all(output_dir)?;

    let yesterday = (Utc::now() - ChronoDuration::days(1))
        .date_naive()
        .format("%Y-%m-%d")
        .to_string();

    let output_path = output_dir.join(format!("workflow_{yesterday}.json"));
    if output_path.exists() {
        return Ok(None);
    }

    let conn = Connection::open(db_path)?;
    let mut stmt = conn.prepare(
        "SELECT ts, app, payload_json
         FROM events_v2
         WHERE date(ts) <= ?1
         ORDER BY ts ASC",
    )?;

    let rows = stmt.query_map(params![yesterday], |row| {
        let ts: String = row.get(0)?;
        let app: String = row.get(1)?;
        let payload_json: String = row.get(2)?;
        Ok((ts, app, payload_json))
    })?;

    let mut total_events = 0usize;
    let mut patterns: HashMap<(String, String, String), PatternAccumulator> = HashMap::new();

    for row in rows {
        let (ts, app, payload_json) = row?;
        total_events += 1;

        let payload: Value = serde_json::from_str(&payload_json).unwrap_or(Value::Null);
        let Some(obj) = payload.as_object() else {
            continue;
        };

        let control_type = obj
            .get("control_type")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let element_name = obj
            .get("element_name")
            .and_then(|v| v.as_str())
            .unwrap_or_default();

        if control_type.is_empty() || element_name.is_empty() {
            continue;
        }

        let app_short = normalize_app_for_pattern(&app);
        let key = (
            app_short,
            control_type.to_string(),
            element_name.to_string(),
        );
        let entry = patterns.entry(key).or_default();
        entry.count += 1;

        if entry.examples.len() < 3 {
            entry.examples.push(json!({
                "ts": ts,
                "window_title": obj
                    .get("window_title")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default(),
                "automation_id": obj
                    .get("automation_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
            }));
        }
    }

    if total_events < min_events {
        return Ok(None);
    }

    let mut ranked: Vec<(String, String, String, u64, Vec<Value>)> = patterns
        .into_iter()
        .filter_map(|((app, control_type, element_name), acc)| {
            if acc.count < pattern_threshold {
                None
            } else {
                Some((app, control_type, element_name, acc.count, acc.examples))
            }
        })
        .collect();

    ranked.sort_by(|a, b| b.3.cmp(&a.3));
    ranked.truncate(20);

    if ranked.is_empty() {
        return Ok(None);
    }

    let mut top_apps = BTreeSet::new();
    let mut steps = Vec::with_capacity(ranked.len());

    for (idx, (app, control_type, element_name, frequency, examples)) in ranked.iter().enumerate() {
        if top_apps.len() < 5 {
            top_apps.insert(app.clone());
        }

        let first_example = examples.first().cloned().unwrap_or_else(|| json!({}));
        let window_title = first_example
            .get("window_title")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let automation_id = first_example
            .get("automation_id")
            .and_then(|v| v.as_str())
            .unwrap_or_default();

        steps.push(json!({
            "step_number": idx + 1,
            "action_type": infer_action_type(control_type),
            "target": {
                "app": app,
                "control_type": control_type,
                "element_name": element_name,
                "window_title": window_title,
                "automation_id": automation_id
            },
            "frequency": frequency,
            "description": format!("{} {} '{}' ({} repeats)", app, control_type, element_name, frequency)
        }));
    }

    let events_analyzed: u64 = ranked.iter().map(|(_, _, _, count, _)| *count).sum();

    let workflow = json!({
        "workflow_name": format!("Daily Patterns - {yesterday}"),
        "description": format!("Behavior pattern analysis through {yesterday}"),
        "created_at": Utc::now().to_rfc3339(),
        "analysis_period": {
            "until": yesterday,
            "events_analyzed": events_analyzed,
            "events_scanned": total_events
        },
        "patterns": steps,
        "metadata": {
            "total_patterns": ranked.len(),
            "top_apps": top_apps.into_iter().collect::<Vec<_>>(),
            "generated_by": "collector_rs_startup_generator"
        }
    });

    fs::write(&output_path, serde_json::to_string_pretty(&workflow)?)?;
    println!("[Workflow] Generated: {}", output_path.display());

    Ok(Some(output_path))
}

fn resolve_db_path() -> PathBuf {
    if let Ok(override_path) = std::env::var("STEER_DB_PATH") {
        let trimmed = override_path.trim();
        if trimmed.is_empty() {
            PathBuf::from("steer.db")
        } else {
            PathBuf::from(trimmed)
        }
    } else if let Some(mut path) = dirs::data_local_dir() {
        path.push("steer");
        let _ = fs::create_dir_all(&path);
        path.push("steer.db");
        path
    } else {
        PathBuf::from("steer.db")
    }
}

fn floor_to_five_minute_bucket(ts: DateTime<Utc>) -> DateTime<Utc> {
    let minute_bucket = ts.minute() - (ts.minute() % 5);
    ts.with_second(0)
        .and_then(|v| v.with_nanosecond(0))
        .and_then(|v| v.with_minute(minute_bucket))
        .unwrap_or(ts)
}

fn format_iso_z(ts: DateTime<Utc>) -> String {
    ts.format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

fn normalize_app_for_pattern(app: &str) -> String {
    let source = if let Some((_, suffix)) = app.rsplit_once(" - ") {
        suffix
    } else {
        app
    };

    source
        .split('.')
        .next()
        .unwrap_or(source)
        .trim()
        .to_string()
}

fn normalize_app_for_display(app: &str) -> String {
    if let Some((_, suffix)) = app.rsplit_once(" - ") {
        suffix.to_string()
    } else {
        app.to_string()
    }
}

fn infer_action_type(control_type: &str) -> &'static str {
    match control_type {
        "Button" | "MenuItem" | "TabItem" | "Hyperlink" => "click",
        "Edit" => "type",
        "Text" => "read",
        "ListItem" | "TreeItem" | "RadioButton" => "select",
        "CheckBox" => "toggle",
        _ => "interact",
    }
}

fn truncate(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        value.to_string()
    } else {
        value.chars().take(max_chars).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderMap, HeaderValue};
    use chrono::TimeZone;

    #[test]
    fn floors_to_five_minute_bucket() {
        let ts = Utc.with_ymd_and_hms(2026, 2, 11, 13, 17, 44).unwrap();
        let bucket = floor_to_five_minute_bucket(ts);
        assert_eq!(bucket.minute(), 15);
        assert_eq!(bucket.second(), 0);
    }

    #[test]
    fn infers_action_types() {
        assert_eq!(infer_action_type("Button"), "click");
        assert_eq!(infer_action_type("Edit"), "type");
        assert_eq!(infer_action_type("Unknown"), "interact");
    }

    #[test]
    fn request_auth_passes_without_configured_token() {
        let state = AppState {
            started_at: Instant::now(),
            stats: Arc::new(Mutex::new(IngestStats::default())),
            guard: Arc::new(PrivacyGuard::new("test-salt".to_string())),
            ingest_token: None,
        };
        let headers = HeaderMap::new();
        assert!(request_is_authorized(&state, &headers));
    }

    #[test]
    fn request_auth_accepts_matching_header_token() {
        let state = AppState {
            started_at: Instant::now(),
            stats: Arc::new(Mutex::new(IngestStats::default())),
            guard: Arc::new(PrivacyGuard::new("test-salt".to_string())),
            ingest_token: Some("secret".to_string()),
        };
        let mut headers = HeaderMap::new();
        headers.insert("x-collector-token", HeaderValue::from_static("secret"));
        assert!(request_is_authorized(&state, &headers));
    }

    #[test]
    fn request_auth_rejects_wrong_token() {
        let state = AppState {
            started_at: Instant::now(),
            stats: Arc::new(Mutex::new(IngestStats::default())),
            guard: Arc::new(PrivacyGuard::new("test-salt".to_string())),
            ingest_token: Some("secret".to_string()),
        };
        let mut headers = HeaderMap::new();
        headers.insert("x-collector-token", HeaderValue::from_static("wrong"));
        assert!(!request_is_authorized(&state, &headers));
    }
}
