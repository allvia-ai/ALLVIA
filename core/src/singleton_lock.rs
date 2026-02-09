use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LockPayload {
    pid: u32,
    created_at: String,
}

#[derive(Debug)]
pub struct LockGuard {
    path: PathBuf,
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

pub fn acquire_lock() -> Result<Option<LockGuard>, String> {
    if env_flag("STEER_ALLOW_MULTI") || env_flag("STEER_LOCK_DISABLED") {
        return Ok(None);
    }

    let lock_path = resolve_lock_path();
    if let Some(parent) = lock_path.parent() {
        if let Err(err) = fs::create_dir_all(parent) {
            return Err(format!("Failed to create lock dir: {}", err));
        }
    }

    let stale_secs = env_i64("STEER_LOCK_STALE_SECS").unwrap_or(900);

    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&lock_path)
    {
        Ok(mut file) => {
            let payload = LockPayload {
                pid: std::process::id(),
                created_at: chrono::Utc::now().to_rfc3339(),
            };
            let json = serde_json::to_string(&payload).unwrap_or_else(|_| "{}".to_string());
            if let Err(err) = file.write_all(json.as_bytes()) {
                return Err(format!("Failed to write lock file: {}", err));
            }
            Ok(Some(LockGuard { path: lock_path }))
        }
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
            if let Some(payload) = read_payload(&lock_path) {
                if is_stale(&payload, stale_secs) {
                    let _ = fs::remove_file(&lock_path);
                    return acquire_lock();
                }
                return Err(format!(
                    "Another instance is already running (pid {}, since {}).",
                    payload.pid, payload.created_at
                ));
            }
            Err("Another instance is already running (lock exists).".to_string())
        }
        Err(err) => Err(format!("Failed to acquire lock: {}", err)),
    }
}

fn read_payload(path: &Path) -> Option<LockPayload> {
    let content = fs::read_to_string(path).ok()?;
    serde_json::from_str::<LockPayload>(&content).ok()
}

fn is_stale(payload: &LockPayload, stale_secs: i64) -> bool {
    if stale_secs <= 0 {
        return false;
    }
    let created = chrono::DateTime::parse_from_rfc3339(&payload.created_at)
        .map(|ts| ts.with_timezone(&chrono::Utc))
        .unwrap_or_else(|_| chrono::Utc::now());
    let age = chrono::Utc::now()
        .signed_duration_since(created)
        .num_seconds();
    age > stale_secs
}

fn resolve_lock_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    Path::new(&home).join(".steer").join("steer.lock")
}

fn env_flag(key: &str) -> bool {
    std::env::var(key)
        .ok()
        .map(|v| {
            matches!(
                v.trim().to_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

fn env_i64(key: &str) -> Option<i64> {
    std::env::var(key).ok().and_then(|v| v.parse::<i64>().ok())
}
