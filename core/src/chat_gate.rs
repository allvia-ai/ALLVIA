use std::env;

#[derive(Debug, Clone)]
pub struct ChatGateConfig {
    pub enabled: bool,
    pub require_mention: bool,
    pub allowed_channels: Vec<String>,
    pub allowed_chat_types: Vec<String>,
    pub allowed_senders: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ChatGateContext {
    pub channel: Option<String>,
    pub chat_type: Option<String>,
    pub sender: Option<String>,
    pub mentioned: Option<bool>,
}

impl ChatGateConfig {
    pub fn from_env() -> Self {
        Self {
            enabled: env_flag("CHAT_GATE_ENABLED", false),
            require_mention: env_flag("CHAT_REQUIRE_MENTION", false),
            allowed_channels: parse_list(&env::var("CHAT_ALLOWED_CHANNELS").unwrap_or_default()),
            allowed_chat_types: parse_list(
                &env::var("CHAT_ALLOWED_CHAT_TYPES").unwrap_or_default(),
            ),
            allowed_senders: parse_list(&env::var("CHAT_ALLOWED_SENDERS").unwrap_or_default()),
        }
    }

    pub fn is_allowed(&self, ctx: &ChatGateContext) -> bool {
        if !self.enabled {
            return true;
        }
        if self.require_mention && ctx.mentioned != Some(true) {
            return false;
        }
        if !self.allowed_channels.is_empty() {
            if let Some(channel) = ctx.channel.as_ref().map(|s| s.to_lowercase()) {
                if !self.allowed_channels.iter().any(|c| c == &channel) {
                    return false;
                }
            } else {
                return false;
            }
        }
        if !self.allowed_chat_types.is_empty() {
            if let Some(chat_type) = ctx.chat_type.as_ref().map(|s| s.to_lowercase()) {
                if !self.allowed_chat_types.iter().any(|c| c == &chat_type) {
                    return false;
                }
            } else {
                return false;
            }
        }
        if !self.allowed_senders.is_empty() {
            if let Some(sender) = ctx.sender.as_ref().map(|s| s.to_lowercase()) {
                if !self.allowed_senders.iter().any(|c| c == &sender) {
                    return false;
                }
            } else {
                return false;
            }
        }
        true
    }
}

fn env_flag(key: &str, default_val: bool) -> bool {
    match env::var(key) {
        Ok(v) => matches!(
            v.trim().to_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        ),
        Err(_) => default_val,
    }
}

fn parse_list(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_when_disabled() {
        let cfg = ChatGateConfig {
            enabled: false,
            require_mention: false,
            allowed_channels: vec![],
            allowed_chat_types: vec![],
            allowed_senders: vec![],
        };
        let ctx = ChatGateContext {
            channel: None,
            chat_type: None,
            sender: None,
            mentioned: None,
        };
        assert!(cfg.is_allowed(&ctx));
    }

    #[test]
    fn blocks_without_mention_when_required() {
        let cfg = ChatGateConfig {
            enabled: true,
            require_mention: true,
            allowed_channels: vec![],
            allowed_chat_types: vec![],
            allowed_senders: vec![],
        };
        let ctx = ChatGateContext {
            channel: Some("telegram".to_string()),
            chat_type: Some("group".to_string()),
            sender: Some("user1".to_string()),
            mentioned: Some(false),
        };
        assert!(!cfg.is_allowed(&ctx));
    }

    #[test]
    fn allows_matching_channel_and_sender() {
        let cfg = ChatGateConfig {
            enabled: true,
            require_mention: false,
            allowed_channels: vec!["telegram".to_string()],
            allowed_chat_types: vec!["group".to_string()],
            allowed_senders: vec!["user1".to_string()],
        };
        let ctx = ChatGateContext {
            channel: Some("Telegram".to_string()),
            chat_type: Some("group".to_string()),
            sender: Some("User1".to_string()),
            mentioned: Some(false),
        };
        assert!(cfg.is_allowed(&ctx));
    }
}
