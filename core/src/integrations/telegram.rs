use anyhow::Result;
use reqwest::Client;

pub struct TelegramBot {
    token: String,
    chat_id: String,
    client: Client,
}

impl TelegramBot {
    pub fn new(token: &str, chat_id: &str) -> Self {
        Self {
            token: token.to_string(),
            chat_id: chat_id.to_string(),
            client: Client::new(),
        }
    }

    pub fn from_env() -> Result<Self> {
        dotenv::dotenv().ok();
        let token = std::env::var("TELEGRAM_BOT_TOKEN")
            .map_err(|_| anyhow::anyhow!("TELEGRAM_BOT_TOKEN not set"))?;
        let chat_id = std::env::var("TELEGRAM_CHAT_ID")
            .map_err(|_| anyhow::anyhow!("TELEGRAM_CHAT_ID not set"))?;
        Ok(Self::new(&token, &chat_id))
    }

    pub async fn send(&self, message: &str) -> Result<()> {
        let url = format!("https://api.telegram.org/bot{}/sendMessage", self.token);

        let params = [
            ("chat_id", self.chat_id.as_str()),
            ("text", message),
            ("parse_mode", "Markdown"),
        ];

        let resp = self.client.post(&url).form(&params).send().await?;

        if !resp.status().is_success() {
            let err = resp.text().await?;
            return Err(anyhow::anyhow!("Telegram API Error: {}", err));
        }

        Ok(())
    }
}
