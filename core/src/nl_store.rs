use crate::nl_automation::{IntentResult, Plan, SlotMap};
use lazy_static::lazy_static;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{Mutex, Once};
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionState {
    pub session_id: String,
    pub intent: IntentResult,
    pub slots: SlotMap,
    pub plan_id: Option<String>,
    pub prompt: String,
    pub last_used_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct PersistedNlStore {
    version: u32,
    sessions: HashMap<String, SessionState>,
    plans: HashMap<String, Plan>,
    plan_progress: HashMap<String, usize>,
}

lazy_static! {
    static ref SESSION_STORE: Mutex<HashMap<String, SessionState>> = Mutex::new(HashMap::new());
    static ref PLAN_STORE: Mutex<HashMap<String, Plan>> = Mutex::new(HashMap::new());
    static ref PLAN_PROGRESS: Mutex<HashMap<String, usize>> = Mutex::new(HashMap::new());
}
static STORE_INIT: Once = Once::new();

pub fn create_session(intent: IntentResult, slots: SlotMap, prompt: String) -> SessionState {
    ensure_loaded();
    let session_id = Uuid::new_v4().to_string();
    let now = unix_ts();
    let state = SessionState {
        session_id: session_id.clone(),
        intent,
        slots,
        plan_id: None,
        prompt,
        last_used_at: now,
    };
    if let Ok(mut store) = SESSION_STORE.lock() {
        cleanup_expired_sessions(&mut store);
        store.insert(session_id.clone(), state.clone());
    }
    persist_snapshot();
    state
}

pub fn get_session(session_id: &str) -> Option<SessionState> {
    ensure_loaded();
    let result = {
        let mut store = SESSION_STORE.lock().ok()?;
        cleanup_expired_sessions(&mut store);
        let mut state = store.get(session_id).cloned()?;
        state.last_used_at = unix_ts();
        store.insert(session_id.to_string(), state.clone());
        Some(state)
    };
    persist_snapshot();
    result
}

pub fn update_session_slots(session_id: &str, updates: &SlotMap) -> Option<SessionState> {
    ensure_loaded();
    let result = {
        let mut store = SESSION_STORE.lock().ok()?;
        cleanup_expired_sessions(&mut store);
        let state = store.get_mut(session_id)?;
        for (k, v) in updates {
            state.slots.insert(k.clone(), v.clone());
        }
        state.last_used_at = unix_ts();
        Some(state.clone())
    };
    persist_snapshot();
    result
}

pub fn set_session_plan(session_id: &str, plan: Plan) -> Option<SessionState> {
    ensure_loaded();
    let result = {
        let mut store = SESSION_STORE.lock().ok()?;
        cleanup_expired_sessions(&mut store);
        let state = store.get_mut(session_id)?;
        state.plan_id = Some(plan.plan_id.clone());
        state.last_used_at = unix_ts();
        if let Ok(mut plans) = PLAN_STORE.lock() {
            plans.insert(plan.plan_id.clone(), plan);
        }
        if let Ok(mut progress) = PLAN_PROGRESS.lock() {
            progress.insert(state.plan_id.clone().unwrap_or_default(), 0);
        }
        Some(state.clone())
    };
    persist_snapshot();
    result
}

pub fn get_plan(plan_id: &str) -> Option<Plan> {
    ensure_loaded();
    PLAN_STORE
        .lock()
        .ok()
        .and_then(|store| store.get(plan_id).cloned())
}

pub fn find_session_by_plan(plan_id: &str) -> Option<SessionState> {
    ensure_loaded();
    let result = {
        let mut store = SESSION_STORE.lock().ok()?;
        cleanup_expired_sessions(&mut store);
        store
            .values()
            .find(|state| state.plan_id.as_deref() == Some(plan_id))
            .cloned()
    };
    persist_snapshot();
    result
}

pub fn get_plan_progress(plan_id: &str) -> Option<usize> {
    ensure_loaded();
    PLAN_PROGRESS
        .lock()
        .ok()
        .and_then(|store| store.get(plan_id).cloned())
}

pub fn set_plan_progress(plan_id: &str, next_step: usize) {
    ensure_loaded();
    if let Ok(mut store) = PLAN_PROGRESS.lock() {
        store.insert(plan_id.to_string(), next_step);
    }
    persist_snapshot();
}

pub fn clear_plan_progress(plan_id: &str) {
    ensure_loaded();
    if let Ok(mut store) = PLAN_PROGRESS.lock() {
        store.remove(plan_id);
    }
    persist_snapshot();
}

fn unix_ts() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn session_ttl_seconds() -> i64 {
    std::env::var("STEER_NL_SESSION_TTL_SECONDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(3600)
}

fn cleanup_expired_sessions(store: &mut HashMap<String, SessionState>) {
    let ttl = session_ttl_seconds();
    if ttl <= 0 {
        return;
    }
    let now = unix_ts();
    let mut expired_plans: Vec<String> = Vec::new();
    store.retain(|_, state| {
        let expired = now.saturating_sub(state.last_used_at) > ttl;
        if expired {
            if let Some(plan_id) = state.plan_id.as_ref() {
                expired_plans.push(plan_id.clone());
            }
        }
        !expired
    });
    if !expired_plans.is_empty() {
        if let Ok(mut plans) = PLAN_STORE.lock() {
            for plan_id in &expired_plans {
                plans.remove(plan_id);
            }
        }
        if let Ok(mut progress) = PLAN_PROGRESS.lock() {
            for plan_id in &expired_plans {
                progress.remove(plan_id);
            }
        }
    }
}

fn store_path() -> PathBuf {
    if let Ok(value) = std::env::var("STEER_NL_STORE_PATH") {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }
    if let Some(mut path) = dirs::data_local_dir() {
        path.push("steer");
        let _ = fs::create_dir_all(&path);
        path.push("nl_store.json");
        return path;
    }
    PathBuf::from("nl_store.json")
}

fn ensure_loaded() {
    STORE_INIT.call_once(|| {
        let path = store_path();
        let Ok(raw) = fs::read_to_string(&path) else {
            return;
        };
        let Ok(snapshot) = serde_json::from_str::<PersistedNlStore>(&raw) else {
            eprintln!("⚠️ Failed to parse nl_store snapshot: {}", path.display());
            return;
        };
        if let Ok(mut sessions) = SESSION_STORE.lock() {
            *sessions = snapshot.sessions;
        }
        if let Ok(mut plans) = PLAN_STORE.lock() {
            *plans = snapshot.plans;
        }
        if let Ok(mut progress) = PLAN_PROGRESS.lock() {
            *progress = snapshot.plan_progress;
        }
    });
}

fn persist_snapshot() {
    let snapshot = {
        let sessions = match SESSION_STORE.lock() {
            Ok(v) => v.clone(),
            Err(_) => return,
        };
        let plans = match PLAN_STORE.lock() {
            Ok(v) => v.clone(),
            Err(_) => return,
        };
        let progress = match PLAN_PROGRESS.lock() {
            Ok(v) => v.clone(),
            Err(_) => return,
        };
        PersistedNlStore {
            version: 1,
            sessions,
            plans,
            plan_progress: progress,
        }
    };
    let path = store_path();
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            let _ = fs::create_dir_all(parent);
        }
    }
    let Ok(serialized) = serde_json::to_string_pretty(&snapshot) else {
        return;
    };
    let tmp = path.with_extension("json.tmp");
    if fs::write(&tmp, serialized).is_ok() {
        let _ = fs::rename(&tmp, &path);
    }
}
