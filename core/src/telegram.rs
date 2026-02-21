use crate::controller::planner::RunGoalOutcome;
use crate::llm_gateway::LLMClient;
use crate::telegram_transport;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::sync::Semaphore;
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
    poll_timeout_sec: u64,
    llm: Arc<dyn LLMClient>,
    tx_analyzer: Option<mpsc::Sender<String>>,
}

impl TelegramBot {
    fn task_timeout_sec() -> u64 {
        std::env::var("STEER_TELEGRAM_TASK_TIMEOUT_SEC")
            .ok()
            .and_then(|v| v.trim().parse::<u64>().ok())
            .map(|v| v.clamp(30, 1800))
            .unwrap_or(240)
    }

    fn run_semaphore() -> Arc<Semaphore> {
        static SEM: std::sync::OnceLock<Arc<Semaphore>> = std::sync::OnceLock::new();
        SEM.get_or_init(|| {
            let max_parallel = std::env::var("STEER_TELEGRAM_MAX_CONCURRENT")
                .ok()
                .and_then(|v| v.trim().parse::<usize>().ok())
                .map(|v| v.clamp(1, 8))
                .unwrap_or(1);
            Arc::new(Semaphore::new(max_parallel))
        })
        .clone()
    }

    pub fn new(
        token: String,
        allowed_user_id: Option<u64>,
        llm: Arc<dyn LLMClient>,
        tx_analyzer: Option<mpsc::Sender<String>>,
    ) -> Self {
        let poll_timeout_sec = std::env::var("STEER_TELEGRAM_POLL_TIMEOUT_SEC")
            .ok()
            .and_then(|v| v.trim().parse::<u64>().ok())
            .map(|v| v.clamp(3, 60))
            .unwrap_or(12);
        let http_timeout_sec = std::env::var("STEER_TELEGRAM_HTTP_TIMEOUT_SEC")
            .ok()
            .and_then(|v| v.trim().parse::<u64>().ok())
            .map(|v| v.clamp(10, 120))
            .unwrap_or((poll_timeout_sec + 8).clamp(10, 120));

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(http_timeout_sec))
            .build()
            .unwrap_or_default();

        Self {
            token,
            allowed_user_id,
            client,
            poll_timeout_sec,
            llm,
            tx_analyzer,
        }
    }

    pub fn from_env(
        llm: Arc<dyn LLMClient>,
        tx_analyzer: Option<mpsc::Sender<String>>,
    ) -> Option<Self> {
        crate::load_env_with_fallback();
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
                                    let run_sem = Self::run_semaphore();
                                    let queued = run_sem.available_permits() == 0;

                                    // Ack reception
                                    let ack = if queued {
                                        "🤖 Command received. Queued after current task..."
                                    } else {
                                        "🤖 Command received. Processing..."
                                    };
                                    let _ = self
                                        .send_message(chat_id, ack)
                                        .await
                                        .map_err(|e| {
                                            error!("⚠️ Telegram ack send failed: {}", e);
                                            e
                                        });

                                    // Spawn agent task
                                    tokio::spawn(async move {
                                        let sem = Self::run_semaphore();
                                        let _permit = match sem.acquire_owned().await {
                                            Ok(v) => v,
                                            Err(_) => {
                                                let _ = bot_clone
                                                    .send_message_chunked(
                                                        chat_id,
                                                        "❌ Task Failed: internal queue unavailable",
                                                    )
                                                    .await;
                                                return;
                                            }
                                        };
                                        let planner = crate::controller::planner::Planner::new(
                                            bot_clone.llm.clone(),
                                            bot_clone.tx_analyzer.clone(),
                                        );
                                        let session_key = format!("telegram_chat_{}", chat_id);
                                        let timeout_sec = Self::task_timeout_sec();

                                        match tokio::time::timeout(
                                            Duration::from_secs(timeout_sec),
                                            planner.run_goal_tracked(&text_clone, Some(&session_key)),
                                        )
                                        .await
                                        {
                                            Ok(Ok(outcome)) => {
                                                let stage_runs = crate::db::list_task_stage_runs(
                                                    &outcome.run_id,
                                                )
                                                .unwrap_or_default();
                                                let assertions =
                                                    crate::db::list_task_stage_assertions(
                                                        &outcome.run_id,
                                                    )
                                                    .unwrap_or_default();
                                                let reply = Self::build_run_report(
                                                    &outcome,
                                                    &stage_runs,
                                                    &assertions,
                                                );
                                                let _ = bot_clone
                                                    .send_message_chunked(chat_id, &reply)
                                                    .await;
                                            }
                                            Ok(Err(e)) => {
                                                let _ = bot_clone
                                                    .send_message_chunked(
                                                        chat_id,
                                                        &format!("❌ Task Failed: {}", e),
                                                    )
                                                    .await;
                                            }
                                            Err(_) => {
                                                let _ = bot_clone
                                                    .send_message_chunked(
                                                        chat_id,
                                                        &format!(
                                                            "❌ Task Failed: timeout (>{}s). 다음 명령으로 넘어갑니다.",
                                                            timeout_sec
                                                        ),
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
                    let msg = e.to_string();
                    // Long-poll timeout can happen on unstable networks.
                    // Treat it as a soft timeout so next poll starts quickly.
                    if msg.to_ascii_lowercase().contains("timed out") {
                        info!("ℹ️ Telegram poll timeout; retrying quickly.");
                        tokio::time::sleep(tokio::time::Duration::from_millis(250)).await;
                    } else {
                        error!("⚠️ Telegram Poll Error: {}", msg);
                        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
                    }
                }
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        }
    }

    async fn get_updates(&self, offset: u64) -> Result<Vec<Update>> {
        let url = format!(
            "https://api.telegram.org/bot{}/getUpdates?offset={}&timeout={}",
            self.token, offset, self.poll_timeout_sec
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
        telegram_transport::send_message_chunked(
            &self.client,
            &self.token,
            &chat_id.to_string(),
            text,
            None,
            telegram_transport::DEFAULT_MAX_SEND_ATTEMPTS,
        )
        .await
    }

    pub async fn send_message_chunked(&self, chat_id: i64, text: &str) -> Result<()> {
        telegram_transport::send_message_chunked(
            &self.client,
            &self.token,
            &chat_id.to_string(),
            text,
            None,
            telegram_transport::DEFAULT_MAX_SEND_ATTEMPTS,
        )
        .await
    }

    fn build_run_report(
        outcome: &RunGoalOutcome,
        stage_runs: &[crate::db::TaskStageRunRecord],
        assertions: &[crate::db::TaskStageAssertionRecord],
    ) -> String {
        fn truncate_chars(s: &str, max_chars: usize) -> String {
            let mut out = String::new();
            for (idx, ch) in s.chars().enumerate() {
                if idx >= max_chars {
                    out.push_str("...");
                    break;
                }
                out.push(ch);
            }
            out
        }

        let summary = outcome.summary.clone().unwrap_or_else(|| "n/a".to_string());
        let mut latest_stage_map: BTreeMap<(i64, String), crate::db::TaskStageRunRecord> =
            BTreeMap::new();
        for stage in stage_runs {
            let key = (stage.stage_order, stage.stage_name.clone());
            match latest_stage_map.get(&key) {
                Some(prev) if prev.id >= stage.id => {}
                _ => {
                    latest_stage_map.insert(key, stage.clone());
                }
            }
        }
        let latest_stages: Vec<crate::db::TaskStageRunRecord> =
            latest_stage_map.into_values().collect();
        let stage_done_count = latest_stages
            .iter()
            .filter(|s| s.status.eq_ignore_ascii_case("completed"))
            .count();
        let stage_total = latest_stages.len();

        let mut lines = Vec::new();
        lines.push(if outcome.business_complete {
            "상태: ✅ 성공".to_string()
        } else {
            "상태: ❌ 실패".to_string()
        });
        lines.push(format!("run_id: {}", outcome.run_id));
        lines.push(format!("요약: {}", summary));
        lines.push("판정:".to_string());
        lines.push(format!("- planner_complete={}", outcome.planner_complete));
        lines.push(format!(
            "- execution_complete={}",
            outcome.execution_complete
        ));
        lines.push(format!("- business_complete={}", outcome.business_complete));
        lines.push(format!("- final_status={}", outcome.status));

        if !latest_stages.is_empty() {
            lines.push(format!("단계: {}/{} 완료", stage_done_count, stage_total));
            for stage in latest_stages.iter().take(8) {
                let details = stage.details.clone().unwrap_or_default();
                let short_details = if details.chars().count() > 120 {
                    truncate_chars(&details, 120)
                } else {
                    details
                };
                if short_details.is_empty() {
                    lines.push(format!(
                        "- {}.{}={}",
                        stage.stage_order, stage.stage_name, stage.status
                    ));
                } else {
                    lines.push(format!(
                        "- {}.{}={} ({})",
                        stage.stage_order, stage.stage_name, stage.status, short_details
                    ));
                }
            }
        }

        let mut failed_assertion_map: BTreeMap<
            (String, String),
            crate::db::TaskStageAssertionRecord,
        > = BTreeMap::new();
        for assertion in assertions.iter().filter(|a| !a.passed) {
            let key = (
                assertion.stage_name.clone(),
                assertion.assertion_key.clone(),
            );
            match failed_assertion_map.get(&key) {
                Some(prev) if prev.id >= assertion.id => {}
                _ => {
                    failed_assertion_map.insert(key, assertion.clone());
                }
            }
        }
        let failed_assertions: Vec<crate::db::TaskStageAssertionRecord> =
            failed_assertion_map.into_values().collect();
        if assertions.is_empty() {
            lines.push("검증: assertion 없음".to_string());
        } else if failed_assertions.is_empty() {
            lines.push(format!("검증: assertions 통과 ({})", assertions.len()));
        } else {
            lines.push(format!(
                "검증: assertions 실패 {}/{}",
                failed_assertions.len(),
                assertions.len()
            ));
            lines.push("실패 근거:".to_string());
            for assertion in failed_assertions.iter().take(6) {
                let evidence = assertion.evidence.clone().unwrap_or_default();
                let short_evidence = if evidence.chars().count() > 140 {
                    truncate_chars(&evidence, 140)
                } else {
                    evidence
                };
                lines.push(format!(
                    "- {}.{} expected={} actual={} evidence={}",
                    assertion.stage_name,
                    assertion.assertion_key,
                    assertion.expected,
                    assertion.actual,
                    if short_evidence.is_empty() {
                        "n/a".to_string()
                    } else {
                        short_evidence
                    }
                ));
            }
        }

        lines.push("다음 조치:".to_string());
        if outcome.business_complete {
            lines.push("- 추가 요청 실행 가능".to_string());
        } else {
            lines.push("- 실패 근거 항목부터 보강 후 재실행".to_string());
            lines.push("- 동일 요청 재실행 전 front 앱/입력 포커스 확인".to_string());
        }

        lines.join("\n")
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
        if self
            .send_message_chunked(chat_id, &refined_text)
            .await
            .is_ok()
        {
            return Ok(());
        }
        self.send_message_chunked(chat_id, raw_text).await
    }

    fn is_allowed(&self, user: &Option<User>) -> bool {
        match (self.allowed_user_id, user) {
            (Some(allowed), Some(u)) => u.id == allowed,
            (None, _) => true,
            _ => false,
        }
    }
}
