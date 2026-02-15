use crate::send_policy::{self, SendDecision, SendPolicyContext};
use crate::telegram_transport;
use anyhow::Result;
use reqwest::Client;

pub struct TelegramBot {
    token: String,
    chat_id: String,
    client: Client,
}

impl TelegramBot {
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

        telegram_transport::send_message_chunked(
            &self.client,
            &self.token,
            &self.chat_id,
            message,
            parse_mode,
            Self::MAX_SEND_ATTEMPTS,
        )
        .await
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
