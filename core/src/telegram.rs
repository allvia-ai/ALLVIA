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
    result: Option<Vec<Update>>,
}

#[derive(Deserialize, Debug)]
struct TelegramApiStatusResponse {
    ok: bool,
    description: Option<String>,
}

pub struct TelegramBot {
    token: String,
    allowed_user_id: Option<u64>,
    client: reqwest::Client,
    poll_timeout_sec: u64,
    llm: Arc<dyn LLMClient>,
    tx_analyzer: Option<mpsc::Sender<String>>,
}

#[derive(Debug, Default, Clone)]
struct RunResultContext {
    links: Vec<String>,
    highlights: Vec<String>,
}

impl TelegramBot {
    fn env_truthy_default(key: &str, default_value: bool) -> bool {
        match std::env::var(key) {
            Ok(v) => matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            ),
            Err(_) => default_value,
        }
    }

    fn is_webhook_conflict_error(msg: &str) -> bool {
        let lower = msg.to_ascii_lowercase();
        (lower.contains("409") && lower.contains("conflict"))
            || (lower.contains("webhook") && lower.contains("active"))
            || lower.contains("can't use getupdates")
    }

    fn task_timeout_sec() -> u64 {
        std::env::var("STEER_TELEGRAM_TASK_TIMEOUT_SEC")
            .ok()
            .and_then(|v| v.trim().parse::<u64>().ok())
            .map(|v| v.clamp(30, 1800))
            .unwrap_or(240)
    }

    fn infer_n8n_digest_request(message: &str) -> Option<String> {
        if let Some(explicit) = crate::ai_digest::extract_explicit_n8n_request(message) {
            return Some(explicit);
        }
        if !Self::env_truthy_default("STEER_TELEGRAM_AUTO_ROUTE_AI_DIGEST", true) {
            return None;
        }
        if !crate::ai_digest::looks_like_ai_digest_request(message) {
            return None;
        }
        if crate::ai_digest::resolve_program_webhook_url().is_err() {
            return None;
        }
        let cleaned = crate::ai_digest::strip_local_execution_prefix(message);
        if cleaned.trim().is_empty() {
            Some(crate::ai_digest::default_request_text().to_string())
        } else {
            Some(cleaned)
        }
    }

    async fn delete_webhook(&self, drop_pending_updates: bool) -> Result<()> {
        let drop_param = if drop_pending_updates {
            "true"
        } else {
            "false"
        };
        let url = format!(
            "https://api.telegram.org/bot{}/deleteWebhook?drop_pending_updates={}",
            self.token, drop_param
        );
        let resp = self.client.get(&url).send().await?;
        let status = resp.status();
        let body_text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(anyhow::anyhow!(
                "deleteWebhook failed (status={}): {}",
                status,
                body_text
            ));
        }
        let parsed: TelegramApiStatusResponse = serde_json::from_str(&body_text).map_err(|e| {
            anyhow::anyhow!("deleteWebhook parse failed: {} (body={})", e, body_text)
        })?;
        if !parsed.ok {
            return Err(anyhow::anyhow!(
                "deleteWebhook returned ok=false: {}",
                parsed
                    .description
                    .unwrap_or_else(|| "unknown error".to_string())
            ));
        }
        Ok(())
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
        let auto_clear_webhook =
            Self::env_truthy_default("STEER_TELEGRAM_CLEAR_WEBHOOK_ON_POLL", true);
        if auto_clear_webhook {
            match self.delete_webhook(false).await {
                Ok(_) => info!("ℹ️ Telegram webhook cleared for long polling."),
                Err(e) => error!("⚠️ Telegram webhook clear failed: {}", e),
            }
        }
        let mut offset = 0;
        let mut last_webhook_reset: Option<std::time::Instant> = None;

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
                                    let n8n_request =
                                        Self::infer_n8n_digest_request(&text);
                                    let route_to_n8n = n8n_request.is_some();
                                    let run_sem = Self::run_semaphore();
                                    let queued = run_sem.available_permits() == 0;

                                    // Ack reception
                                    let ack = if route_to_n8n {
                                        "🤖 Command received. Routing to n8n digest..."
                                    } else if queued {
                                        "🤖 Command received. Queued after current task..."
                                    } else {
                                        "🤖 Command received. Processing..."
                                    };
                                    let _ = self.send_message(chat_id, ack).await.map_err(|e| {
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

                                        if let Some(request_text) = n8n_request {
                                            let reply =
                                                match crate::ai_digest::trigger_program_webhook(
                                                    request_text.trim(),
                                                    None,
                                                )
                                                .await
                                                {
                                                    Ok(result) => {
                                                        crate::ai_digest::format_human_summary(
                                                            &result,
                                                        )
                                                    }
                                                    Err(e) => {
                                                        format!(
                                                            "❌ n8n digest trigger failed: {}",
                                                            e
                                                        )
                                                    }
                                                };
                                            let _ = bot_clone
                                                .send_message_chunked(chat_id, &reply)
                                                .await;
                                            return;
                                        }

                                        let goal_text = {
                                            let cleaned =
                                                crate::ai_digest::strip_local_execution_prefix(
                                                    &text_clone,
                                                );
                                            if cleaned.is_empty() {
                                                text_clone.clone()
                                            } else {
                                                cleaned
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
                                            planner
                                                .run_goal_tracked(&goal_text, Some(&session_key)),
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
                                                let result_context =
                                                    Self::collect_result_context(&session_key);
                                                let reply = Self::build_run_report(
                                                    &outcome,
                                                    &stage_runs,
                                                    &assertions,
                                                    &goal_text,
                                                    &result_context,
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
                                    let _ = self
                                        .send_message(
                                            msg.chat.id,
                                            "⛔️ 허용된 사용자만 이 봇을 사용할 수 있습니다.",
                                        )
                                        .await
                                        .map_err(|e| {
                                            error!(
                                                "⚠️ Telegram unauthorized notice send failed: {}",
                                                e
                                            );
                                            e
                                        });
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    let msg = e.to_string();
                    if auto_clear_webhook && Self::is_webhook_conflict_error(&msg) {
                        let should_retry_reset = last_webhook_reset
                            .map(|ts| ts.elapsed() >= Duration::from_secs(30))
                            .unwrap_or(true);
                        if should_retry_reset {
                            match self.delete_webhook(true).await {
                                Ok(_) => info!(
                                    "ℹ️ Telegram webhook conflict recovered (deleteWebhook drop_pending_updates=true)."
                                ),
                                Err(reset_err) => error!(
                                    "⚠️ Telegram webhook conflict recovery failed: {}",
                                    reset_err
                                ),
                            }
                            last_webhook_reset = Some(std::time::Instant::now());
                        }
                        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                        continue;
                    }
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
        let status = resp.status();
        let body_text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(anyhow::anyhow!(
                "Telegram API Error: {} ({})",
                status,
                body_text
            ));
        }
        let body: GetUpdatesResponse = serde_json::from_str(&body_text).map_err(|e| {
            anyhow::anyhow!(
                "Telegram getUpdates decode failed: {} (body={})",
                e,
                body_text
            )
        })?;
        if !body.ok {
            return Err(anyhow::anyhow!(
                "Telegram API returned ok=false: {}",
                body_text
            ));
        }
        Ok(body.result.unwrap_or_default())
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
        goal: &str,
        result_context: &RunResultContext,
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

        if outcome.business_complete {
            let mut lines = Vec::new();
            lines.push("상태: ✅ 성공".to_string());
            lines.push(format!("요청: {}", truncate_chars(goal.trim(), 120)));
            if !summary.is_empty() && summary != "n/a" {
                lines.push(format!("결과: {}", truncate_chars(&summary, 160)));
            }
            if !result_context.highlights.is_empty() {
                lines.push("핵심 요약:".to_string());
                for item in result_context.highlights.iter().take(5) {
                    lines.push(format!("- {}", truncate_chars(item, 240)));
                }
            }
            if !result_context.links.is_empty() {
                lines.push("노션 링크:".to_string());
                for link in result_context.links.iter().take(4) {
                    lines.push(format!("- {}", link));
                }
            }
            lines.push(format!("run_id: {}", outcome.run_id));
            lines.push("다음 조치:".to_string());
            lines.push("- 추가 요청 실행 가능".to_string());
            return lines.join("\n");
        }

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

        if !result_context.links.is_empty() {
            lines.push("결과 링크:".to_string());
            for link in result_context.links.iter().take(4) {
                lines.push(format!("- {}", link));
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

    fn collect_result_context(session_key: &str) -> RunResultContext {
        if session_key.trim().is_empty() {
            return RunResultContext::default();
        }
        let _ = crate::session_store::init_session_store();
        let guard = match crate::session_store::get_session_store() {
            Ok(g) => g,
            Err(_) => return RunResultContext::default(),
        };
        let store = match guard.as_ref() {
            Some(s) => s,
            None => return RunResultContext::default(),
        };
        let session = match store.get_latest_by_key(session_key) {
            Some(s) => s,
            None => return RunResultContext::default(),
        };
        Self::extract_result_context_from_steps(&session.steps)
    }

    fn extract_result_context_from_steps(
        steps: &[crate::session_store::SessionStep],
    ) -> RunResultContext {
        let mut ctx = RunResultContext {
            links: Self::extract_result_links_from_steps(steps),
            highlights: Vec::new(),
        };

        for step in steps.iter().rev() {
            if !step.action_type.eq_ignore_ascii_case("notion_write") {
                continue;
            }
            if let Some(data) = step.data.as_ref() {
                if let Some(preview) = data.get("content_preview").and_then(|v| v.as_str()) {
                    ctx.highlights = Self::extract_news_highlights_from_preview(preview);
                }
            }
            break;
        }

        ctx
    }

    fn extract_result_links_from_steps(
        steps: &[crate::session_store::SessionStep],
    ) -> Vec<String> {
        fn push_http_link(out: &mut Vec<String>, raw: &str) {
            let candidate = raw
                .trim()
                .trim_matches(|c| c == '"' || c == '\'' || c == '`');
            if !(candidate.starts_with("https://") || candidate.starts_with("http://")) {
                return;
            }
            if !out.iter().any(|v| v.eq_ignore_ascii_case(candidate)) {
                out.push(candidate.to_string());
            }
        }

        let mut links = Vec::new();
        for step in steps.iter().rev() {
            if let Some(data) = step.data.as_ref() {
                for key in ["page_url", "workflow_url", "execution_url", "run_url"] {
                    if let Some(url) = data.get(key).and_then(|v| v.as_str()) {
                        push_http_link(&mut links, url);
                    }
                }
            }

            if let Some(url) = step.description.strip_prefix("Notion page created: ") {
                push_http_link(&mut links, url);
            }
        }
        links
    }

    fn extract_news_highlights_from_preview(preview: &str) -> Vec<String> {
        fn trim_title_line(line: &str) -> Option<String> {
            let trimmed = line.trim();
            let (num, rest) = trimmed.split_once('.')?;
            if !num.chars().all(|c| c.is_ascii_digit()) {
                return None;
            }
            let title = rest.trim();
            if title.is_empty() {
                None
            } else {
                Some(title.to_string())
            }
        }

        fn shorten(s: &str, max_chars: usize) -> String {
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

        let lines: Vec<&str> = preview.lines().collect();
        let mut highlights = Vec::new();
        let mut idx = 0usize;

        while idx < lines.len() && highlights.len() < 5 {
            let current = lines[idx].trim();
            let Some(title) = trim_title_line(current) else {
                idx += 1;
                continue;
            };

            let mut link = String::new();
            let mut core = String::new();
            let mut j = idx + 1;
            while j < lines.len() {
                let next = lines[j].trim();
                if next.is_empty() || trim_title_line(next).is_some() {
                    break;
                }
                if link.is_empty() {
                    if let Some(v) = next.strip_prefix("링크:") {
                        link = v.trim().to_string();
                    }
                }
                if core.is_empty() {
                    if let Some(v) = next.strip_prefix("- 핵심:") {
                        core = v.trim().to_string();
                    }
                }
                j += 1;
            }

            let mut bullet = title;
            if !core.is_empty() {
                bullet.push_str(" — ");
                bullet.push_str(&core);
            }
            if !link.is_empty() {
                bullet.push_str(" (");
                bullet.push_str(&link);
                bullet.push(')');
            }
            highlights.push(shorten(&bullet, 260));
            idx = j.max(idx + 1);
        }

        highlights
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

#[cfg(test)]
mod tests {
    use super::TelegramBot;
    use chrono::Utc;
    use serde_json::json;

    #[test]
    fn webhook_conflict_detector_matches_telegram_conflict_messages() {
        assert!(TelegramBot::is_webhook_conflict_error(
            "Telegram API Error: 409 Conflict: terminated by other getUpdates request"
        ));
        assert!(TelegramBot::is_webhook_conflict_error(
            "Conflict: can't use getUpdates method while webhook is active"
        ));
        assert!(!TelegramBot::is_webhook_conflict_error(
            "Telegram API Error: 401 Unauthorized"
        ));
    }

    #[test]
    fn extract_result_context_from_steps_includes_notion_page_url() {
        let steps = vec![crate::session_store::SessionStep {
            step_index: 0,
            action_type: "notion_write".to_string(),
            description: "Notion page created: https://www.notion.so/abcd1234".to_string(),
            status: "success".to_string(),
            timestamp: Utc::now(),
            data: Some(json!({
                "page_id": "abcd1234",
                "page_url": "https://www.notion.so/abcd1234",
                "content_preview": "AI 뉴스 기사 요약\n1. 제목 A\n링크: https://example.com/a\n요약:\n- 핵심: 요약 A"
            })),
        }];
        let ctx = TelegramBot::extract_result_context_from_steps(&steps);
        assert_eq!(ctx.links, vec!["https://www.notion.so/abcd1234".to_string()]);
        assert!(!ctx.highlights.is_empty());
    }

    #[test]
    fn infer_n8n_digest_request_auto_routes_news_to_notion() {
        std::env::remove_var("STEER_TELEGRAM_AUTO_ROUTE_AI_DIGEST");
        std::env::set_var(
            "STEER_AI_DIGEST_PROGRAM_WEBHOOK_URL",
            "http://127.0.0.1:5678/webhook/test/programtrigger/ai-digest-program",
        );
        let routed = TelegramBot::infer_n8n_digest_request(
            "최근 ai 트렌드 중요한거 5개 llm으로 요약해서 노션에 정리해줘",
        );
        assert!(routed.is_some());
        std::env::remove_var("STEER_AI_DIGEST_PROGRAM_WEBHOOK_URL");
    }

    #[test]
    fn build_run_report_includes_result_links_section() {
        let outcome = crate::controller::planner::RunGoalOutcome {
            run_id: "surf_test_1".to_string(),
            planner_complete: true,
            execution_complete: true,
            business_complete: true,
            status: "business_completed".to_string(),
            summary: Some("ok".to_string()),
        };
        let ctx = super::RunResultContext {
            links: vec!["https://www.notion.so/abcd1234".to_string()],
            highlights: vec!["제목 A — 요약 A (https://example.com/a)".to_string()],
        };
        let report = TelegramBot::build_run_report(
            &outcome,
            &[],
            &[],
            "최근 ai 트렌드 중요한거 5개 llm으로 요약해서 노션에 정리해줘",
            &ctx,
        );
        assert!(report.contains("노션 링크:"));
        assert!(report.contains("https://www.notion.so/abcd1234"));
        assert!(report.contains("핵심 요약:"));
    }
}
