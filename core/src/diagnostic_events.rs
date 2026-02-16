use serde_json::{json, Map, Value};
use std::fs::{create_dir_all, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use once_cell::sync::Lazy;

static DIAG_SEQ: AtomicU64 = AtomicU64::new(0);
static FILE_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

fn enabled() -> bool {
    std::env::var("STEER_DIAGNOSTIC_EVENTS")
        .ok()
        .map(|v| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(true)
}

fn default_path() -> PathBuf {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    cwd.join("scenario_results").join("diagnostic_events.jsonl")
}

fn output_path() -> PathBuf {
    if let Ok(raw) = std::env::var("STEER_DIAGNOSTIC_EVENTS_PATH") {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }
    default_path()
}

fn ensure_parent(path: &Path) {
    if let Some(parent) = path.parent() {
        let _ = create_dir_all(parent);
    }
}

pub fn emit(event_type: &str, payload: Value) {
    if !enabled() {
        return;
    }

    let seq = DIAG_SEQ.fetch_add(1, Ordering::Relaxed) + 1;
    let ts = chrono::Utc::now().to_rfc3339();

    let mut obj = Map::new();
    obj.insert("type".to_string(), Value::String(event_type.to_string()));
    obj.insert("seq".to_string(), json!(seq));
    obj.insert("ts".to_string(), Value::String(ts));

    match payload {
        Value::Object(map) => {
            for (k, v) in map {
                obj.insert(k, v);
            }
        }
        other => {
            obj.insert("payload".to_string(), other);
        }
    }

    let line = Value::Object(obj).to_string();
    let path = output_path();
    ensure_parent(&path);

    let _guard = FILE_LOCK.lock().ok();
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{}", line);
    }
}
