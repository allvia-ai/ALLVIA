//! Session Store - Clawdbot-style session persistence
//!
//! Ported from: clawdbot-main/src/agents/pi-embedded-runner/
//!
//! Features:
//! - JSON file-based session storage
//! - Session key management
//! - Resume capability
//! - Auto-save on each step

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

// =====================================================
// SESSION DATA STRUCTURES
// =====================================================

/// A single message in the conversation history
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMessage {
    pub role: String, // "user", "assistant", "system", "tool"
    pub content: String,
    pub timestamp: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
}

/// A recorded action step
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionStep {
    pub step_index: usize,
    pub action_type: String,
    pub description: String,
    pub status: String, // "success", "failed", "pending"
    pub timestamp: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

/// Complete session state
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub key: String, // Channel-based key for session isolation
    pub goal: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub status: SessionStatus,
    pub messages: Vec<SessionMessage>,
    pub steps: Vec<SessionStep>,
    pub metadata: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    Active,
    Paused,
    Completed,
    Failed,
    Aborted,
}

impl Session {
    pub fn new(goal: &str, key: Option<&str>) -> Self {
        let id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now();
        Session {
            id: id.clone(),
            key: key.unwrap_or(&id).to_string(),
            goal: goal.to_string(),
            created_at: now,
            updated_at: now,
            status: SessionStatus::Active,
            messages: Vec::new(),
            steps: Vec::new(),
            metadata: HashMap::new(),
        }
    }

    /// Add a message to the conversation
    pub fn add_message(&mut self, role: &str, content: &str) {
        self.messages.push(SessionMessage {
            role: role.to_string(),
            content: content.to_string(),
            timestamp: Utc::now(),
            tool_call_id: None,
            tool_name: None,
        });
        self.updated_at = Utc::now();
    }

    /// Add a tool result message
    pub fn add_tool_result(&mut self, tool_name: &str, tool_call_id: &str, result: &str) {
        self.messages.push(SessionMessage {
            role: "tool".to_string(),
            content: result.to_string(),
            timestamp: Utc::now(),
            tool_call_id: Some(tool_call_id.to_string()),
            tool_name: Some(tool_name.to_string()),
        });
        self.updated_at = Utc::now();
    }

    /// Record a step execution
    pub fn add_step(
        &mut self,
        action_type: &str,
        description: &str,
        status: &str,
        data: Option<serde_json::Value>,
    ) {
        let step_index = self.steps.len();
        self.steps.push(SessionStep {
            step_index,
            action_type: action_type.to_string(),
            description: description.to_string(),
            status: status.to_string(),
            timestamp: Utc::now(),
            data,
        });
        self.updated_at = Utc::now();
    }

    /// Get last N messages for context
    pub fn get_history(&self, n: usize) -> Vec<&SessionMessage> {
        let start = if self.messages.len() > n {
            self.messages.len() - n
        } else {
            0
        };
        self.messages[start..].iter().collect()
    }

    /// Get resume point (last successful step index + 1)
    pub fn get_resume_point(&self) -> usize {
        for (i, step) in self.steps.iter().rev().enumerate() {
            if step.status == "success" {
                return self.steps.len() - i;
            }
        }
        0
    }

    /// Check if session can be resumed
    pub fn can_resume(&self) -> bool {
        matches!(self.status, SessionStatus::Paused | SessionStatus::Failed)
            && !self.steps.is_empty()
    }
}

// =====================================================
// SESSION STORE
// =====================================================

/// Manages session persistence to disk
pub struct SessionStore {
    base_dir: PathBuf,
    sessions: HashMap<String, Session>,
}

impl SessionStore {
    /// Create a new session store
    pub fn new(base_dir: &Path) -> Result<Self> {
        fs::create_dir_all(base_dir).context("Failed to create session directory")?;

        let mut store = SessionStore {
            base_dir: base_dir.to_path_buf(),
            sessions: HashMap::new(),
        };

        // Load existing sessions
        store.load_all()?;

        Ok(store)
    }

    /// Get default session directory
    pub fn default_dir() -> PathBuf {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        PathBuf::from(home).join(".steer").join("sessions")
    }

    /// Create or get session by key
    pub fn get_or_create(&mut self, key: &str, goal: &str) -> &mut Session {
        if let Some(existing_id) = self
            .sessions
            .iter()
            .find_map(|(id, session)| (session.key == key).then(|| id.clone()))
        {
            // Safety: existing_id comes from current map keys.
            return self
                .sessions
                .get_mut(&existing_id)
                .expect("existing session id must be present");
        }

        let session = Session::new(goal, Some(key));
        let session_id = session.id.clone();
        self.sessions.insert(session_id.clone(), session);
        self.sessions
            .get_mut(&session_id)
            .expect("newly inserted session id must be present")
    }

    /// Get session by ID
    pub fn get(&self, id: &str) -> Option<&Session> {
        self.sessions.get(id)
    }

    /// Get mutable session by ID
    pub fn get_mut(&mut self, id: &str) -> Option<&mut Session> {
        self.sessions.get_mut(id)
    }

    /// Get session by key
    pub fn get_by_key(&self, key: &str) -> Option<&Session> {
        self.sessions.values().find(|s| s.key == key)
    }

    /// Save session to disk
    pub fn save(&self, session: &Session) -> Result<()> {
        let path = self.session_path(&session.id);
        let json = serde_json::to_string_pretty(session).context("Failed to serialize session")?;
        fs::write(&path, json).context(format!("Failed to write session to {}", path.display()))?;
        println!("💾 [Session] Saved: {}", session.id);
        Ok(())
    }

    /// Save all sessions
    pub fn save_all(&self) -> Result<()> {
        for session in self.sessions.values() {
            self.save(session)?;
        }
        Ok(())
    }

    /// Load single session from disk
    pub fn load(&mut self, id: &str) -> Result<Option<&Session>> {
        let path = self.session_path(id);
        if !path.exists() {
            return Ok(None);
        }

        let json = fs::read_to_string(&path)
            .context(format!("Failed to read session from {}", path.display()))?;
        let session: Session =
            serde_json::from_str(&json).context("Failed to parse session JSON")?;

        self.sessions.insert(id.to_string(), session);
        Ok(self.sessions.get(id))
    }

    /// Load all sessions from disk
    pub fn load_all(&mut self) -> Result<()> {
        if !self.base_dir.exists() {
            return Ok(());
        }

        for entry in fs::read_dir(&self.base_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().map(|e| e == "json").unwrap_or(false) {
                if let Ok(json) = fs::read_to_string(&path) {
                    if let Ok(session) = serde_json::from_str::<Session>(&json) {
                        self.sessions.insert(session.id.clone(), session);
                    }
                }
            }
        }

        println!("📂 [Session] Loaded {} sessions", self.sessions.len());
        Ok(())
    }

    /// List active sessions
    pub fn list_active(&self) -> Vec<&Session> {
        self.sessions
            .values()
            .filter(|s| matches!(s.status, SessionStatus::Active | SessionStatus::Paused))
            .collect()
    }

    /// List resumable sessions
    pub fn list_resumable(&self) -> Vec<&Session> {
        self.sessions.values().filter(|s| s.can_resume()).collect()
    }

    /// Delete session
    pub fn delete(&mut self, id: &str) -> Result<bool> {
        if self.sessions.remove(id).is_some() {
            let path = self.session_path(id);
            if path.exists() {
                fs::remove_file(&path)?;
            }
            return Ok(true);
        }
        Ok(false)
    }

    /// Clean old completed sessions
    pub fn cleanup_old(&mut self, max_age_days: i64) -> Result<usize> {
        let cutoff = Utc::now() - chrono::Duration::days(max_age_days);
        let to_delete: Vec<String> = self
            .sessions
            .iter()
            .filter(|(_, s)| {
                matches!(s.status, SessionStatus::Completed | SessionStatus::Aborted)
                    && s.updated_at < cutoff
            })
            .map(|(id, _)| id.clone())
            .collect();

        let count = to_delete.len();
        for id in to_delete {
            let _ = self.delete(&id);
        }

        Ok(count)
    }

    fn session_path(&self, id: &str) -> PathBuf {
        self.base_dir.join(format!("{}.json", id))
    }
}

// =====================================================
// GLOBAL SINGLETON (lazy_static)
// =====================================================

use std::sync::Mutex;

lazy_static::lazy_static! {
    static ref SESSION_STORE: Mutex<Option<SessionStore>> = Mutex::new(None);
}

/// Initialize the global session store
pub fn init_session_store() -> Result<()> {
    let dir = SessionStore::default_dir();
    let store = SessionStore::new(&dir)?;

    if let Ok(mut guard) = SESSION_STORE.lock() {
        *guard = Some(store);
    }

    Ok(())
}

/// Get the global session store
pub fn get_session_store() -> Result<std::sync::MutexGuard<'static, Option<SessionStore>>> {
    SESSION_STORE
        .lock()
        .map_err(|_| anyhow::anyhow!("Failed to lock session store"))
}

/// Quick helper: save current session
pub fn save_session(session: &Session) -> Result<()> {
    let guard = get_session_store()?;
    if let Some(store) = guard.as_ref() {
        store.save(session)?;
    }
    Ok(())
}

// =====================================================
// TESTS
// =====================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_session_creation() {
        let session = Session::new("Test goal", Some("test-key"));
        assert_eq!(session.goal, "Test goal");
        assert_eq!(session.key, "test-key");
        assert_eq!(session.status, SessionStatus::Active);
    }

    #[test]
    fn test_session_messages() {
        let mut session = Session::new("Test", None);
        session.add_message("user", "Hello");
        session.add_message("assistant", "Hi there!");

        assert_eq!(session.messages.len(), 2);
        assert_eq!(session.messages[0].role, "user");
        assert_eq!(session.messages[1].content, "Hi there!");
    }

    #[test]
    fn test_session_store() -> Result<()> {
        let dir = tempdir()?;
        let mut store = SessionStore::new(dir.path())?;

        let session = store.get_or_create("test-key", "Test goal");
        let id = session.id.clone();

        session.add_message("user", "Hello");
        store.save(store.get(&id).unwrap())?;

        // Reload
        let store2 = SessionStore::new(dir.path())?;
        let loaded = store2.get(&id);
        assert!(loaded.is_some());
        assert_eq!(loaded.unwrap().messages.len(), 1);

        Ok(())
    }
}
