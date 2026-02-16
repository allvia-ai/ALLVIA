use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static LOCK_ACQUIRED_COUNT: AtomicU64 = AtomicU64::new(0);
static LOCK_BYPASS_COUNT: AtomicU64 = AtomicU64::new(0);
static LOCK_BLOCKED_COUNT: AtomicU64 = AtomicU64::new(0);
static LOCK_STALE_RECOVERED_COUNT: AtomicU64 = AtomicU64::new(0);
static LOCK_REJECTED_COUNT: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LockPayload {
    pid: u32,
    created_at: String,
    scope: String,
    start_fingerprint: Option<String>,
}

#[derive(Debug)]
pub struct LockGuard {
    path: PathBuf,
}

#[derive(Debug, Clone, Serialize)]
pub struct LockMetricsSnapshot {
    pub acquired: u64,
    pub bypassed: u64,
    pub blocked: u64,
    pub stale_recovered: u64,
    pub rejected: u64,
}

pub fn lock_metrics_snapshot() -> LockMetricsSnapshot {
    LockMetricsSnapshot {
        acquired: LOCK_ACQUIRED_COUNT.load(Ordering::Relaxed),
        bypassed: LOCK_BYPASS_COUNT.load(Ordering::Relaxed),
        blocked: LOCK_BLOCKED_COUNT.load(Ordering::Relaxed),
        stale_recovered: LOCK_STALE_RECOVERED_COUNT.load(Ordering::Relaxed),
        rejected: LOCK_REJECTED_COUNT.load(Ordering::Relaxed),
    }
}

enum LockMode {
    Enforce,
    Bypass,
    Reject(String),
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

pub fn acquire_lock() -> Result<Option<LockGuard>, String> {
    let allow_multi = env_flag("STEER_ALLOW_MULTI");
    let lock_disabled = env_flag("STEER_LOCK_DISABLED");
    let explicit_disable_flag = env_flag("STEER_ALLOW_LOCK_DISABLE");
    let test_context = env_flag("STEER_TEST_MODE") || env_flag("CI");
    let allow_lock_disabled_non_test = env_flag("STEER_ALLOW_LOCK_DISABLED_NON_TEST");
    match decide_lock_mode(
        allow_multi,
        lock_disabled,
        explicit_disable_flag,
        test_context,
        allow_lock_disabled_non_test,
    ) {
        LockMode::Reject(msg) => {
            LOCK_REJECTED_COUNT.fetch_add(1, Ordering::Relaxed);
            return Err(msg);
        }
        LockMode::Bypass => {
            LOCK_BYPASS_COUNT.fetch_add(1, Ordering::Relaxed);
            crate::diagnostic_events::emit(
                "singleton_lock.bypass",
                serde_json::json!({
                    "allow_multi": allow_multi,
                    "lock_disabled": lock_disabled,
                    "test_context": test_context,
                    "allow_lock_disabled_non_test": allow_lock_disabled_non_test
                }),
            );
            return Ok(None);
        }
        LockMode::Enforce => {}
    }

    if lock_disabled && !test_context && !allow_lock_disabled_non_test {
        return Err(
            "STEER_LOCK_DISABLED is test-only (requires STEER_TEST_MODE=1 or CI=1). \
Set STEER_ALLOW_LOCK_DISABLED_NON_TEST=1 to override explicitly."
                .to_string(),
        );
    }

    if allow_multi || (lock_disabled && (test_context || allow_lock_disabled_non_test)) {
        LOCK_BYPASS_COUNT.fetch_add(1, Ordering::Relaxed);
        crate::diagnostic_events::emit(
            "singleton_lock.bypass",
            serde_json::json!({
                "allow_multi": allow_multi,
                "lock_disabled": lock_disabled,
                "test_context": test_context,
                "allow_lock_disabled_non_test": allow_lock_disabled_non_test
            }),
        );
        return Ok(None);
    }

    let lock_scope = resolve_lock_scope();
    let lock_path = resolve_lock_path(&lock_scope);
    if let Some(parent) = lock_path.parent() {
        if let Err(err) = fs::create_dir_all(parent) {
            return Err(format!("Failed to create lock dir: {}", err));
        }
    }

    let stale_secs = env_i64("STEER_LOCK_STALE_SECS").unwrap_or(900);

    // bounded retries when cleaning stale/recycled-owner lock files.
    for _ in 0..3 {
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock_path)
        {
            Ok(mut file) => {
                let payload = LockPayload {
                    pid: std::process::id(),
                    created_at: chrono::Utc::now().to_rfc3339(),
                    scope: lock_scope.clone(),
                    start_fingerprint: process_start_fingerprint(std::process::id()),
                };
                let json = serde_json::to_string(&payload).unwrap_or_else(|_| "{}".to_string());
                if let Err(err) = file.write_all(json.as_bytes()) {
                    return Err(format!("Failed to write lock file: {}", err));
                }
                LOCK_ACQUIRED_COUNT.fetch_add(1, Ordering::Relaxed);
                crate::diagnostic_events::emit(
                    "singleton_lock.acquired",
                    serde_json::json!({
                        "scope": lock_scope,
                        "path": lock_path.to_string_lossy().to_string(),
                        "pid": std::process::id()
                    }),
                );
                return Ok(Some(LockGuard { path: lock_path }));
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                if let Some(payload) = read_payload(&lock_path) {
                    let owner_dead = !process_alive(payload.pid);
                    let owner_reused = owner_pid_reused(&payload);
                    let stale =
                        is_stale(&payload, stale_secs) || lock_file_stale(&lock_path, stale_secs);
                    if owner_dead || owner_reused || stale {
                        LOCK_STALE_RECOVERED_COUNT.fetch_add(1, Ordering::Relaxed);
                        crate::diagnostic_events::emit(
                            "singleton_lock.stale_recovered",
                            serde_json::json!({
                                "scope": lock_scope,
                                "path": lock_path.to_string_lossy().to_string(),
                                "owner_pid": payload.pid,
                                "owner_dead": owner_dead,
                                "owner_reused": owner_reused,
                                "stale": stale
                            }),
                        );
                        let _ = fs::remove_file(&lock_path);
                        continue;
                    }
                    LOCK_BLOCKED_COUNT.fetch_add(1, Ordering::Relaxed);
                    crate::diagnostic_events::emit(
                        "singleton_lock.blocked",
                        serde_json::json!({
                            "scope": lock_scope,
                            "path": lock_path.to_string_lossy().to_string(),
                            "owner_pid": payload.pid
                        }),
                    );
                    return Err(format!(
                        "Another instance is already running (pid {}, since {}, scope {}).",
                        payload.pid, payload.created_at, payload.scope
                    ));
                }
                LOCK_BLOCKED_COUNT.fetch_add(1, Ordering::Relaxed);
                crate::diagnostic_events::emit(
                    "singleton_lock.blocked",
                    serde_json::json!({
                        "scope": lock_scope,
                        "path": lock_path.to_string_lossy().to_string(),
                        "owner_pid": serde_json::Value::Null
                    }),
                );
                return Err("Another instance is already running (lock exists).".to_string());
            }
            Err(err) => return Err(format!("Failed to acquire lock: {}", err)),
        }
    }

    Err("Failed to acquire lock after stale cleanup retries.".to_string())
}

fn decide_lock_mode(
    allow_multi: bool,
    lock_disabled: bool,
    explicit_disable_flag: bool,
    test_context: bool,
    allow_lock_disabled_non_test: bool,
) -> LockMode {
    if explicit_disable_flag && !test_context && !allow_lock_disabled_non_test {
        return LockMode::Reject(
            "STEER_ALLOW_LOCK_DISABLE is test-only (requires STEER_TEST_MODE=1 or CI=1)"
                .to_string(),
        );
    }
    if allow_multi && !test_context {
        return LockMode::Reject(
            "STEER_ALLOW_MULTI is test-only (requires STEER_TEST_MODE=1 or CI=1)".to_string(),
        );
    }
    if allow_multi || (lock_disabled && (test_context || allow_lock_disabled_non_test)) {
        return LockMode::Bypass;
    }
    LockMode::Enforce
}

fn lock_file_stale(path: &Path, stale_secs: i64) -> bool {
    if stale_secs <= 0 {
        return false;
    }
    let Ok(meta) = fs::metadata(path) else {
        return false;
    };
    let Ok(modified) = meta.modified() else {
        return false;
    };
    let modified: chrono::DateTime<chrono::Utc> = modified.into();
    let age = chrono::Utc::now()
        .signed_duration_since(modified)
        .num_seconds();
    age > stale_secs
}

fn owner_pid_reused(payload: &LockPayload) -> bool {
    let Some(expected) = payload.start_fingerprint.as_ref() else {
        return false;
    };
    let Some(current) = process_start_fingerprint(payload.pid) else {
        return false;
    };
    current.trim() != expected.trim()
}

fn process_start_fingerprint(pid: u32) -> Option<String> {
    if pid == 0 {
        return None;
    }
    let out = Command::new("ps")
        .arg("-o")
        .arg("lstart=")
        .arg("-p")
        .arg(pid.to_string())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
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

fn resolve_lock_scope() -> String {
    if let Ok(explicit) = std::env::var("STEER_LOCK_SCOPE") {
        let trimmed = explicit.trim();
        if !trimmed.is_empty() {
            return sanitize_scope(trimmed);
        }
    }
    let base = std::env::current_dir()
        .ok()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| "default".to_string());
    let mut hasher = Sha256::new();
    hasher.update(base.as_bytes());
    let digest = hasher.finalize();
    format!("cwd-{:x}", digest)[..16].to_string()
}

fn sanitize_scope(input: &str) -> String {
    let mut out = String::new();
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    let trimmed = out.trim_matches('_');
    if trimmed.is_empty() {
        "default".to_string()
    } else {
        trimmed.to_string()
    }
}

fn resolve_lock_path(scope: &str) -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    Path::new(&home)
        .join(".steer")
        .join("locks")
        .join(format!("steer.{}.lock", sanitize_scope(scope)))
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

fn process_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    #[test]
    fn sanitize_scope_replaces_unsafe_chars() {
        let got = sanitize_scope("a/b:c?d e");
        assert_eq!(got, "a_b_c_d_e");
    }

    #[test]
    fn resolve_lock_path_scoped_file_name() {
        let p = resolve_lock_path("project-A");
        let s = p.to_string_lossy();
        assert!(s.contains(".steer/locks/steer.project-A.lock"));
    }

    #[test]
    fn decide_lock_mode_allows_non_test_override_for_lock_disabled() {
        let mode = decide_lock_mode(false, true, false, false, true);
        assert!(matches!(mode, LockMode::Bypass));
    }

    #[test]
    fn decide_lock_mode_rejects_allow_multi_outside_test() {
        let mode = decide_lock_mode(true, false, false, false, true);
        assert!(matches!(mode, LockMode::Reject(_)));
    }

    #[test]
    fn lock_metrics_snapshot_reads_atomic_counters() {
        LOCK_ACQUIRED_COUNT.store(0, Ordering::Relaxed);
        LOCK_BYPASS_COUNT.store(0, Ordering::Relaxed);
        LOCK_BLOCKED_COUNT.store(0, Ordering::Relaxed);
        LOCK_STALE_RECOVERED_COUNT.store(0, Ordering::Relaxed);
        LOCK_REJECTED_COUNT.store(0, Ordering::Relaxed);

        LOCK_ACQUIRED_COUNT.fetch_add(2, Ordering::Relaxed);
        LOCK_BYPASS_COUNT.fetch_add(1, Ordering::Relaxed);
        LOCK_BLOCKED_COUNT.fetch_add(3, Ordering::Relaxed);
        LOCK_STALE_RECOVERED_COUNT.fetch_add(4, Ordering::Relaxed);
        LOCK_REJECTED_COUNT.fetch_add(5, Ordering::Relaxed);

        let snapshot = lock_metrics_snapshot();
        assert_eq!(snapshot.acquired, 2);
        assert_eq!(snapshot.bypassed, 1);
        assert_eq!(snapshot.blocked, 3);
        assert_eq!(snapshot.stale_recovered, 4);
        assert_eq!(snapshot.rejected, 5);
    }
}
