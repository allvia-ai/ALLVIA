use crate::db;
use crate::schema::AgentAction;
use crate::security;
use crate::shell_analysis;
use crate::tool_policy;
use std::env;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SecurityLevel {
    Safe,
    Caution,
    Critical,
}

pub struct PolicyEngine {
    pub write_lock: bool,
}

impl PolicyEngine {
    pub fn new() -> Self {
        Self { write_lock: true } // Default Locked
    }

    pub fn check(&self, action: &AgentAction) -> Result<(), String> {
        self.check_with_context(action, None)
    }

    pub fn check_with_context(
        &self,
        action: &AgentAction,
        cwd: Option<&str>,
    ) -> Result<(), String> {
        if !tool_policy::is_action_allowed(action) {
            return Err("Tool policy blocked this action.".to_string());
        }
        if let AgentAction::ShellExecution { command } = action {
            if !is_shell_command_allowed(command, cwd) {
                return Err("Shell command not in allowlist. Approval required.".to_string());
            }
        }

        let level = self.classify(action);
        match level {
            SecurityLevel::Safe => Ok(()),
            SecurityLevel::Caution => {
                if self.write_lock {
                    Err("Write Lock Engaged: Action requires approval.".to_string())
                } else {
                    Ok(())
                }
            }
            SecurityLevel::Critical => Err(
                "Critical Action: Requires explicit 2FA/Confirmation (Not implemented)."
                    .to_string(),
            ),
        }
    }

    fn classify(&self, action: &AgentAction) -> SecurityLevel {
        match action {
            AgentAction::UiSnapshot { .. }
            | AgentAction::UiFind { .. }
            | AgentAction::SystemSearch { .. } => SecurityLevel::Safe,
            AgentAction::UiClick { .. }
            | AgentAction::UiClickText { .. }
            | AgentAction::KeyboardType { .. } => SecurityLevel::Caution,
            AgentAction::UiType { .. } => SecurityLevel::Caution,
            AgentAction::SystemOpen { .. } => SecurityLevel::Caution,
            AgentAction::ShellExecution { command } => {
                match security::CommandClassifier::classify(command) {
                    security::SafetyLevel::Critical => SecurityLevel::Critical,
                    _ => SecurityLevel::Caution,
                }
            }
            AgentAction::Terminate => SecurityLevel::Critical,
            AgentAction::DebugFakeLog => SecurityLevel::Safe,
        }
    }

    pub fn unlock(&mut self) {
        self.write_lock = false;
        println!("[Policy] Write Lock UNLOCKED.");
    }

    pub fn lock(&mut self) {
        self.write_lock = true;
        println!("[Policy] Write Lock ENGAGED.");
    }
}

fn is_shell_command_allowed(command: &str, cwd: Option<&str>) -> bool {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return false;
    }

    let analysis = shell_analysis::analyze_shell_command(trimmed);
    if analysis.has_substitution && env_bool("SHELL_ALLOW_SUBSTITUTION", false) == false {
        return false;
    }
    if analysis.has_composites && env_bool("SHELL_ALLOW_COMPOSITES", false) == false {
        return false;
    }

    let denylist = parse_list(&env::var("SHELL_DENYLIST").unwrap_or_default());
    if denylist.iter().any(|d| trimmed.contains(d)) {
        return false;
    }

    let mut allowlist = default_allowlist();
    allowlist.extend(parse_list(&env::var("SHELL_ALLOWLIST").unwrap_or_default()));

    if allowlist.iter().any(|a| a == "*" || a == "all") {
        return true;
    }

    if allowlist.is_empty() {
        return true;
    }

    let segments = if analysis.segments.is_empty() {
        vec![trimmed.to_string()]
    } else {
        analysis.segments
    };
    for segment in segments {
        let allowed_by_list = allowlist
            .iter()
            .any(|a| segment == *a || segment.starts_with(a));
        let allowed_by_db = db::is_exec_allowlisted(&segment, cwd).unwrap_or(false);
        if !allowed_by_list && !allowed_by_db {
            return false;
        }
    }
    true
}

fn parse_list(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

fn default_allowlist() -> Vec<String> {
    vec![
        "ls".to_string(),
        "pwd".to_string(),
        "whoami".to_string(),
        "date".to_string(),
        "uptime".to_string(),
        "df".to_string(),
        "du".to_string(),
        "ps".to_string(),
        "cat".to_string(),
        "rg".to_string(),
        "sed".to_string(),
        "head".to_string(),
        "tail".to_string(),
        "git status".to_string(),
        "git diff".to_string(),
        "git log".to_string(),
    ]
}

fn env_bool(key: &str, default_val: bool) -> bool {
    match env::var(key) {
        Ok(v) => {
            let v = v.trim().to_lowercase();
            matches!(v.as_str(), "1" | "true" | "yes" | "on")
        }
        Err(_) => default_val,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::AgentAction;

    #[test]
    fn test_safe_action_allowed() {
        let policy = PolicyEngine::new();
        let action = AgentAction::UiSnapshot { scope: None };
        assert!(policy.check(&action).is_ok());
    }

    #[test]
    fn test_caution_action_blocked_when_locked() {
        let policy = PolicyEngine::new(); // Default locked
        let action = AgentAction::UiClick {
            element_id: "btn".to_string(),
            double_click: false,
        };
        assert!(policy.check(&action).is_err());
    }

    #[test]
    fn test_caution_action_allowed_when_unlocked() {
        let mut policy = PolicyEngine::new();
        policy.unlock();
        let action = AgentAction::UiClick {
            element_id: "btn".to_string(),
            double_click: false,
        };
        assert!(policy.check(&action).is_ok());
    }

    #[test]
    fn test_dangerous_shell_blocked() {
        let policy = PolicyEngine::new();
        let action = AgentAction::ShellExecution {
            command: "rm -rf /".to_string(),
        };
        // Should be Critical
        assert!(policy.check(&action).is_err());
    }
}
