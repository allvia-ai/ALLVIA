// Subagent Manager - Ported from clawdbot-main/src/agents/subagent-registry.ts
// Provides multi-agent spawning and orchestration

use lazy_static::lazy_static;
use std::collections::HashMap;
use std::sync::Mutex;

// =====================================================
// Subagent Types
// =====================================================

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubagentStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone)]
pub struct Subagent {
    pub id: String,
    pub name: String,
    pub task: String,
    pub status: SubagentStatus,
    pub result: Option<String>,
    pub error: Option<String>,
    pub created_at: std::time::Instant,
    pub completed_at: Option<std::time::Instant>,
}

impl Subagent {
    pub fn new(name: &str, task: &str) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.to_string(),
            task: task.to_string(),
            status: SubagentStatus::Pending,
            result: None,
            error: None,
            created_at: std::time::Instant::now(),
            completed_at: None,
        }
    }

    pub fn duration_ms(&self) -> u64 {
        self.completed_at
            .map(|c| c.duration_since(self.created_at).as_millis() as u64)
            .unwrap_or_else(|| self.created_at.elapsed().as_millis() as u64)
    }
}

// =====================================================
// Subagent Registry (singleton)
// =====================================================

lazy_static! {
    static ref SUBAGENT_REGISTRY: Mutex<HashMap<String, Subagent>> = Mutex::new(HashMap::new());
}

pub struct SubagentManager;

impl SubagentManager {
    /// Spawn a new subagent for a task
    pub fn spawn(name: &str, task: &str) -> String {
        let agent = Subagent::new(name, task);
        let id = agent.id.clone();

        if let Ok(mut registry) = SUBAGENT_REGISTRY.lock() {
            registry.insert(id.clone(), agent);
        }

        println!("🤖 [Subagent] Spawned '{}' with task: {}", name, task);
        id
    }

    /// Get subagent by ID
    pub fn get(id: &str) -> Option<Subagent> {
        SUBAGENT_REGISTRY
            .lock()
            .ok()
            .and_then(|r| r.get(id).cloned())
    }

    /// Update subagent status
    pub fn update_status(
        id: &str,
        status: SubagentStatus,
        result: Option<String>,
        error: Option<String>,
    ) {
        if let Ok(mut registry) = SUBAGENT_REGISTRY.lock() {
            if let Some(agent) = registry.get_mut(id) {
                agent.status = status.clone();
                agent.result = result;
                agent.error = error;

                if matches!(
                    status,
                    SubagentStatus::Completed | SubagentStatus::Failed | SubagentStatus::Cancelled
                ) {
                    agent.completed_at = Some(std::time::Instant::now());
                }
            }
        }
    }

    /// Mark as running
    pub fn start(id: &str) {
        Self::update_status(id, SubagentStatus::Running, None, None);
    }

    /// Mark as completed with result
    pub fn complete(id: &str, result: &str) {
        Self::update_status(
            id,
            SubagentStatus::Completed,
            Some(result.to_string()),
            None,
        );
        println!("✅ [Subagent] {} completed", id);
    }

    /// Mark as failed
    pub fn fail(id: &str, error: &str) {
        Self::update_status(id, SubagentStatus::Failed, None, Some(error.to_string()));
        println!("❌ [Subagent] {} failed: {}", id, error);
    }

    /// Cancel a running subagent
    pub fn cancel(id: &str) {
        Self::update_status(
            id,
            SubagentStatus::Cancelled,
            None,
            Some("Cancelled by user".to_string()),
        );
        println!("🛑 [Subagent] {} cancelled", id);
    }

    /// List all active (pending/running) subagents
    pub fn list_active() -> Vec<Subagent> {
        SUBAGENT_REGISTRY
            .lock()
            .map(|r| {
                r.values()
                    .filter(|a| {
                        matches!(a.status, SubagentStatus::Pending | SubagentStatus::Running)
                    })
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// List all subagents
    pub fn list_all() -> Vec<Subagent> {
        SUBAGENT_REGISTRY
            .lock()
            .map(|r| r.values().cloned().collect())
            .unwrap_or_default()
    }

    /// Wait for a subagent to complete (with timeout)
    pub async fn wait_for(id: &str, timeout_ms: u64) -> Option<Subagent> {
        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_millis(timeout_ms);

        loop {
            if let Some(agent) = Self::get(id) {
                if matches!(
                    agent.status,
                    SubagentStatus::Completed | SubagentStatus::Failed | SubagentStatus::Cancelled
                ) {
                    return Some(agent);
                }
            }

            if start.elapsed() > timeout {
                return None;
            }

            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }

    /// Clear completed/failed agents older than specified duration
    pub fn cleanup_old(max_age_secs: u64) {
        if let Ok(mut registry) = SUBAGENT_REGISTRY.lock() {
            let cutoff = std::time::Duration::from_secs(max_age_secs);
            registry.retain(|_, a| {
                if matches!(
                    a.status,
                    SubagentStatus::Completed | SubagentStatus::Failed | SubagentStatus::Cancelled
                ) {
                    a.created_at.elapsed() < cutoff
                } else {
                    true // Keep active agents
                }
            });
        }
    }
}

// =====================================================
// LLM Action Integration
// =====================================================

/// Parse spawn_agent action from LLM
pub fn parse_spawn_action(action: &serde_json::Value) -> Option<(String, String)> {
    let name = action
        .get("name")
        .or_else(|| action.get("agent_name"))
        .and_then(|v| v.as_str())
        .unwrap_or("worker");

    let task = action.get("task").and_then(|v| v.as_str())?;

    Some((name.to_string(), task.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spawn_and_complete() {
        let id = SubagentManager::spawn("test_agent", "Do something");

        let agent = SubagentManager::get(&id).unwrap();
        assert_eq!(agent.status, SubagentStatus::Pending);

        SubagentManager::start(&id);
        let agent = SubagentManager::get(&id).unwrap();
        assert_eq!(agent.status, SubagentStatus::Running);

        SubagentManager::complete(&id, "Done!");
        let agent = SubagentManager::get(&id).unwrap();
        assert_eq!(agent.status, SubagentStatus::Completed);
        assert_eq!(agent.result, Some("Done!".to_string()));
    }
}
