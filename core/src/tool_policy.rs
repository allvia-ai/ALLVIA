use crate::schema::AgentAction;
use std::env;

#[derive(Debug, Clone)]
struct ToolPolicy {
    allow: Vec<String>,
    deny: Vec<String>,
}

impl ToolPolicy {
    fn from_env() -> Self {
        let allow = parse_list(&env::var("TOOL_ALLOWLIST").unwrap_or_default());
        let deny = parse_list(&env::var("TOOL_DENYLIST").unwrap_or_default());
        Self { allow, deny }
    }

    fn is_allowed(&self, tool: &str) -> bool {
        let tool = tool.trim().to_lowercase();
        if tool.is_empty() {
            return false;
        }

        if self
            .deny
            .iter()
            .any(|pattern| matches_pattern(pattern, &tool))
        {
            return false;
        }

        if self.allow.is_empty() {
            return true;
        }

        self.allow
            .iter()
            .any(|pattern| matches_pattern(pattern, &tool))
    }
}

pub fn is_action_allowed(action: &AgentAction) -> bool {
    let policy = ToolPolicy::from_env();
    let tool_name = action_kind(action);
    policy.is_allowed(tool_name)
}

fn action_kind(action: &AgentAction) -> &'static str {
    match action {
        AgentAction::UiSnapshot { .. } => "ui.snapshot",
        AgentAction::UiFind { .. } => "ui.find",
        AgentAction::UiClick { .. } => "ui.click",
        AgentAction::UiClickText { .. } => "ui.click_text",
        AgentAction::UiType { .. } => "ui.type",
        AgentAction::KeyboardType { .. } => "keyboard.type",
        AgentAction::SystemOpen { .. } => "system.open",
        AgentAction::SystemSearch { .. } => "system.search",
        AgentAction::Terminate => "system.terminate",
        AgentAction::DebugFakeLog => "debug.fake_log",
        AgentAction::ShellExecution { .. } => "shell.exec",
    }
}

fn parse_list(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(normalize_pattern)
        .filter(|s| !s.is_empty())
        .collect()
}

fn normalize_pattern(raw: &str) -> String {
    let trimmed = raw.trim().to_lowercase();
    if trimmed.is_empty() {
        return trimmed;
    }
    if trimmed == "*" {
        return trimmed;
    }
    if trimmed.contains('*') || trimmed.contains('.') {
        return trimmed;
    }
    // Treat bare group names like "ui" or "system" as prefix wildcard.
    format!("{}.*", trimmed)
}

fn matches_pattern(pattern: &str, value: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if !pattern.contains('*') {
        return pattern == value;
    }

    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.is_empty() {
        return false;
    }

    let mut remainder = value;
    let mut is_first = true;
    for (idx, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }
        if is_first && !pattern.starts_with('*') {
            if !remainder.starts_with(part) {
                return false;
            }
            remainder = &remainder[part.len()..];
            is_first = false;
            continue;
        }
        if let Some(pos) = remainder.find(part) {
            remainder = &remainder[pos + part.len()..];
        } else {
            return false;
        }
        is_first = false;
        if idx == parts.len() - 1 && !pattern.ends_with('*') {
            return remainder.is_empty();
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::AgentAction;

    #[test]
    fn matches_wildcard_prefix() {
        assert!(matches_pattern("ui.*", "ui.click"));
        assert!(!matches_pattern("ui.*", "shell.exec"));
    }

    #[test]
    fn matches_star() {
        assert!(matches_pattern("*", "any.tool"));
    }

    #[test]
    fn deny_overrides_allow() {
        let policy = ToolPolicy {
            allow: vec!["ui.*".to_string()],
            deny: vec!["ui.click".to_string()],
        };
        assert!(!policy.is_allowed("ui.click"));
        assert!(policy.is_allowed("ui.type"));
    }

    #[test]
    fn action_kind_mapping() {
        let action = AgentAction::ShellExecution {
            command: "ls".to_string(),
        };
        assert_eq!(action_kind(&action), "shell.exec");
    }
}
