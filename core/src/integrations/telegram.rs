use crate::send_policy::{self, SendDecision, SendPolicyContext};
use anyhow::Result;
use reqwest::Client;
use serde_json::Value;
use std::time::Duration;
use tokio::time::sleep;

pub struct TelegramBot {
    token: String,
    chat_id: String,
    client: Client,
}

impl TelegramBot {
    const TEXT_CHUNK_LIMIT: usize = 3900;
    const MAX_SEND_ATTEMPTS: u32 = 4;

    pub fn new(token: &str, chat_id: &str) -> Self {
        Self {
            token: token.to_string(),
            chat_id: chat_id.to_string(),
            client: Client::new(),
        }
    }

    pub fn from_env() -> Result<Self> {
        crate::load_env_with_fallback();
        let token = std::env::var("TELEGRAM_BOT_TOKEN")
            .map_err(|_| anyhow::anyhow!("TELEGRAM_BOT_TOKEN not set"))?;
        let chat_id = std::env::var("TELEGRAM_CHAT_ID")
            .map_err(|_| anyhow::anyhow!("TELEGRAM_CHAT_ID not set"))?;
        Ok(Self::new(&token, &chat_id))
    }

    fn split_message_chunks(message: &str, max_len: usize) -> Vec<String> {
        if message.trim().is_empty() {
            return Vec::new();
        }
        let mut chunks: Vec<String> = Vec::new();
        let mut current = String::new();

        for line in message.lines() {
            let line = line.trim_end();
            let candidate = if current.is_empty() {
                line.to_string()
            } else {
                format!("{}\n{}", current, line)
            };
            if candidate.chars().count() <= max_len {
                current = candidate;
                continue;
            }

            if !current.is_empty() {
                chunks.push(current);
                current = String::new();
            }

            if line.chars().count() <= max_len {
                current = line.to_string();
                continue;
            }

            let mut segment = String::new();
            for ch in line.chars() {
                segment.push(ch);
                if segment.chars().count() >= max_len {
                    chunks.push(segment);
                    segment = String::new();
                }
            }
            if !segment.is_empty() {
                current = segment;
            }
        }

        if !current.is_empty() {
            chunks.push(current);
        }
        chunks
    }

    fn parse_retry_after_seconds(body: &str) -> Option<u64> {
        let parsed: Value = serde_json::from_str(body).ok()?;
        if let Some(v) = parsed
            .get("parameters")
            .and_then(|p| p.get("retry_after"))
            .and_then(|v| v.as_u64())
        {
            return Some(v.max(1));
        }
        parsed
            .get("retry_after")
            .and_then(|v| v.as_u64())
            .map(|v| v.max(1))
    }

    async fn send_chunk_with_retry(&self, chunk: &str, parse_mode: Option<&str>) -> Result<()> {
        let url = format!("https://api.telegram.org/bot{}/sendMessage", self.token);
        let mut backoff_secs: u64 = 1;

        for attempt in 1..=Self::MAX_SEND_ATTEMPTS {
            let mut params = vec![("chat_id", self.chat_id.as_str()), ("text", chunk)];
            if let Some(mode) = parse_mode {
                params.push(("parse_mode", mode));
            }

            match self.client.post(&url).form(&params).send().await {
                Ok(resp) => {
                    if resp.status().is_success() {
                        return Ok(());
                    }
                    let status = resp.status();
                    let body = resp.text().await.unwrap_or_default();
                    let retryable = status.as_u16() == 429 || status.is_server_error();
                    if retryable && attempt < Self::MAX_SEND_ATTEMPTS {
                        let retry_after = Self::parse_retry_after_seconds(&body)
                            .unwrap_or(backoff_secs)
                            .min(30);
                        sleep(Duration::from_secs(retry_after)).await;
                        backoff_secs = (backoff_secs * 2).min(30);
                        continue;
                    }
                    return Err(anyhow::anyhow!(
                        "Telegram API Error (status={}): {}",
                        status,
                        body
                    ));
                }
                Err(e) => {
                    if attempt < Self::MAX_SEND_ATTEMPTS {
                        sleep(Duration::from_secs(backoff_secs.min(30))).await;
                        backoff_secs = (backoff_secs * 2).min(30);
                        continue;
                    }
                    return Err(anyhow::anyhow!("Telegram request failed: {}", e));
                }
            }
        }

        Err(anyhow::anyhow!("Telegram send failed after retries"))
    }

    async fn send_message_internal(&self, message: &str, parse_mode: Option<&str>) -> Result<()> {
        let ctx = SendPolicyContext {
            session_key: Some(format!("telegram_chat_{}", self.chat_id)),
            channel: Some("telegram".to_string()),
            chat_type: None,
            target_id: Some(self.chat_id.clone()),
        };
        if matches!(
            send_policy::should_send_with_context("telegram", message, Some(&ctx)),
            SendDecision::Deny
        ) {
            println!(
                "🔕 [TELEGRAM] Suppressed by send policy (chat_id={})",
                self.chat_id
            );
            return Ok(());
        }

        let chunks = Self::split_message_chunks(message, Self::TEXT_CHUNK_LIMIT);
        if chunks.is_empty() {
            return Ok(());
        }
        for chunk in chunks {
            self.send_chunk_with_retry(&chunk, parse_mode).await?;
        }
        Ok(())
    }

    fn is_markdown_parse_error(err: &anyhow::Error) -> bool {
        let msg = err.to_string().to_lowercase();
        msg.contains("can't parse entities") || msg.contains("parse entities")
    }

    pub async fn send(&self, message: &str) -> Result<()> {
        match self.send_message_internal(message, Some("Markdown")).await {
            Ok(()) => Ok(()),
            Err(e) if Self::is_markdown_parse_error(&e) => {
                self.send_message_internal(message, None).await
            }
            Err(e) => Err(e),
        }
    }

    pub async fn send_plain(&self, message: &str) -> Result<()> {
        self.send_message_internal(message, None).await
    }
}
