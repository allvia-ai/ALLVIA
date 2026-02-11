// Approval Gate - Ported from clawdbot-main/src/agents/bash-tools.exec.ts
// Provides command approval workflow for dangerous operations

use lazy_static::lazy_static;
use std::collections::HashSet;
use std::sync::Mutex;

// =====================================================
// Approval Types (from clawdbot)
// =====================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalLevel {
    /// Always auto-approve (safe commands)
    AutoApprove,
    /// Require user approval
    RequireApproval,
    /// Block entirely (never run)
    Blocked,
}

#[derive(Debug, Clone)]
pub struct ApprovalRequest {
    pub id: String,
    pub command: String,
    pub level: ApprovalLevel,
    pub reason: String,
    pub created_at: std::time::Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalStatus {
    Pending,
    Approved,
    Denied,
    Expired,
}

// =====================================================
// Dangerous Command Patterns (from clawdbot safeBins concept)
// =====================================================

lazy_static! {
    /// Commands that are always blocked
    static ref BLOCKED_PATTERNS: Vec<&'static str> = vec![
        "rm -rf /",
        "rm -rf /*",
        "rm -rf ~",
        "mkfs",
        ":(){:|:&};:",  // Fork bomb
        "dd if=/dev/zero",
        "chmod -R 777 /",
        "> /dev/sda",
    ];

    /// Commands that require approval
    static ref APPROVAL_PATTERNS: Vec<&'static str> = vec![
        "sudo",
        "rm -rf",
        "rm -r",
        "chmod",
        "chown",
        "kill -9",
        "killall",
        "shutdown",
        "reboot",
        "passwd",
        "curl | sh",
        "curl | bash",
        "wget | sh",
        "pip install",
        "npm install -g",
        "brew install",
        "apt install",
        "apt-get install",
    ];

    /// Safe commands (always auto-approve)
    static ref SAFE_BINS: HashSet<&'static str> = {
        let mut set = HashSet::new();
        set.insert("ls");
        set.insert("pwd");
        set.insert("echo");
        set.insert("cat");
        set.insert("head");
        set.insert("tail");
        set.insert("grep");
        set.insert("find");
        set.insert("which");
        set.insert("whoami");
        set.insert("date");
        set.insert("cal");
        set.insert("df");
        set.insert("du");
        set.insert("wc");
        set.insert("sort");
        set.insert("uniq");
        set.insert("diff");
        set.insert("env");
        set.insert("printenv");
        set
    };

    /// Pending approvals registry
    static ref PENDING_APPROVALS: Mutex<Vec<ApprovalRequest>> = Mutex::new(Vec::new());
}

// =====================================================
// Approval Gate Implementation
// =====================================================

pub struct ApprovalGate;

impl ApprovalGate {
    /// Check if a command requires approval, is blocked, or can auto-run
    pub fn check_command(cmd: &str) -> ApprovalLevel {
        let cmd_lower = cmd.to_lowercase();
        let cmd_trimmed = cmd.trim();

        // 1. Check blocked patterns first
        for pattern in BLOCKED_PATTERNS.iter() {
            if cmd_lower.contains(pattern) {
                return ApprovalLevel::Blocked;
            }
        }

        // 2. Check if it's a safe binary
        let first_word = cmd_trimmed.split_whitespace().next().unwrap_or("");
        let binary_name = std::path::Path::new(first_word)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(first_word);

        if SAFE_BINS.contains(binary_name) {
            return ApprovalLevel::AutoApprove;
        }

        // 3. Check approval patterns
        for pattern in APPROVAL_PATTERNS.iter() {
            if cmd_lower.contains(pattern) {
                return ApprovalLevel::RequireApproval;
            }
        }

        // 4. Default: require approval for unknown commands.
        ApprovalLevel::RequireApproval
    }

    /// Create an approval request for a command
    pub fn request_approval(cmd: &str) -> ApprovalRequest {
        let level = Self::check_command(cmd);
        let reason = match level {
            ApprovalLevel::Blocked => {
                "Command contains dangerous patterns and is blocked.".to_string()
            }
            ApprovalLevel::RequireApproval => format!("Command may modify system: '{}'", cmd),
            ApprovalLevel::AutoApprove => "Safe command.".to_string(),
        };

        let request = ApprovalRequest {
            id: uuid::Uuid::new_v4().to_string(),
            command: cmd.to_string(),
            level,
            reason,
            created_at: std::time::Instant::now(),
        };

        if level == ApprovalLevel::RequireApproval {
            if let Ok(mut pending) = PENDING_APPROVALS.lock() {
                pending.push(request.clone());
            }
        }

        request
    }

    /// Approve a pending request by ID
    pub fn approve(id: &str) -> bool {
        if let Ok(mut pending) = PENDING_APPROVALS.lock() {
            if let Some(pos) = pending.iter().position(|r| r.id == id) {
                pending.remove(pos);
                return true;
            }
        }
        false
    }

    /// Deny a pending request by ID
    pub fn deny(id: &str) -> bool {
        Self::approve(id) // Same logic - just remove from pending
    }

    /// Get all pending approvals
    pub fn get_pending() -> Vec<ApprovalRequest> {
        PENDING_APPROVALS
            .lock()
            .map(|p| p.clone())
            .unwrap_or_default()
    }

    /// Clear expired approvals (older than 2 minutes)
    pub fn clear_expired() {
        if let Ok(mut pending) = PENDING_APPROVALS.lock() {
            let expiry = std::time::Duration::from_secs(120);
            pending.retain(|r| r.created_at.elapsed() < expiry);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_blocked_commands() {
        assert_eq!(
            ApprovalGate::check_command("rm -rf /"),
            ApprovalLevel::Blocked
        );
        assert_eq!(
            ApprovalGate::check_command("rm -rf /*"),
            ApprovalLevel::Blocked
        );
    }

    #[test]
    fn test_safe_commands() {
        assert_eq!(
            ApprovalGate::check_command("ls -la"),
            ApprovalLevel::AutoApprove
        );
        assert_eq!(
            ApprovalGate::check_command("pwd"),
            ApprovalLevel::AutoApprove
        );
        assert_eq!(
            ApprovalGate::check_command("cat file.txt"),
            ApprovalLevel::AutoApprove
        );
    }

    #[test]
    fn test_approval_required() {
        assert_eq!(
            ApprovalGate::check_command("sudo apt install git"),
            ApprovalLevel::RequireApproval
        );
        assert_eq!(
            ApprovalGate::check_command("rm -rf mydir"),
            ApprovalLevel::RequireApproval
        );
    }
}

// =====================================================
// LEGACY API COMPATIBILITY (for api_server.rs, execution_controller.rs)
// =====================================================

/// Decision result expected by execution_controller
#[derive(Debug, Clone)]
pub struct ApprovalDecision {
    pub status: String,
    pub risk_level: String,
    pub policy: String,
    pub message: String,
    pub requires_approval: bool,
}

/// Register decision - legacy API
pub fn register_decision(_decision: &str, action: &str, _plan: &impl std::fmt::Debug) {
    println!(
        "📝 [ApprovalGate] Decision registered for action: {}",
        action
    );
}

/// Preview approval - legacy API  
pub fn preview_approval(action: &str, _plan: &impl std::fmt::Debug) -> ApprovalDecision {
    // Try to parse action as JSON
    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(action) {
        let action_type = parsed
            .get("action")
            .and_then(|a| a.as_str())
            .unwrap_or("unknown");

        if action_type == "shell" || action_type == "run_shell" {
            if let Some(cmd) = parsed.get("command").and_then(|c| c.as_str()) {
                let level = ApprovalGate::check_command(cmd);
                let (status, risk, requires_approval) = match level {
                    ApprovalLevel::Blocked => ("denied".to_string(), "critical".to_string(), true),
                    ApprovalLevel::RequireApproval => {
                        ("pending".to_string(), "high".to_string(), true)
                    }
                    ApprovalLevel::AutoApprove => {
                        ("approved".to_string(), "low".to_string(), false)
                    }
                };
                return ApprovalDecision {
                    status,
                    risk_level: risk,
                    policy: "default".to_string(),
                    message: format!("Shell command: {}", cmd),
                    requires_approval,
                };
            }
        }

        return ApprovalDecision {
            status: "approved".to_string(),
            risk_level: "low".to_string(),
            policy: "default".to_string(),
            message: format!("Action: {}", action_type),
            requires_approval: false,
        };
    }

    let trimmed = action.trim();
    let looks_like_shell = trimmed.contains(' ')
        || trimmed.contains('/')
        || trimmed.contains('|')
        || trimmed.contains(';')
        || trimmed.starts_with("sudo");
    if !looks_like_shell {
        return ApprovalDecision {
            status: "approved".to_string(),
            risk_level: "low".to_string(),
            policy: "default".to_string(),
            message: format!("Action: {}", action),
            requires_approval: false,
        };
    }

    // Shell-like plain text action: gate by command policy.
    let level = ApprovalGate::check_command(trimmed);
    let (status, risk, requires_approval) = match level {
        ApprovalLevel::Blocked => ("denied".to_string(), "critical".to_string(), true),
        ApprovalLevel::RequireApproval => ("pending".to_string(), "high".to_string(), true),
        ApprovalLevel::AutoApprove => ("approved".to_string(), "low".to_string(), false),
    };
    ApprovalDecision {
        status,
        risk_level: risk,
        policy: "default".to_string(),
        message: format!("Action: {}", action),
        requires_approval,
    }
}

/// Evaluate approval - legacy API (used by execution_controller)
pub fn evaluate_approval(action: &str, plan: &impl std::fmt::Debug) -> ApprovalDecision {
    preview_approval(action, plan)
}
