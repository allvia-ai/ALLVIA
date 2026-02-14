// Approval Gate - Ported from clawdbot-main/src/agents/bash-tools.exec.ts
// Provides command approval workflow for dangerous operations

use lazy_static::lazy_static;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::time::{Duration, Instant};

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

    /// Safe binaries (conditionally auto-approved when arguments are non-risky)
    static ref SAFE_BINS: HashSet<&'static str> = {
        let mut set = HashSet::new();
        set.insert("ls");
        set.insert("pwd");
        set.insert("echo");
        set.insert("cat");
        set.insert("head");
        set.insert("tail");
        set.insert("grep");
        set.insert("cut");
        set.insert("tr");
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
    /// JSON actions that are read-only/low-risk and can auto-pass by default.
    static ref SAFE_NON_SHELL_ACTIONS: HashSet<&'static str> = {
        let mut set = HashSet::new();
        set.insert("snapshot");
        set.insert("read");
        set.insert("read_clipboard");
        set.insert("extract");
        set.insert("wait");
        set.insert("done");
        set
    };

    /// Pending approvals registry
    static ref PENDING_APPROVALS: Mutex<Vec<ApprovalRequest>> = Mutex::new(Vec::new());
    /// User decisions keyed by plan_id + action fingerprint.
    static ref DECISION_REGISTRY: Mutex<HashMap<String, DecisionEntry>> = Mutex::new(HashMap::new());
}

#[derive(Debug, Clone)]
struct DecisionEntry {
    status: ApprovalStatus,
    expires_at: Instant,
}

// =====================================================
// Approval Gate Implementation
// =====================================================

pub struct ApprovalGate;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShellOperator {
    Pipe,
    And,
    Or,
    Seq,
}

#[derive(Debug, Default)]
struct ParsedShellCommand {
    segments: Vec<String>,
    operators: Vec<ShellOperator>,
    has_redirection: bool,
    has_substitution: bool,
    has_unterminated_quote: bool,
}

impl ApprovalGate {
    fn push_segment(buffer: &mut String, segments: &mut Vec<String>) {
        let trimmed = buffer.trim();
        if !trimmed.is_empty() {
            segments.push(trimmed.to_string());
        }
        buffer.clear();
    }

    fn parse_shell_command(cmd: &str) -> ParsedShellCommand {
        let mut parsed = ParsedShellCommand::default();
        let mut current = String::new();
        let mut chars = cmd.chars().peekable();
        let mut in_single = false;
        let mut in_double = false;
        let mut escaped = false;

        while let Some(ch) = chars.next() {
            if escaped {
                current.push(ch);
                escaped = false;
                continue;
            }

            if ch == '\\' && !in_single {
                escaped = true;
                current.push(ch);
                continue;
            }

            if ch == '\'' && !in_double {
                in_single = !in_single;
                current.push(ch);
                continue;
            }
            if ch == '"' && !in_single {
                in_double = !in_double;
                current.push(ch);
                continue;
            }

            if !in_single && !in_double {
                if ch == '`' {
                    parsed.has_substitution = true;
                    current.push(ch);
                    continue;
                }
                if ch == '$' && matches!(chars.peek(), Some('(')) {
                    parsed.has_substitution = true;
                    current.push(ch);
                    continue;
                }
                if ch == '>' || ch == '<' {
                    parsed.has_redirection = true;
                    current.push(ch);
                    continue;
                }
                if ch == '&' && matches!(chars.peek(), Some('&')) {
                    let _ = chars.next();
                    Self::push_segment(&mut current, &mut parsed.segments);
                    parsed.operators.push(ShellOperator::And);
                    continue;
                }
                if ch == '|' {
                    if matches!(chars.peek(), Some('|')) {
                        let _ = chars.next();
                        Self::push_segment(&mut current, &mut parsed.segments);
                        parsed.operators.push(ShellOperator::Or);
                        continue;
                    }
                    Self::push_segment(&mut current, &mut parsed.segments);
                    parsed.operators.push(ShellOperator::Pipe);
                    continue;
                }
                if ch == ';' {
                    Self::push_segment(&mut current, &mut parsed.segments);
                    parsed.operators.push(ShellOperator::Seq);
                    continue;
                }
            }

            current.push(ch);
        }

        if escaped || in_single || in_double {
            parsed.has_unterminated_quote = true;
        }
        Self::push_segment(&mut current, &mut parsed.segments);
        parsed
    }

    fn command_words(cmd: &str) -> Vec<String> {
        let mut words = Vec::new();
        let mut current = String::new();
        let mut chars = cmd.chars().peekable();
        let mut in_single = false;
        let mut in_double = false;
        let mut escaped = false;

        while let Some(ch) = chars.next() {
            if escaped {
                current.push(ch);
                escaped = false;
                continue;
            }

            if ch == '\\' && !in_single {
                escaped = true;
                continue;
            }

            if ch == '\'' && !in_double {
                in_single = !in_single;
                continue;
            }
            if ch == '"' && !in_single {
                in_double = !in_double;
                continue;
            }

            if ch.is_whitespace() && !in_single && !in_double {
                if !current.is_empty() {
                    words.push(current.to_lowercase());
                    current.clear();
                }
                continue;
            }
            current.push(ch);
        }

        if !current.is_empty() {
            words.push(current.to_lowercase());
        }
        words
    }

    fn command_binary(words: &[String]) -> String {
        let Some(first) = words.first() else {
            return String::new();
        };
        std::path::Path::new(first)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(first)
            .to_string()
    }

    fn requires_approval_by_tokens(binary: &str, words: &[String]) -> bool {
        if binary.is_empty() {
            return true;
        }
        if matches!(
            binary,
            "sudo"
                | "rm"
                | "chmod"
                | "chown"
                | "kill"
                | "killall"
                | "shutdown"
                | "reboot"
                | "passwd"
                | "curl"
                | "wget"
        ) {
            return true;
        }
        if matches!(
            binary,
            "apt" | "apt-get" | "brew" | "pip" | "pip3" | "npm" | "cargo"
        ) && words.iter().skip(1).any(|w| {
            matches!(
                w.as_str(),
                "install" | "i" | "upgrade" | "remove" | "uninstall" | "global" | "-g"
            )
        }) {
            return true;
        }
        false
    }

    fn is_blocked_segment(segment: &str, binary: &str, words: &[String]) -> bool {
        let seg_lc = segment.trim().to_lowercase();
        if seg_lc.is_empty() {
            return false;
        }

        for pattern in BLOCKED_PATTERNS.iter() {
            if seg_lc.starts_with(pattern) {
                return true;
            }
        }

        if binary == "mkfs" {
            return true;
        }
        if binary == "rm" {
            let has_rf = words
                .iter()
                .any(|w| w.starts_with('-') && w.contains('r') && w.contains('f'));
            let has_root_target = words
                .iter()
                .any(|w| matches!(w.as_str(), "/" | "/*" | "~" | "~/"));
            if has_rf && has_root_target {
                return true;
            }
        }
        if seg_lc.contains("/dev/sda") && seg_lc.contains('>') {
            return true;
        }

        false
    }

    fn token_looks_path_like(token: &str) -> bool {
        token.contains('/')
            || token.contains('\\')
            || token.starts_with('.')
            || token.starts_with('~')
            || token.starts_with('*')
            || token.ends_with('*')
    }

    fn safe_bin_args_ok(binary: &str, words: &[String]) -> bool {
        if words.len() <= 1 {
            return true;
        }
        for arg in words.iter().skip(1) {
            if arg.is_empty() {
                continue;
            }
            if arg.starts_with('-') {
                continue;
            }
            if arg.chars().all(|c| c.is_ascii_digit()) {
                continue;
            }
            // echo is text-oriented; keep existing low-friction behavior for literals.
            if binary == "echo" {
                continue;
            }
            // Safe bins should not auto-approve positional/path-like args.
            if Self::token_looks_path_like(arg) {
                return false;
            }
            return false;
        }
        true
    }

    /// Check if a command requires approval, is blocked, or can auto-run
    pub fn check_command(cmd: &str) -> ApprovalLevel {
        let cmd_trimmed = cmd.trim();
        if cmd_trimmed.is_empty() {
            return ApprovalLevel::RequireApproval;
        }

        let parsed = Self::parse_shell_command(cmd_trimmed);
        if parsed.has_unterminated_quote {
            return ApprovalLevel::RequireApproval;
        }
        if parsed.has_substitution {
            return ApprovalLevel::RequireApproval;
        }

        if parsed.has_redirection {
            if parsed.segments.iter().any(|segment| {
                let lowered = segment.to_lowercase();
                lowered.contains("/dev/sda") && lowered.contains('>')
            }) {
                return ApprovalLevel::Blocked;
            }
            return ApprovalLevel::RequireApproval;
        }

        let mut has_required_segment = false;
        for segment in &parsed.segments {
            let words = Self::command_words(segment);
            let binary_name = Self::command_binary(&words);

            if Self::is_blocked_segment(segment, &binary_name, &words) {
                return ApprovalLevel::Blocked;
            }
            if Self::requires_approval_by_tokens(&binary_name, &words) {
                has_required_segment = true;
                continue;
            }
            if SAFE_BINS.contains(binary_name.as_str())
                && Self::safe_bin_args_ok(&binary_name, &words)
            {
                continue;
            }
            has_required_segment = true;
        }

        if has_required_segment {
            return ApprovalLevel::RequireApproval;
        }

        if parsed.operators.iter().any(|op| {
            matches!(
                op,
                ShellOperator::And | ShellOperator::Or | ShellOperator::Seq
            )
        }) {
            return ApprovalLevel::RequireApproval;
        }

        ApprovalLevel::AutoApprove
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
    use crate::nl_automation::{IntentType, Plan};
    use serial_test::serial;

    #[test]
    #[serial]
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
    #[serial]
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
            ApprovalLevel::RequireApproval
        );
        assert_eq!(
            ApprovalGate::check_command("echo hello world"),
            ApprovalLevel::AutoApprove
        );
        assert_eq!(
            ApprovalGate::check_command("echo 'rm -rf /'"),
            ApprovalLevel::AutoApprove
        );
    }

    #[test]
    #[serial]
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

    #[test]
    #[serial]
    fn test_token_aware_classification_avoids_false_positive_contains() {
        assert_eq!(
            ApprovalGate::check_command("echo sudo is blocked"),
            ApprovalLevel::AutoApprove
        );
    }

    #[test]
    #[serial]
    fn test_package_install_requires_approval() {
        assert_eq!(
            ApprovalGate::check_command("npm install -g pnpm"),
            ApprovalLevel::RequireApproval
        );
        assert_eq!(
            ApprovalGate::check_command("cargo install ripgrep"),
            ApprovalLevel::RequireApproval
        );
    }

    #[test]
    #[serial]
    fn test_unknown_json_action_requires_approval() {
        reset_decisions();
        let plan = test_plan(&format!("plan-unknown-action-{}", uuid::Uuid::new_v4()));
        let action = r#"{"action":"open_app","name":"Safari"}"#;
        let decision = preview_approval(action, &plan);
        assert_eq!(decision.status, "pending");
        assert!(decision.requires_approval);
    }

    #[test]
    #[serial]
    fn test_json_action_key_is_canonicalized_for_decision_reuse() {
        reset_decisions();
        let plan = test_plan("plan-json-canonical-key");
        let action_a = r#"{"action":"open_app","name":"Safari","meta":{"b":2,"a":1},"args":[2,1]}"#;
        let action_b =
            r#"{ "name":"Safari","meta":{"a":1,"b":2},"args":[2,1],"action":"open_app" }"#;
        register_decision("approve", action_a, &plan);
        let decision = preview_approval(action_b, &plan);
        assert_eq!(decision.status, "approved");
        assert!(!decision.requires_approval);
        assert_eq!(decision.policy, "user_decision");
    }

    #[test]
    #[serial]
    fn test_safe_bin_with_shell_features_requires_approval() {
        assert_eq!(
            ApprovalGate::check_command("echo hello > /tmp/a.txt"),
            ApprovalLevel::RequireApproval
        );
        assert_eq!(
            ApprovalGate::check_command("cat file.txt | wc -l"),
            ApprovalLevel::RequireApproval
        );
        assert_eq!(
            ApprovalGate::check_command("echo hello | wc -c"),
            ApprovalLevel::AutoApprove
        );
        assert_eq!(
            ApprovalGate::check_command("echo hello && pwd"),
            ApprovalLevel::RequireApproval
        );
    }

    #[test]
    #[serial]
    fn test_user_approval_overrides_policy() {
        reset_decisions();
        let plan = test_plan(&format!("plan-approval-override-{}", uuid::Uuid::new_v4()));
        let action = r#"{"action":"shell","command":"sudo apt install git"}"#;

        let before = preview_approval(action, &plan);
        assert_eq!(before.status, "pending");
        assert!(before.requires_approval);

        register_decision("approve", action, &plan);
        let after = preview_approval(action, &plan);
        assert_eq!(after.status, "approved");
        assert!(!after.requires_approval);
        assert_eq!(after.policy, "user_decision");
    }

    #[test]
    #[serial]
    fn test_user_denial_overrides_policy() {
        reset_decisions();
        let plan = test_plan(&format!("plan-deny-override-{}", uuid::Uuid::new_v4()));
        let action = r#"{"action":"shell","command":"sudo apt install git"}"#;

        register_decision("deny", action, &plan);
        let decision = evaluate_approval(action, &plan);
        assert_eq!(decision.status, "denied");
        assert!(decision.requires_approval);
        assert_eq!(decision.policy, "user_decision");
    }

    #[test]
    #[serial]
    fn test_evaluate_approval_ask_fallback_deny() {
        reset_decisions();
        std::env::set_var("STEER_APPROVAL_ASK_FALLBACK", "deny");
        let plan = test_plan(&format!("plan-fallback-deny-{}", uuid::Uuid::new_v4()));
        let action = r#"{"action":"shell","command":"sudo apt install git"}"#;
        let decision = evaluate_approval(action, &plan);
        assert_eq!(decision.status, "denied");
        assert!(decision.requires_approval);
        assert_eq!(decision.policy, "ask_fallback_deny");
        std::env::remove_var("STEER_APPROVAL_ASK_FALLBACK");
    }

    #[test]
    #[serial]
    fn test_evaluate_approval_ask_fallback_allow_once() {
        reset_decisions();
        std::env::set_var("STEER_APPROVAL_ASK_FALLBACK", "allow-once");
        let plan = test_plan(&format!("plan-fallback-allow-{}", uuid::Uuid::new_v4()));
        let action = r#"{"action":"shell","command":"sudo apt install git"}"#;
        let decision = evaluate_approval(action, &plan);
        assert_eq!(decision.status, "approved");
        assert!(!decision.requires_approval);
        assert_eq!(decision.policy, "ask_fallback_allow_once");
        std::env::remove_var("STEER_APPROVAL_ASK_FALLBACK");
    }

    #[test]
    #[serial]
    fn test_decision_persists_via_db() {
        if let Err(e) = crate::db::init() {
            eprintln!("skip: db init unavailable for persistence test: {}", e);
            return;
        }
        reset_decisions();
        let plan = test_plan(&format!("plan-db-persist-{}", uuid::Uuid::new_v4()));
        let action = r#"{"action":"shell","command":"sudo apt install git"}"#;

        register_decision("approve", action, &plan);
        reset_decisions();

        let after = preview_approval(action, &plan);
        assert_eq!(after.status, "approved");
        assert!(!after.requires_approval);
        assert_eq!(after.policy, "user_decision");
    }

    fn test_plan(plan_id: &str) -> Plan {
        Plan {
            plan_id: plan_id.to_string(),
            intent: IntentType::GenericTask,
            slots: std::collections::HashMap::new(),
            steps: Vec::new(),
        }
    }

    fn reset_decisions() {
        if let Ok(mut registry) = DECISION_REGISTRY.lock() {
            registry.clear();
        }
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
pub fn register_decision(decision: &str, action: &str, plan: &crate::nl_automation::Plan) {
    let Some(status) = parse_user_decision(decision) else {
        println!(
            "⚠️ [ApprovalGate] Ignored unsupported decision '{}' for action '{}'",
            decision, action
        );
        return;
    };
    let ttl = decision_ttl_for(decision);
    let key = decision_key(&plan.plan_id, action);
    let entry = DecisionEntry {
        status,
        expires_at: Instant::now() + ttl,
    };
    if let Ok(mut registry) = DECISION_REGISTRY.lock() {
        cleanup_expired_decisions_locked(&mut registry);
        registry.insert(key.clone(), entry);
    }
    if let Err(e) = crate::db::upsert_approval_decision(
        &key,
        &plan.plan_id,
        action,
        status_to_storage(status),
        std::cmp::min(ttl.as_secs(), i64::MAX as u64) as i64,
    ) {
        println!(
            "⚠️ [ApprovalGate] Failed to persist decision for plan {}: {}",
            plan.plan_id, e
        );
    }
    println!(
        "📝 [ApprovalGate] Decision '{}' registered for plan {} action: {}",
        decision, plan.plan_id, action
    );
}

/// Preview approval - legacy API  
pub fn preview_approval(action: &str, plan: &crate::nl_automation::Plan) -> ApprovalDecision {
    if let Some(override_status) = get_registered_decision(&plan.plan_id, action) {
        let (status, risk, requires_approval, message) = match override_status {
            ApprovalStatus::Approved => (
                "approved".to_string(),
                "low".to_string(),
                false,
                "User approved this action".to_string(),
            ),
            ApprovalStatus::Denied => (
                "denied".to_string(),
                "high".to_string(),
                true,
                "User denied this action".to_string(),
            ),
            ApprovalStatus::Pending => (
                "pending".to_string(),
                "high".to_string(),
                true,
                "Approval decision is pending".to_string(),
            ),
            ApprovalStatus::Expired => (
                "pending".to_string(),
                "high".to_string(),
                true,
                "Approval decision expired".to_string(),
            ),
        };
        return ApprovalDecision {
            status,
            risk_level: risk,
            policy: "user_decision".to_string(),
            message: format!("{}: {}", message, action),
            requires_approval,
        };
    }

    // Try to parse action as JSON
    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(action) {
        let action_type = parsed
            .get("action")
            .and_then(|a| a.as_str())
            .unwrap_or("unknown");
        let action_type_lc = action_type.trim().to_lowercase();

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

        if SAFE_NON_SHELL_ACTIONS.contains(action_type_lc.as_str()) {
            return ApprovalDecision {
                status: "approved".to_string(),
                risk_level: "low".to_string(),
                policy: "default".to_string(),
                message: format!("Action: {}", action_type),
                requires_approval: false,
            };
        }

        return ApprovalDecision {
            status: "pending".to_string(),
            risk_level: "high".to_string(),
            policy: "default".to_string(),
            message: format!("Action requires explicit approval: {}", action_type),
            requires_approval: true,
        };
    }

    let trimmed = action.trim();
    let action_lc = trimmed.to_lowercase();
    if matches!(action_lc.as_str(), "done" | "continue" | "next" | "skip") {
        return ApprovalDecision {
            status: "approved".to_string(),
            risk_level: "low".to_string(),
            policy: "default".to_string(),
            message: format!("Action: {}", action),
            requires_approval: false,
        };
    }

    // Plain text action: gate by the same segmented command policy.
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
pub fn evaluate_approval(action: &str, plan: &crate::nl_automation::Plan) -> ApprovalDecision {
    let preview = preview_approval(action, plan);
    apply_pending_ask_fallback(preview)
}

fn normalize_action(action: &str) -> String {
    action
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string()
}

fn canonicalize_json_value(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut keys = map.keys().cloned().collect::<Vec<_>>();
            keys.sort();
            let mut sorted = Map::new();
            for key in keys {
                if let Some(item) = map.get(&key) {
                    sorted.insert(key, canonicalize_json_value(item));
                }
            }
            Value::Object(sorted)
        }
        Value::Array(items) => Value::Array(items.iter().map(canonicalize_json_value).collect()),
        _ => value.clone(),
    }
}

fn action_fingerprint(action: &str) -> String {
    let normalized = normalize_action(action);
    let canonical = if let Ok(parsed) = serde_json::from_str::<Value>(&normalized) {
        let stable = canonicalize_json_value(&parsed);
        serde_json::to_string(&stable).unwrap_or(normalized)
    } else {
        normalized.to_lowercase()
    };
    let mut hasher = Sha256::new();
    hasher.update(canonical.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn decision_key(plan_id: &str, action: &str) -> String {
    format!("{}::{}", plan_id, action_fingerprint(action))
}

fn legacy_decision_key(plan_id: &str, action: &str) -> String {
    format!("{}::{}", plan_id, normalize_action(action))
}

fn parse_user_decision(decision: &str) -> Option<ApprovalStatus> {
    let normalized = decision.trim().to_lowercase();
    if matches!(
        normalized.as_str(),
        "approve"
            | "approved"
            | "allow"
            | "allow-once"
            | "allow_once"
            | "allow-always"
            | "allow_always"
            | "yes"
            | "y"
    ) {
        return Some(ApprovalStatus::Approved);
    }
    if matches!(
        normalized.as_str(),
        "deny" | "denied" | "reject" | "rejected" | "no" | "n"
    ) {
        return Some(ApprovalStatus::Denied);
    }
    None
}

fn approval_ask_fallback_mode() -> String {
    std::env::var("STEER_APPROVAL_ASK_FALLBACK")
        .ok()
        .map(|v| v.trim().to_lowercase())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "ask".to_string())
}

fn apply_pending_ask_fallback(mut decision: ApprovalDecision) -> ApprovalDecision {
    if !(decision.requires_approval && decision.status == "pending") {
        return decision;
    }
    match approval_ask_fallback_mode().as_str() {
        "ask" | "pending" => decision,
        "allow" | "allow-once" | "allow_once" | "allow-once-only" => {
            decision.status = "approved".to_string();
            decision.requires_approval = false;
            decision.policy = "ask_fallback_allow_once".to_string();
            decision.message = format!("{} [ask_fallback=allow-once]", decision.message);
            decision
        }
        _ => {
            decision.status = "denied".to_string();
            decision.requires_approval = true;
            decision.policy = "ask_fallback_deny".to_string();
            decision.message = format!("{} [ask_fallback=deny]", decision.message);
            decision
        }
    }
}

fn decision_ttl() -> std::time::Duration {
    let ttl_seconds = std::env::var("STEER_APPROVAL_DECISION_TTL_SECONDS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(600);
    Duration::from_secs(ttl_seconds)
}

fn decision_ttl_for(decision: &str) -> std::time::Duration {
    let normalized = decision.trim().to_lowercase();
    if matches!(normalized.as_str(), "allow-always" | "allow_always") {
        let ttl_seconds = std::env::var("STEER_APPROVAL_ALLOW_ALWAYS_TTL_SECONDS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(60 * 60 * 24 * 30);
        return Duration::from_secs(ttl_seconds);
    }
    decision_ttl()
}

fn cleanup_expired_decisions_locked(registry: &mut HashMap<String, DecisionEntry>) {
    let now = Instant::now();
    registry.retain(|_, entry| entry.expires_at > now);
}

fn get_registered_decision(plan_id: &str, action: &str) -> Option<ApprovalStatus> {
    let key = decision_key(plan_id, action);
    let legacy_key = legacy_decision_key(plan_id, action);
    let mut keys = vec![key.clone()];
    if legacy_key != key {
        keys.push(legacy_key.clone());
    }

    if let Ok(mut registry) = DECISION_REGISTRY.lock() {
        cleanup_expired_decisions_locked(&mut registry);
        for candidate in &keys {
            if let Some(entry) = registry.get(candidate).cloned() {
                if candidate != &key {
                    registry.insert(key.clone(), entry.clone());
                }
                return Some(entry.status);
            }
        }
    }

    for candidate in &keys {
        let Some(stored) = crate::db::get_active_approval_decision(candidate)
            .ok()
            .flatten()
        else {
            continue;
        };
        let Some(status) = parse_stored_status(&stored.status) else {
            continue;
        };
        let expires_at = instant_from_expiry(&stored.expires_at)
            .unwrap_or_else(|| Instant::now() + decision_ttl());
        if let Ok(mut registry) = DECISION_REGISTRY.lock() {
            registry.insert(key.clone(), DecisionEntry { status, expires_at });
            if candidate != &key {
                registry.insert(candidate.clone(), DecisionEntry { status, expires_at });
            }
        }
        if candidate != &key {
            let ttl_seconds = chrono::DateTime::parse_from_rfc3339(&stored.expires_at)
                .ok()
                .map(|expiry| {
                    let now = chrono::Utc::now();
                    let expiry_utc = expiry.with_timezone(&chrono::Utc);
                    (expiry_utc - now).num_seconds().max(1)
                })
                .unwrap_or_else(|| decision_ttl().as_secs().min(i64::MAX as u64) as i64);
            let _ = crate::db::upsert_approval_decision(
                &key,
                plan_id,
                action,
                status_to_storage(status),
                ttl_seconds,
            );
        }
        return Some(status);
    }

    None
}

fn status_to_storage(status: ApprovalStatus) -> &'static str {
    match status {
        ApprovalStatus::Pending => "pending",
        ApprovalStatus::Approved => "approved",
        ApprovalStatus::Denied => "denied",
        ApprovalStatus::Expired => "expired",
    }
}

fn parse_stored_status(value: &str) -> Option<ApprovalStatus> {
    match value.trim().to_lowercase().as_str() {
        "pending" => Some(ApprovalStatus::Pending),
        "approved" => Some(ApprovalStatus::Approved),
        "denied" => Some(ApprovalStatus::Denied),
        "expired" => Some(ApprovalStatus::Expired),
        _ => None,
    }
}

fn instant_from_expiry(expires_at: &str) -> Option<Instant> {
    let parsed = chrono::DateTime::parse_from_rfc3339(expires_at).ok()?;
    let expiry_utc = parsed.with_timezone(&chrono::Utc);
    let now_utc = chrono::Utc::now();
    let remaining = (expiry_utc - now_utc).to_std().ok()?;
    Some(Instant::now() + remaining)
}
