use crate::db::ChatMessage;
use chrono::{DateTime, Local, TimeZone, Utc};
use std::env;

#[derive(Debug, Clone)]
pub struct ContextPruneConfig {
    pub max_messages: usize,
    pub ttl_seconds: Option<i64>,
    pub session_reset: SessionResetConfig,
}

impl ContextPruneConfig {
    pub fn from_env() -> Self {
        let max_messages = env::var("CONTEXT_PRUNE_MAX_MESSAGES")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(8);

        let ttl_seconds = env::var("CONTEXT_PRUNE_TTL_SECONDS")
            .ok()
            .and_then(|v| v.parse::<i64>().ok())
            .filter(|v| *v > 0);

        let session_reset = SessionResetConfig::from_env();

        Self {
            max_messages,
            ttl_seconds,
            session_reset,
        }
    }
}

pub fn history_fetch_limit() -> i64 {
    let cfg = ContextPruneConfig::from_env();
    let min_fetch = std::cmp::max(10, cfg.max_messages * 2) as i64;
    min_fetch
}

pub fn prune_chat_history(history: &[ChatMessage]) -> Vec<ChatMessage> {
    let cfg = ContextPruneConfig::from_env();
    let mut filtered: Vec<ChatMessage> = history.to_vec();

    if let Some(cutoff) = cfg.session_reset.cutoff_utc() {
        filtered = filtered
            .into_iter()
            .filter(|msg| {
                DateTime::parse_from_rfc3339(&msg.created_at)
                    .ok()
                    .map(|ts| ts.with_timezone(&Utc) >= cutoff)
                    .unwrap_or(true)
            })
            .collect();
    }

    if let Some(ttl) = cfg.ttl_seconds {
        let now = Utc::now();
        filtered = filtered
            .into_iter()
            .filter(|msg| {
                DateTime::parse_from_rfc3339(&msg.created_at)
                    .ok()
                    .map(|ts| {
                        now.signed_duration_since(ts.with_timezone(&Utc))
                            .num_seconds()
                            <= ttl
                    })
                    .unwrap_or(true)
            })
            .collect();
    }

    if cfg.max_messages > 0 && filtered.len() > cfg.max_messages {
        let start = filtered.len() - cfg.max_messages;
        filtered = filtered[start..].to_vec();
    }

    filtered
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SessionResetMode {
    Off,
    Daily,
    Idle,
    DailyOrIdle,
}

#[derive(Debug, Clone)]
pub struct SessionResetConfig {
    pub mode: SessionResetMode,
    pub at_hour: u32,
    pub idle_minutes: Option<i64>,
}

impl SessionResetConfig {
    pub fn from_env() -> Self {
        let raw_mode = env::var("SESSION_RESET_MODE").unwrap_or_else(|_| "off".to_string());
        let mode = match raw_mode.trim().to_lowercase().as_str() {
            "daily" => SessionResetMode::Daily,
            "idle" => SessionResetMode::Idle,
            "both" | "daily_idle" | "daily-or-idle" | "daily_or_idle" => {
                SessionResetMode::DailyOrIdle
            }
            _ => SessionResetMode::Off,
        };

        let at_hour = env::var("SESSION_RESET_AT_HOUR")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .filter(|v| *v <= 23)
            .unwrap_or(4);

        let idle_minutes = env::var("SESSION_RESET_IDLE_MINUTES")
            .ok()
            .and_then(|v| v.parse::<i64>().ok())
            .filter(|v| *v > 0);

        Self {
            mode,
            at_hour,
            idle_minutes,
        }
    }

    pub fn cutoff_utc(&self) -> Option<DateTime<Utc>> {
        match self.mode {
            SessionResetMode::Off => None,
            SessionResetMode::Idle => self.idle_cutoff(),
            SessionResetMode::Daily => self.daily_cutoff(None),
            SessionResetMode::DailyOrIdle => {
                let daily = self.daily_cutoff(self.idle_minutes);
                let idle = self.idle_cutoff();
                match (daily, idle) {
                    (Some(d), Some(i)) => Some(if d > i { d } else { i }),
                    (Some(d), None) => Some(d),
                    (None, Some(i)) => Some(i),
                    _ => None,
                }
            }
        }
    }

    fn daily_cutoff(&self, idle_minutes: Option<i64>) -> Option<DateTime<Utc>> {
        let now = Local::now();
        let today = now.date_naive();
        let boundary_naive = today.and_hms_opt(self.at_hour, 0, 0)?;
        let boundary_local = match Local.from_local_datetime(&boundary_naive) {
            chrono::LocalResult::Single(dt) => dt,
            chrono::LocalResult::Ambiguous(dt, _) => dt,
            chrono::LocalResult::None => return None,
        };
        let mut boundary = boundary_local;
        if now < boundary {
            boundary = boundary - chrono::Duration::days(1);
        }

        let mut cutoff = boundary.with_timezone(&Utc);
        if let Some(minutes) = idle_minutes {
            if minutes > 0 {
                let idle_cutoff = (now - chrono::Duration::minutes(minutes)).with_timezone(&Utc);
                if idle_cutoff > cutoff {
                    cutoff = idle_cutoff;
                }
            }
        }
        Some(cutoff)
    }

    fn idle_cutoff(&self) -> Option<DateTime<Utc>> {
        let minutes = self.idle_minutes?;
        if minutes <= 0 {
            return None;
        }
        let now = Utc::now();
        Some(now - chrono::Duration::minutes(minutes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn make_message(role: &str, content: &str, created_at: &str) -> ChatMessage {
        ChatMessage {
            role: role.to_string(),
            content: content.to_string(),
            created_at: created_at.to_string(),
        }
    }

    #[test]
    fn prunes_to_max_messages() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var("CONTEXT_PRUNE_MAX_MESSAGES", "2");
        std::env::remove_var("CONTEXT_PRUNE_TTL_SECONDS");
        std::env::set_var("SESSION_RESET_MODE", "off");
        std::env::remove_var("SESSION_RESET_IDLE_MINUTES");
        std::env::remove_var("SESSION_RESET_AT_HOUR");

        let history = vec![
            make_message("user", "a", "2024-01-01T00:00:00Z"),
            make_message("assistant", "b", "2024-01-01T00:00:01Z"),
            make_message("user", "c", "2024-01-01T00:00:02Z"),
        ];
        let pruned = prune_chat_history(&history);
        assert_eq!(pruned.len(), 2);
        assert_eq!(pruned[0].content, "b");
        assert_eq!(pruned[1].content, "c");
    }

    #[test]
    fn prunes_by_ttl() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var("CONTEXT_PRUNE_MAX_MESSAGES", "10");
        std::env::set_var("CONTEXT_PRUNE_TTL_SECONDS", "1");
        std::env::set_var("SESSION_RESET_MODE", "off");

        let now = Utc::now();
        let old = now - chrono::Duration::seconds(10);
        let history = vec![
            make_message("user", "old", &old.to_rfc3339()),
            make_message("assistant", "new", &now.to_rfc3339()),
        ];
        let pruned = prune_chat_history(&history);
        assert_eq!(pruned.len(), 1);
        assert_eq!(pruned[0].content, "new");
    }

    #[test]
    fn prunes_by_idle_reset() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var("CONTEXT_PRUNE_MAX_MESSAGES", "10");
        std::env::remove_var("CONTEXT_PRUNE_TTL_SECONDS");
        std::env::set_var("SESSION_RESET_MODE", "idle");
        std::env::set_var("SESSION_RESET_IDLE_MINUTES", "1");

        let now = Utc::now();
        let old = now - chrono::Duration::minutes(10);
        let history = vec![
            make_message("user", "old", &old.to_rfc3339()),
            make_message("assistant", "new", &now.to_rfc3339()),
        ];
        let pruned = prune_chat_history(&history);
        assert_eq!(pruned.len(), 1);
        assert_eq!(pruned[0].content, "new");
    }
}
