use crate::llm_gateway::LLMClient;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;
use tokio::sync::mpsc;
// Duplicate removed
use log::{error, info};

#[derive(Serialize, Deserialize, Debug)]
struct Update {
    update_id: u64,
    message: Option<Message>,
}

#[derive(Serialize, Deserialize, Debug)]
struct Message {
    message_id: u64,
    from: Option<User>,
    chat: Chat,
    text: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
struct User {
    id: u64,
    username: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
struct Chat {
    id: i64,
}

#[derive(Serialize, Deserialize, Debug)]
struct GetUpdatesResponse {
    ok: bool,
    result: Vec<Update>,
}

pub struct TelegramBot {
    token: String,
    allowed_user_id: Option<u64>,
    client: reqwest::Client,
    llm: Arc<dyn LLMClient>,
    tx_analyzer: Option<mpsc::Sender<String>>,
}

impl TelegramBot {
    pub fn new(
        token: String,
        allowed_user_id: Option<u64>,
        llm: Arc<dyn LLMClient>,
        tx_analyzer: Option<mpsc::Sender<String>>,
    ) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .unwrap_or_default();

        Self {
            token,
            allowed_user_id,
            client,
            llm,
            tx_analyzer,
        }
    }

    pub fn from_env(
        llm: Arc<dyn LLMClient>,
        tx_analyzer: Option<mpsc::Sender<String>>,
    ) -> Option<Self> {
        let token = std::env::var("TELEGRAM_BOT_TOKEN").ok()?;
        let allowed_user_id = std::env::var("TELEGRAM_USER_ID")
            .ok()
            .and_then(|id| id.parse().ok());

        Some(Self::new(token, allowed_user_id, llm, tx_analyzer))
    }

    pub async fn start_polling(self: Arc<Self>) {
        info!("🤖 Telegram Bot started. Waiting for messages...");
        let mut offset = 0;

        loop {
            match self.get_updates(offset).await {
                Ok(updates) => {
                    for update in updates {
                        offset = update.update_id + 1;
                        if let Some(msg) = update.message {
                            if let Some(text) = msg.text {
                                if self.is_allowed(&msg.from) {
                                    info!("📩 Received command: '{}'", text);
                                    let bot_clone = self.clone();
                                    let chat_id = msg.chat.id;
                                    let text_clone = text.clone();

                                    // Ack reception
                                    let _ = self
                                        .send_message(chat_id, "🤖 Command received. Processing...")
                                        .await;

                                    // Spawn agent task
                                    tokio::spawn(async move {
                                        let planner = crate::controller::planner::Planner::new(
                                            bot_clone.llm.clone(),
                                            bot_clone.tx_analyzer.clone(),
                                        );
                                        let session_key = format!("telegram_chat_{}", chat_id);

                                        match planner
                                            .run_goal_tracked(&text_clone, Some(&session_key))
                                            .await
                                        {
                                            Ok(outcome) => {
                                                let summary = outcome
                                                    .summary
                                                    .clone()
                                                    .unwrap_or_else(|| "n/a".to_string());
                                                let reply = if outcome.business_complete {
                                                    format!(
                                                        "✅ Task Completed.\nrun_id={}\nstatus={}\nplanner_complete={}\nexecution_complete={}\nbusiness_complete={}\nsummary={}",
                                                        outcome.run_id,
                                                        outcome.status,
                                                        outcome.planner_complete,
                                                        outcome.execution_complete,
                                                        outcome.business_complete,
                                                        summary
                                                    )
                                                } else {
                                                    format!(
                                                        "❌ Task Incomplete.\nrun_id={}\nstatus={}\nplanner_complete={}\nexecution_complete={}\nbusiness_complete={}\nsummary={}\n로그/증거를 확인하세요.",
                                                        outcome.run_id,
                                                        outcome.status,
                                                        outcome.planner_complete,
                                                        outcome.execution_complete,
                                                        outcome.business_complete,
                                                        summary
                                                    )
                                                };
                                                let _ =
                                                    bot_clone.send_message(chat_id, &reply).await;
                                            }
                                            Err(e) => {
                                                let _ = bot_clone
                                                    .send_message(
                                                        chat_id,
                                                        &format!("❌ Task Failed: {}", e),
                                                    )
                                                    .await;
                                            }
                                        }
                                    });
                                } else {
                                    info!(
                                        "🚫 Ignored message from unauthorized user: {:?}",
                                        msg.from
                                    );
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    error!("⚠️ Telegram Poll Error: {}", e);
                    tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                }
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        }
    }

    async fn get_updates(&self, offset: u64) -> Result<Vec<Update>> {
        let url = format!(
            "https://api.telegram.org/bot{}/getUpdates?offset={}&timeout=30",
            self.token, offset
        );
        let resp = self.client.get(&url).send().await?;
        if !resp.status().is_success() {
            return Err(anyhow::anyhow!("Telegram API Error: {}", resp.status()));
        }
        let body: GetUpdatesResponse = resp.json().await?;
        if !body.ok {
            return Err(anyhow::anyhow!("Telegram API returned ok=false"));
        }
        Ok(body.result)
    }

    pub async fn send_message(&self, chat_id: i64, text: &str) -> Result<()> {
        let url = format!("https://api.telegram.org/bot{}/sendMessage", self.token);
        let body = json!({
            "chat_id": chat_id,
            "text": text
        });
        let _ = self.client.post(&url).json(&body).send().await?;
        Ok(())
    }

    pub async fn improve_message(&self, raw_text: &str) -> String {
        let messages = vec![
            json!({
                "role": "system",
                "content": concat!(
                    "너는 로컬 OS 에이전트 실행 결과를 텔레그램용으로 정리하는 리포터다.\n",
                    "입력은 테스트 시나리오 로그 또는 자연어 요청 실행 요약이다.\n",
                    "입력에 있는 사실만 사용하고 추측하지 마라.\n",
                    "출력은 반드시 한국어로 작성한다.\n",
                    "구분선(---), 장식용 이모지, 불필요한 수식어는 금지한다.\n",
                    "반드시 아래 형식을 정확히 지켜라:\n",
                    "작업: ...\n",
                    "요청: ...\n",
                    "수행: ...\n",
                    "결과: ...\n",
                    "상태: ✅ 성공 또는 ❌ 실패\n",
                    "근거:\n",
                    "- ...\n",
                    "- ...\n",
                    "- ...\n",
                    "- 로그: 파일명\n",
                    "- 캡처: 파일명\n",
                    "근거 불릿은 최소 3개 이상 작성하라.\n",
                    "입력에 로그/캡처 파일명이 있으면 그대로 포함하라.\n",
                    "입력에 'Node evidence:' 또는 '노드 캡처' 항목이 있으면 근거에 반드시 포함하라.\n",
                    "Node evidence가 있으면 최소 2개 이상 불릿으로 유지하라.\n",
                    "노드 캡처 수/노드 캡처 폴더/노드샷 항목이 있으면 삭제하지 마라.\n",
                    "각 줄은 짧고 명확하게 작성하라.\n",
                    "근거 불릿은 입력 원문의 로그 문장을 그대로 또는 최소한으로만 축약해서 작성하라.\n",
                    "입력에 없는 사실(예: 실제 메모 작성 완료, 전송 완료, 저장 완료)을 추정해 쓰지 마라.\n",
                    "open_app 로그만 있으면 '앱 열림' 수준으로만 기술하고 작업 완료를 확대 해석하지 마라.\n",
                    "실패 신호(error, failed, panic, ❌, refused)가 있으면 상태는 ❌ 실패로 작성한다.\n",
                    "정보가 부족하면 '확인 불가'라고 명시한다."
                )
            }),
            json!({
                "role": "user",
                "content": format!("Raw Input: \"{}\"", raw_text)
            }),
        ];

        match self.llm.chat_completion(messages).await {
            Ok(refined) => refined.trim().to_string(),
            Err(_) => raw_text.to_string(),
        }
    }

    pub async fn send_smart_notification(&self, chat_id: i64, raw_text: &str) -> Result<()> {
        let refined_text = self.improve_message(raw_text).await;
        let url = format!("https://api.telegram.org/bot{}/sendMessage", self.token);
        let body = json!({
            "chat_id": chat_id,
            "text": refined_text
        });

        let resp = self.client.post(&url).json(&body).send().await?;
        if !resp.status().is_success() {
            // Fallback to raw text if markdown parsing fails
            let body_fallback = json!({
                "chat_id": chat_id,
                "text": format!("{}\n(Refined format failed, raw text sent)", raw_text)
            });
            let _ = self.client.post(&url).json(&body_fallback).send().await?;
        }
        Ok(())
    }

    fn is_allowed(&self, user: &Option<User>) -> bool {
        match (self.allowed_user_id, user) {
            (Some(allowed), Some(u)) => u.id == allowed,
            (None, _) => true,
            _ => false,
        }
    }
}
