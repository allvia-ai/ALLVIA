use crate::schema::EventEnvelope;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::Path;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SessionRecord {
    pub session_id: String,
    pub start_ts: String,
    pub end_ts: String,
    pub duration_sec: i64,
    pub summary: SessionSummary,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct SessionSummary {
    pub apps: HashMap<String, i64>,
    pub top_app: String,
    pub event_count: usize,
    pub key_events: Vec<String>, // [P2] Enrich Session Summary
    pub resources: Vec<String>,
    pub domains: Vec<String>,
}

pub struct Sessionizer {
    gap_seconds: i64,
}

impl Sessionizer {
    pub fn new(gap_seconds: i64) -> Self {
        Self { gap_seconds }
    }

    pub fn sessionize(&self, events: &[EventEnvelope]) -> Vec<SessionRecord> {
        let mut sessions = Vec::new();
        if events.is_empty() {
            return sessions;
        }

        let mut current_chunk: Vec<&EventEnvelope> = Vec::new();
        let mut last_ts_val: Option<i64> = None;

        for event in events {
            let ts = parse_iso(&event.ts).unwrap_or(0);

            // Gap Check
            if let Some(last) = last_ts_val {
                if ts - last >= self.gap_seconds {
                    if !current_chunk.is_empty() {
                        sessions.push(self.build_session(&current_chunk));
                        current_chunk.clear();
                    }
                }
            }

            // Idle Check (Break session on idle)
            if event.event_type == "os.idle_start" {
                if !current_chunk.is_empty() {
                    sessions.push(self.build_session(&current_chunk));
                    current_chunk.clear();
                }
                last_ts_val = None; // Reset
                continue;
            }

            current_chunk.push(event);
            last_ts_val = Some(ts);
        }

        // Flush remaining
        if !current_chunk.is_empty() {
            sessions.push(self.build_session(&current_chunk));
        }

        sessions
    }

    fn build_session(&self, chunk: &[&EventEnvelope]) -> SessionRecord {
        let start = chunk
            .first()
            .expect("Chunk is explicitly checked to be non-empty");
        let end = chunk
            .last()
            .expect("Chunk is explicitly checked to be non-empty");

        let start_ts = parse_iso(&start.ts).unwrap_or(0);
        let end_ts = parse_iso(&end.ts).unwrap_or(0);
        let duration = end_ts - start_ts;

        let mut app_durations: HashMap<String, i64> = HashMap::new();
        let mut last_event_ts = start_ts;
        let mut last_app = start.app.clone();
        let mut key_events: HashSet<String> = HashSet::new();
        let mut resources: HashSet<String> = HashSet::new();
        let mut domains: HashSet<String> = HashSet::new();

        for event in chunk {
            let curr_ts = parse_iso(&event.ts).unwrap_or(0);
            let delta = curr_ts - last_event_ts;

            // Attribute delta to previous app
            *app_durations.entry(last_app.clone()).or_insert(0) += delta;

            last_event_ts = curr_ts;
            last_app = event.app.clone();

            // Key events (minimal, explainable signals)
            if event.event_type == "app_switch" || event.event_type == "system.open" {
                let app = event
                    .payload
                    .get("app")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&event.app);
                if !app.is_empty() {
                    key_events.insert(format!("app:{}", app));
                }
            }

            if event.event_type.starts_with("file_") {
                if let Some(path) = event.payload.get("path").and_then(|v| v.as_str()) {
                    if let Some(ext) = Path::new(path).extension().and_then(|e| e.to_str()) {
                        key_events.insert(format!("file:.{}", ext));
                    }
                    resources.insert(path.to_string());
                }
            }

            if let Some(res) = &event.resource {
                if !res.id.is_empty() {
                    resources.insert(res.id.clone());
                }
            }

            if let Some(url) = event.browser_url.as_ref() {
                if let Some(domain) = extract_domain(url) {
                    domains.insert(domain);
                }
            }
        }

        // Find top app
        let top_app = app_durations
            .iter()
            .max_by_key(|entry| entry.1)
            .map(|(k, _)| k.clone())
            .unwrap_or_else(|| "unknown".to_string());

        SessionRecord {
            session_id: uuid::Uuid::new_v4().to_string(),
            start_ts: start.ts.clone(),
            end_ts: end.ts.clone(),
            duration_sec: duration,
            summary: SessionSummary {
                apps: app_durations,
                top_app,
                event_count: chunk.len(),
                key_events: to_sorted_vec(key_events),
                resources: to_sorted_vec(resources),
                domains: to_sorted_vec(domains),
            },
        }
    }
}

fn parse_iso(iso: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(iso)
        .ok()
        .map(|dt| dt.timestamp())
}

fn to_sorted_vec(values: HashSet<String>) -> Vec<String> {
    let mut vec: Vec<String> = values.into_iter().collect();
    vec.sort();
    vec
}

fn extract_domain(url: &str) -> Option<String> {
    let trimmed = url.trim();
    let without_scheme = trimmed.split("://").nth(1).unwrap_or(trimmed);
    let host = without_scheme.split('/').next().unwrap_or("");
    if host.is_empty() {
        None
    } else {
        Some(host.split(':').next().unwrap_or(host).to_string())
    }
}
