use std::env;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SendDecision {
    Allow,
    Deny,
}

#[derive(Debug, Clone)]
pub struct SendPolicyContext {
    pub session_key: Option<String>,
    pub channel: Option<String>,
    pub chat_type: Option<String>,
    pub target_id: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct SendPolicyRule {
    action: Option<String>,
    #[serde(default)]
    r#match: SendPolicyMatch,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
struct SendPolicyMatch {
    channel: Option<String>,
    chat_type: Option<String>,
    key_prefix: Option<String>,
    target_id: Option<String>,
}

pub fn should_send(title: &str, message: &str) -> SendDecision {
    should_send_with_context(title, message, None)
}

pub fn should_send_with_context(
    title: &str,
    message: &str,
    ctx: Option<&SendPolicyContext>,
) -> SendDecision {
    let policy = env::var("NOTIFY_POLICY").unwrap_or_else(|_| "allow".to_string());
    let policy = policy.trim().to_lowercase();
    if policy == "deny" {
        return SendDecision::Deny;
    }

    let channel = ctx.and_then(|c| normalize(c.channel.as_deref()));
    let target_id = ctx.and_then(|c| normalize(c.target_id.as_deref()));
    let deny_channels = env::var("NOTIFY_DENY_CHANNELS").unwrap_or_default();
    if contains_csv_exact(channel.as_deref(), &deny_channels) {
        return SendDecision::Deny;
    }
    let allow_channels = env::var("NOTIFY_ALLOW_CHANNELS").unwrap_or_default();
    if !allow_channels.trim().is_empty() && !contains_csv_exact(channel.as_deref(), &allow_channels)
    {
        return SendDecision::Deny;
    }

    let deny_targets = env::var("NOTIFY_DENY_TARGET_IDS").unwrap_or_default();
    if contains_csv_exact(target_id.as_deref(), &deny_targets) {
        return SendDecision::Deny;
    }
    let allow_targets = env::var("NOTIFY_ALLOW_TARGET_IDS").unwrap_or_default();
    if !allow_targets.trim().is_empty() && !contains_csv_exact(target_id.as_deref(), &allow_targets)
    {
        return SendDecision::Deny;
    }

    let deny_keywords = env::var("NOTIFY_DENY_KEYWORDS").unwrap_or_default();
    if contains_any_keyword(title, message, &deny_keywords) {
        return SendDecision::Deny;
    }

    let allow_keywords = env::var("NOTIFY_ALLOW_KEYWORDS").unwrap_or_default();
    if !allow_keywords.trim().is_empty() {
        return if contains_any_keyword(title, message, &allow_keywords) {
            SendDecision::Allow
        } else {
            SendDecision::Deny
        };
    }

    if let Some(ctx) = ctx {
        if let Some(decision) = apply_rule_policy(ctx) {
            return decision;
        }
    }

    SendDecision::Allow
}

fn contains_any_keyword(title: &str, message: &str, raw: &str) -> bool {
    let haystack = format!("{} {}", title.to_lowercase(), message.to_lowercase());
    raw.split(',')
        .map(|k| k.trim().to_lowercase())
        .filter(|k| !k.is_empty())
        .any(|k| haystack.contains(&k))
}

fn apply_rule_policy(ctx: &SendPolicyContext) -> Option<SendDecision> {
    let raw_rules = env::var("NOTIFY_POLICY_RULES").ok()?;
    let rules: Vec<SendPolicyRule> = serde_json::from_str(&raw_rules).ok()?;
    let channel = normalize(ctx.channel.as_deref());
    let chat_type = normalize(ctx.chat_type.as_deref());
    let session_key = normalize(ctx.session_key.as_deref());
    let target_id = normalize(ctx.target_id.as_deref());

    let mut allow_match = false;
    for rule in rules {
        let action = normalize(rule.action.as_deref()).unwrap_or_else(|| "allow".to_string());
        let match_channel = normalize(rule.r#match.channel.as_deref());
        let match_chat = normalize(rule.r#match.chat_type.as_deref());
        let match_prefix = normalize(rule.r#match.key_prefix.as_deref());
        let match_target_id = normalize(rule.r#match.target_id.as_deref());

        if let Some(ref m) = match_channel {
            if channel.as_deref() != Some(m.as_str()) {
                continue;
            }
        }
        if let Some(ref m) = match_chat {
            if chat_type.as_deref() != Some(m.as_str()) {
                continue;
            }
        }
        if let Some(ref m) = match_prefix {
            if let Some(ref key) = session_key {
                if !key.starts_with(m) {
                    continue;
                }
            } else {
                continue;
            }
        }
        if let Some(ref m) = match_target_id {
            if target_id.as_deref() != Some(m.as_str()) {
                continue;
            }
        }

        if action == "deny" {
            return Some(SendDecision::Deny);
        }
        allow_match = true;
    }

    if allow_match {
        return Some(SendDecision::Allow);
    }
    None
}

fn normalize(value: Option<&str>) -> Option<String> {
    let v = value.unwrap_or("").trim().to_lowercase();
    if v.is_empty() {
        None
    } else {
        Some(v)
    }
}

fn contains_csv_exact(value: Option<&str>, raw_list: &str) -> bool {
    let Some(value) = value.map(str::trim).filter(|v| !v.is_empty()) else {
        return false;
    };
    raw_list
        .split(',')
        .map(|v| v.trim().to_lowercase())
        .filter(|v| !v.is_empty())
        .any(|v| v == value.to_lowercase())
}
