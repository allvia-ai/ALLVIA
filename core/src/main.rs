use local_os_agent::{
    analyzer, api_server, applescript, bash_executor, db, dependency_check, feedback_collector,
    integrations, llm_gateway, mcp_client, monitor, orchestrator, pattern_detector, policy,
    recommendation, recommendation_executor, scheduler, security, workflow_intake,
};

use local_os_agent::env_flag;
use local_os_agent::singleton_lock;

#[cfg(target_os = "macos")]
use local_os_agent::macos;

use chrono::Utc;
use local_os_agent::schema::{AgentAction, EventEnvelope};
use serde_json::json;
use std::collections::HashMap;
use tokio::io::{self, AsyncBufReadExt, AsyncWriteExt};
use tracing::{error, info, warn};
use uuid::Uuid;

fn summarize_prompt(prompt: &str, max_chars: usize) -> String {
    let trimmed = prompt.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }
    let short = trimmed.chars().take(max_chars).collect::<String>();
    format!("{}...", short)
}

#[derive(Debug, Clone)]
struct DigestEmail {
    id: String,
    from: String,
    subject: String,
    date: String,
    snippet: String,
}

#[derive(Debug, Clone)]
struct DigestSummary {
    overall_summary: String,
    per_email_lines: Vec<String>,
}

fn extract_email_header(raw: &str, header: &str) -> Option<String> {
    let prefix = format!("{}:", header);
    raw.lines()
        .find_map(|line| line.strip_prefix(&prefix).map(|v| v.trim().to_string()))
}

fn extract_email_snippet(raw: &str) -> String {
    let snippet = raw
        .split("\n---\n\n")
        .nth(1)
        .unwrap_or("")
        .replace('\n', " ")
        .trim()
        .to_string();
    summarize_prompt(&snippet, 220)
}

fn fallback_digest_summary(emails: &[DigestEmail]) -> DigestSummary {
    let per_email_lines = emails
        .iter()
        .enumerate()
        .map(|(idx, email)| {
            format!(
                "{}. [MEDIUM] {} | {}",
                idx + 1,
                summarize_prompt(&email.subject, 72),
                summarize_prompt(&email.snippet, 120)
            )
        })
        .collect::<Vec<_>>();

    let overall_summary = format!(
        "최근 메일 {}건을 수집했습니다. 중요도 분류가 불완전하므로 제목/발신자 기준으로 우선 확인하세요.",
        emails.len()
    );
    DigestSummary {
        overall_summary,
        per_email_lines,
    }
}

fn parse_retry_after_seconds(err: &str) -> Option<u64> {
    let lower = err.to_lowercase();
    let marker = "try again in ";
    let start = lower.find(marker)? + marker.len();
    let remain = &lower[start..];
    let mut numeric = String::new();
    for ch in remain.chars() {
        if ch.is_ascii_digit() || ch == '.' {
            numeric.push(ch);
            continue;
        }
        break;
    }
    if numeric.is_empty() {
        return None;
    }
    let secs = numeric.parse::<f64>().ok()?;
    Some(secs.ceil().max(1.0) as u64)
}

fn notion_text(text: &str, max_chars: usize) -> String {
    summarize_prompt(text, max_chars).replace('\n', " ")
}

fn notion_heading_block(text: &str, level: u8) -> serde_json::Value {
    let rich = json!([{
        "type": "text",
        "text": { "content": notion_text(text, 180) }
    }]);
    match level {
        1 => json!({
            "object": "block",
            "type": "heading_1",
            "heading_1": { "rich_text": rich }
        }),
        3 => json!({
            "object": "block",
            "type": "heading_3",
            "heading_3": { "rich_text": rich }
        }),
        _ => json!({
            "object": "block",
            "type": "heading_2",
            "heading_2": { "rich_text": rich }
        }),
    }
}

fn notion_paragraph_block(text: &str) -> serde_json::Value {
    json!({
        "object": "block",
        "type": "paragraph",
        "paragraph": {
            "rich_text": [{
                "type": "text",
                "text": { "content": notion_text(text, 1800) }
            }]
        }
    })
}

fn notion_bulleted_item_block(text: &str) -> serde_json::Value {
    json!({
        "object": "block",
        "type": "bulleted_list_item",
        "bulleted_list_item": {
            "rich_text": [{
                "type": "text",
                "text": { "content": notion_text(text, 1800) }
            }]
        }
    })
}

fn notion_numbered_item_block(text: &str) -> serde_json::Value {
    json!({
        "object": "block",
        "type": "numbered_list_item",
        "numbered_list_item": {
            "rich_text": [{
                "type": "text",
                "text": { "content": notion_text(text, 1800) }
            }]
        }
    })
}

fn notion_divider_block() -> serde_json::Value {
    json!({
        "object": "block",
        "type": "divider",
        "divider": {}
    })
}

fn build_gmail_digest_blocks(
    stamp: &str,
    emails: &[DigestEmail],
    digest: &DigestSummary,
) -> Vec<serde_json::Value> {
    let mut blocks = vec![
        notion_heading_block("Gmail Digest", 2),
        notion_paragraph_block(&format!("생성 시각: {}", stamp)),
        notion_paragraph_block(&format!("수집 건수: {}", emails.len())),
        notion_heading_block("전체 요약", 3),
        notion_paragraph_block(&digest.overall_summary),
        notion_heading_block("메일별 요약", 3),
    ];

    for line in &digest.per_email_lines {
        let normalized = line
            .split_once(". ")
            .map(|(_, rest)| rest)
            .unwrap_or(line.as_str());
        blocks.push(notion_numbered_item_block(normalized));
    }

    blocks.push(notion_heading_block("원문 인덱스", 3));
    for email in emails {
        let line = format!(
            "{} | {} | {}",
            summarize_prompt(&email.from, 80),
            summarize_prompt(&email.subject, 100),
            summarize_prompt(&email.date, 80)
        );
        blocks.push(notion_bulleted_item_block(&line));
    }

    blocks
}

async fn summarize_digest_with_llm(
    llm: &dyn llm_gateway::LLMClient,
    emails: &[DigestEmail],
) -> anyhow::Result<DigestSummary> {
    let payload = emails
        .iter()
        .map(|email| {
            json!({
                "id": email.id,
                "from": summarize_prompt(&email.from, 80),
                "subject": summarize_prompt(&email.subject, 120),
                "date": summarize_prompt(&email.date, 80),
                "snippet": summarize_prompt(&email.snippet, 120),
            })
        })
        .collect::<Vec<_>>();

    let user_prompt = format!(
        "다음 이메일 목록을 한국어로 요약하세요.\n\
출력은 반드시 JSON만 반환하세요.\n\
스키마:\n\
{{\n\
  \"overall_summary\": \"문장 2~3개\",\n\
  \"items\": [\n\
    {{\"id\":\"원본 id\", \"summary\":\"핵심 요약 1문장\", \"importance\":\"high|medium|low\", \"action_item\":\"필요한 후속조치 1문장\"}}\n\
  ]\n\
}}\n\
규칙:\n\
- items 길이는 입력과 동일\n\
- importance는 high|medium|low 중 하나\n\
- summary/action_item은 80자 이내\n\
\n입력:\n{}",
        serde_json::to_string(&payload)?
    );

    let messages = vec![
        json!({
            "role": "system",
            "content": "You are a strict JSON generator. Never output markdown or extra text."
        }),
        json!({
            "role": "user",
            "content": user_prompt
        }),
    ];

    let mut raw = String::new();
    let max_attempts = 2usize;
    for attempt in 0..max_attempts {
        match llm.chat_completion(messages.clone()).await {
            Ok(v) => {
                raw = v;
                break;
            }
            Err(e) => {
                let msg = e.to_string();
                let rate_limited = msg.to_lowercase().contains("rate_limit") || msg.contains("429");
                if !rate_limited || attempt + 1 >= max_attempts {
                    return Err(anyhow::anyhow!(msg));
                }
                let wait_sec = parse_retry_after_seconds(&msg).unwrap_or(5).min(45);
                warn!(
                    "LLM digest rate-limited; retrying in {}s (attempt {}/{})",
                    wait_sec,
                    attempt + 1,
                    max_attempts
                );
                tokio::time::sleep(std::time::Duration::from_secs(wait_sec)).await;
            }
        }
    }
    if raw.is_empty() {
        return Err(anyhow::anyhow!("LLM digest returned empty response"));
    }

    let parsed = llm_gateway::recover_json(&raw)
        .ok_or_else(|| anyhow::anyhow!("LLM digest output is not valid JSON"))?;

    let mut by_id: HashMap<String, (String, String, String)> = HashMap::new();
    if let Some(items) = parsed.get("items").and_then(|v| v.as_array()) {
        for item in items {
            let id = item
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if id.is_empty() {
                continue;
            }
            let summary = item
                .get("summary")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            let importance = item
                .get("importance")
                .and_then(|v| v.as_str())
                .unwrap_or("medium")
                .trim()
                .to_lowercase();
            let action_item = item
                .get("action_item")
                .and_then(|v| v.as_str())
                .unwrap_or("없음")
                .trim()
                .to_string();
            by_id.insert(id, (summary, importance, action_item));
        }
    }

    let mut per_email_lines = Vec::new();
    for (idx, email) in emails.iter().enumerate() {
        let (summary, importance, action_item) =
            by_id.get(&email.id).cloned().unwrap_or_else(|| {
                (
                    summarize_prompt(&email.snippet, 80),
                    "medium".to_string(),
                    "추가 확인 필요".to_string(),
                )
            });
        let importance_tag = match importance.as_str() {
            "high" => "HIGH",
            "low" => "LOW",
            _ => "MEDIUM",
        };
        per_email_lines.push(format!(
            "{}. [{}] {} | {} | 조치: {}",
            idx + 1,
            importance_tag,
            summarize_prompt(&email.subject, 72),
            summarize_prompt(&summary, 100),
            summarize_prompt(&action_item, 72)
        ));
    }

    let overall_summary = if let Some(text) = parsed.get("overall_summary").and_then(|v| v.as_str())
    {
        text.trim().to_string()
    } else if let Some(arr) = parsed.get("overall_summary").and_then(|v| v.as_array()) {
        arr.iter()
            .filter_map(|v| v.as_str())
            .collect::<Vec<_>>()
            .join(" ")
            .trim()
            .to_string()
    } else {
        String::new()
    };

    if overall_summary.is_empty() {
        return Err(anyhow::anyhow!("LLM digest JSON missing overall_summary"));
    }

    Ok(DigestSummary {
        overall_summary,
        per_email_lines,
    })
}

async fn run_gmail_digest_pipeline(
    count: u32,
    llm: Option<&dyn llm_gateway::LLMClient>,
) -> anyhow::Result<()> {
    let count = count.clamp(1, 10);
    let gmail = integrations::gmail::GmailClient::new().await?;
    let listed = gmail.list_messages(count).await?;
    if listed.is_empty() {
        return Err(anyhow::anyhow!("gmail inbox is empty"));
    }

    let mut emails = Vec::new();
    for (id, subject, from) in listed.into_iter().take(count as usize) {
        let raw = gmail.get_message(&id).await.unwrap_or_default();
        let date = extract_email_header(&raw, "Date").unwrap_or_else(|| "(unknown)".to_string());
        emails.push(DigestEmail {
            id,
            from,
            subject,
            date,
            snippet: extract_email_snippet(&raw),
        });
    }

    let digest = if let Some(client) = llm {
        match summarize_digest_with_llm(client, &emails).await {
            Ok(v) => v,
            Err(e) => {
                warn!("LLM digest failed; using fallback summarizer: {}", e);
                fallback_digest_summary(&emails)
            }
        }
    } else {
        fallback_digest_summary(&emails)
    };

    let now = Utc::now();
    let stamp = now.format("%Y-%m-%d %H:%M:%S UTC").to_string();
    let notion_blocks = build_gmail_digest_blocks(&stamp, &emails, &digest);

    let notion_client = integrations::notion::NotionClient::from_env()?;
    let page_id = std::env::var("NOTION_PAGE_ID").unwrap_or_default();
    let database_id = std::env::var("NOTION_DATABASE_ID").unwrap_or_default();
    let append_to_page = env_flag("NOTION_APPEND_TO_PAGE");
    let notion_url;

    if !database_id.trim().is_empty() {
        let title = format!("Gmail Digest {}", now.format("%Y%m%d_%H%M%S"));
        let created_page_id = notion_client
            .create_database_page_with_children(&database_id, &title, &notion_blocks)
            .await?;
        notion_url = format!("https://www.notion.so/{}", created_page_id.replace('-', ""));
    } else if !page_id.trim().is_empty() {
        if append_to_page {
            let mut append_blocks = vec![notion_divider_block()];
            append_blocks.extend(notion_blocks);
            notion_client
                .append_blocks(&page_id, &append_blocks)
                .await?;
            notion_url = format!("https://www.notion.so/{}", page_id.replace('-', ""));
        } else {
            let title = format!("Gmail Digest {}", now.format("%Y%m%d_%H%M%S"));
            let created_page_id = notion_client
                .create_child_page_with_children(&page_id, &title, &notion_blocks)
                .await?;
            notion_url = format!("https://www.notion.so/{}", created_page_id.replace('-', ""));
        }
    } else {
        return Err(anyhow::anyhow!(
            "NOTION_PAGE_ID or NOTION_DATABASE_ID must be set"
        ));
    }

    let mut telegram_message = format!(
        "Gmail 요약 완료\n시간: {}\n건수: {}\n전체: {}\n",
        stamp,
        emails.len(),
        digest.overall_summary
    );
    for line in digest.per_email_lines.iter().take(5) {
        telegram_message.push_str(line);
        telegram_message.push('\n');
    }
    telegram_message.push_str(&format!("Notion: {}", notion_url));
    telegram_message = summarize_prompt(&telegram_message, 3400);

    let bot = integrations::telegram::TelegramBot::from_env()?;
    bot.send_plain(&telegram_message).await?;

    println!("✅ Gmail digest pipeline completed.");
    println!("   - emails: {}", emails.len());
    println!("   - notion: {}", notion_url);
    println!("   - telegram: sent");

    Ok(())
}

fn load_mock_workflow_proposal() -> anyhow::Result<recommendation::AutomationProposal> {
    let path = std::env::var("STEER_WORKFLOW_MOCK_FILE")
        .unwrap_or_else(|_| "core/mock/workflow_received_mock.json".to_string());
    let raw = std::fs::read_to_string(&path)?;
    let proposal = serde_json::from_str::<recommendation::AutomationProposal>(&raw)?;
    if proposal.n8n_prompt.trim().is_empty() {
        return Err(anyhow::anyhow!(
            "mock workflow has empty n8n_prompt: {}",
            path
        ));
    }
    Ok(proposal)
}

async fn ingest_mock_workflow_recommendation(
    llm_client: Option<std::sync::Arc<dyn llm_gateway::LLMClient>>,
) -> anyhow::Result<()> {
    let proposal = load_mock_workflow_proposal()?;
    let (rec_id, inserted) = workflow_intake::insert_or_get_recommendation_id(&proposal)?;

    if inserted {
        println!(
            "📥 Mock workflow ingested as pending recommendation [{}] {}",
            rec_id, proposal.title
        );
    } else {
        println!(
            "📥 Mock workflow already exists; reusing recommendation [{}] {}",
            rec_id, proposal.title
        );
    }

    if env_flag("STEER_TEST_ASSUME_APPROVED") {
        recommendation_executor::maybe_assume_approved_for_test(rec_id)?;
        match recommendation_executor::approve_and_execute_recommendation(rec_id, llm_client).await
        {
            Ok(outcome) => println!(
                "✅ [TEST] approve-assumed pipeline completed. Workflow ID: {} (reused={})",
                outcome.workflow_id, outcome.reused_existing
            ),
            Err(e) => println!("❌ [TEST] approve-assumed pipeline failed: {}", e),
        }
    }

    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize Tracing
    tracing_subscriber::fmt::init();
    local_os_agent::load_env_with_fallback();

    // [Self-Healing] Panic Hook
    if !env_flag("STEER_PANIC_STD") {
        std::panic::set_hook(Box::new(|info| {
            let backtrace = std::backtrace::Backtrace::capture();
            let timestamp = chrono::Utc::now().to_rfc3339();

            let msg = if let Some(s) = info.payload().downcast_ref::<&str>() {
                *s
            } else if let Some(s) = info.payload().downcast_ref::<String>() {
                &**s
            } else {
                "Unknown panic"
            };

            let location = info
                .location()
                .map(|l| format!("{}:{}", l.file(), l.line()))
                .unwrap_or_else(|| "unknown".to_string());

            let log_entry = format!(
                "[{}] CRASH REPORT\nMessage: {}\nLocation: {}\nBacktrace:\n{:#?}\n--------------------------------------------------\n",
                timestamp, msg, location, backtrace
            );

            // Ensure log directory exists
            let home = std::env::var("HOME").unwrap_or("/".to_string());
            let log_dir = std::path::Path::new(&home).join(".steer/logs");
            if let Ok(_) = std::fs::create_dir_all(&log_dir) {
                let log_file = log_dir.join("crash.log");
                if let Ok(mut file) = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(log_file)
                {
                    use std::io::Write;
                    let _ = writeln!(file, "{}", log_entry);
                }
            }

            eprintln!("❌ FATAL ERROR: {}", msg);
            eprintln!("📄 Crash report saved to ~/.steer/logs/crash.log");
        }));
    } else {
        eprintln!("⚠️  Panic hook disabled (STEER_PANIC_STD=1).");
    }

    // Fast-path: keep `rewrite` output clean (no startup banners/log noise).
    let args: Vec<String> = std::env::args().collect();
    if args.len() >= 3 && args[1] == "rewrite" {
        let message = args[2..].join(" ");
        let refined = match llm_gateway::OpenAILLMClient::new() {
            Ok(c) => {
                let llm = std::sync::Arc::new(c) as std::sync::Arc<dyn llm_gateway::LLMClient>;
                if let Some(bot) = local_os_agent::telegram::TelegramBot::from_env(llm, None) {
                    bot.improve_message(&message).await
                } else {
                    message.clone()
                }
            }
            Err(_) => message.clone(),
        };
        println!("{}", refined);
        return Ok(());
    }

    let _lock = match singleton_lock::acquire_lock() {
        Ok(guard) => guard,
        Err(err) => {
            eprintln!("⛔️ {}", err);
            return Ok(());
        }
    };

    println!("🤖 Local OS Agent (Rust Native Mode) Started!");
    // [Phase 4] Self-Diagnosis: Check Accessibility Permissions
    println!("🔍 Checking Accessibility Permissions...");
    let ax_check = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        tokio::task::spawn_blocking(|| {
            std::process::Command::new("osascript")
                .arg("-e")
                .arg("tell application \"System Events\" to return name of first application process")
                .output()
        }),
    )
    .await;

    match ax_check {
        Ok(Ok(Ok(output))) if output.status.success() => {
            println!("✅ Accessibility Permissions: GRANTED.");
        }
        _ => {
            println!("\n\n################################################################");
            println!("❌ WARNING: ACCESSIBILITY PERMISSIONS MISSING OR REVOKED!");
            println!("   The agent can launch apps but CANNOT click or type.");
            println!("   FIX: Go to System Settings -> Privacy -> Accessibility");
            println!("   ACTION: Remove (-) and Re-add (+) your Terminal / Agent.");
            println!("################################################################\n\n");
            // We continue, but warn heavily.
        }
    }

    println!("--------------------------------------------------");

    // 0. System Health Check
    let health = dependency_check::SystemHealth::check_all();
    health.print_report();

    println!("Type 'help' for commands. (Needs Accessibility Permissions)");
    println!("--------------------------------------------------");

    // 0. Init Check
    if let Err(e) = db::init() {
        eprintln!("Failed to init DB: {}", e);
    }

    // 1. Init LLM
    let llm_client: Option<std::sync::Arc<dyn llm_gateway::LLMClient>> =
        match llm_gateway::OpenAILLMClient::new() {
            Ok(c) => Some(std::sync::Arc::new(c)),
            Err(e) => {
                warn!("⚠️ Failed to init LLM Gateway: {}", e);
                None
            }
        };

    // Fast-path CLI commands: run before background services (API/EventTap/Watchers)
    // so they are not blocked by API port conflicts.
    if args.len() >= 3 && args[1] == "notify" {
        let message = args[2..].join(" ");
        println!("🔔 Sending Smart Notification: {}", message);
        if let Some(llm) = llm_client.clone() {
            if let Some(bot) = local_os_agent::telegram::TelegramBot::from_env(llm, None) {
                let chat_id_str =
                    std::env::var("TELEGRAM_CHAT_ID").unwrap_or_else(|_| "0".to_string());
                if let Ok(chat_id) = chat_id_str.parse::<i64>() {
                    if let Err(e) = bot.send_smart_notification(chat_id, &message).await {
                        eprintln!("❌ Failed to send notification: {}", e);
                    } else {
                        println!("✅ Notification sent successfully (Smart Mode).");
                    }
                } else {
                    eprintln!("❌ TELEGRAM_CHAT_ID not set or invalid.");
                }
            } else {
                eprintln!("❌ Telegram Bot configuration missing (TELEGRAM_BOT_TOKEN).");
            }
        } else {
            eprintln!("❌ LLM Client not available for smart notification.");
        }
        return Ok(());
    }

    if args.len() >= 3 && args[1] == "surf" {
        let goal = args[2..].join(" ");
        println!("🎯 [CLI] Direct surf mode: {}", goal);
        if let Some(llm) = llm_client.clone() {
            let mut cli_policy = policy::PolicyEngine::new();
            cli_policy.unlock();
            let planner = local_os_agent::controller::planner::Planner::new(llm, None);
            match planner.run_goal_tracked(&goal, None).await {
                Ok(outcome) => println!(
                    "✅ Surf completed successfully! run_id={} planner={} execution={} business={}",
                    outcome.run_id,
                    outcome.planner_complete,
                    outcome.execution_complete,
                    outcome.business_complete
                ),
                Err(e) => println!("❌ Surf failed: {}", e),
            }
        } else {
            println!("❌ LLM not available for surf mode");
        }
        return Ok(());
    }

    if args.len() >= 2 && (args[1] == "telegram_listen" || args[1] == "telegram-listen") {
        if let Some(llm) = llm_client.clone() {
            if let Some(bot) = local_os_agent::telegram::TelegramBot::from_env(llm, None) {
                println!("🤖 Telegram listener mode started. Waiting for commands...");
                std::sync::Arc::new(bot).start_polling().await;
            } else {
                eprintln!(
                    "❌ Telegram listener requires TELEGRAM_BOT_TOKEN (optional: TELEGRAM_USER_ID)."
                );
            }
        } else {
            eprintln!("❌ LLM not available for Telegram listener.");
        }
        return Ok(());
    }

    // 2. Start Scheduler (Brain)
    if let Some(llm) = &llm_client {
        let scheduler = scheduler::Scheduler::new(llm.clone());
        scheduler.start();
        info!("🧠 Brain Routine Scheduler Active.");
    }

    // 2.5 Init MCP (non-blocking guard)
    // MCP server handshakes can stall in headless launchd sessions.
    // Bound initialization time so API startup is never blocked.
    match tokio::time::timeout(
        std::time::Duration::from_secs(8),
        tokio::task::spawn_blocking(mcp_client::init_mcp),
    )
    .await
    {
        Ok(Ok(Ok(()))) => info!("🔌 MCP System Initialized."),
        Ok(Ok(Err(e))) => warn!("⚠️ Failed to init MCP: {}", e),
        Ok(Err(e)) => warn!("⚠️ MCP init join error: {}", e),
        Err(_) => warn!("⚠️ MCP init timeout (continuing without blocking startup)"),
    }

    // 1. Start Native Event Tap (replaces IPC Adapter)
    // [Paranoid Audit] Increased capacity to 1000 to prevent dropping mouse bursts
    let (log_tx, mut log_rx) = tokio::sync::mpsc::channel::<String>(1000);

    #[cfg(target_os = "macos")]
    {
        if env_flag("STEER_DISABLE_EVENT_TAP") {
            info!("⚠️  Event Tap disabled via STEER_DISABLE_EVENT_TAP.");
        } else if let Err(e) = macos::events::start_event_tap(log_tx.clone()) {
            error!("❌ Failed to start Event Tap: {}", e);
        }
    }

    // 2. Start "Shadow Analyzer" (Decoupled Module)
    // CRITICAL FIX: Always consume log_rx, even without LLM
    if let Some(c) = llm_client.clone() {
        analyzer::spawn(log_rx, c);
    } else {
        // Fallback: Just save events to DB without LLM analysis
        tokio::spawn(async move {
            while let Some(log_json) = log_rx.recv().await {
                if let Err(e) = db::insert_event(&log_json) {
                    eprintln!("DB insert error: {}", e);
                }
            }
        });
        println!("⚠️  Running in lite mode (no LLM, events still saved)");
    }

    // 4. Start HTTP API Server for Desktop GUI
    println!("🌐 Starting Desktop API Server...");
    let llm_for_api = llm_client.clone();
    tokio::spawn(async move {
        if let Err(e) = api_server::start_api_server(llm_for_api).await {
            eprintln!("⚠️  Desktop API Server failed to start: {}", e);
            eprintln!("   (Continuing without API server)");
        }
    });

    // 5. Start File Watcher (Downloads or override path)
    if env_flag("STEER_DISABLE_DOWNLOAD_WATCHER") {
        println!("ℹ️  Downloads watcher disabled via STEER_DISABLE_DOWNLOAD_WATCHER.");
    } else {
        let home = std::env::var("HOME").unwrap_or("/".to_string());
        let downloads = std::env::var("STEER_DOWNLOADS_DIR")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| format!("{}/Downloads", home));

        // Reuse log_tx to send file events to Analyzer.
        if let Err(e) = monitor::spawn_file_watcher(downloads.clone(), log_tx.clone()) {
            println!("⚠️  Failed to watch {}: {}", downloads, e);
        } else {
            println!("👀 Watching for changes in {}", downloads);
        }
    }

    // 6. Start App Watcher (Active Window Poller)
    if env_flag("STEER_DISABLE_APP_WATCHER") {
        println!("ℹ️  App watcher disabled via STEER_DISABLE_APP_WATCHER.");
    } else {
        monitor::spawn_app_watcher(log_tx.clone());
        println!("👀 Watching for active application changes...");
    }

    let mut telegram_listener_started = false;
    if env_flag("STEER_TELEGRAM_POLLING") {
        if let Some(llm) = llm_client.clone() {
            if let Some(bot) =
                local_os_agent::telegram::TelegramBot::from_env(llm, Some(log_tx.clone()))
            {
                println!("🤖 Telegram polling enabled (STEER_TELEGRAM_POLLING=1).");
                tokio::spawn(async move {
                    std::sync::Arc::new(bot).start_polling().await;
                });
                telegram_listener_started = true;
            } else {
                println!(
                    "⚠️  STEER_TELEGRAM_POLLING=1 but TELEGRAM_BOT_TOKEN is missing; listener not started."
                );
            }
        } else {
            println!("⚠️  STEER_TELEGRAM_POLLING=1 but LLM is unavailable; listener not started.");
        }
    }

    let mut policy = policy::PolicyEngine::new(); // Starts LOCKED
    let mut res_mon = monitor::ResourceMonitor::new();

    // 5. User Input Loop (REPL)
    let stdin = io::stdin();
    let mut reader = io::BufReader::new(stdin);
    let mut buffer = String::new();

    loop {
        buffer.clear();
        print!("> ");
        if let Err(e) = io::stdout().flush().await {
            eprintln!("⚠️ Flush failed: {}", e);
        }

        if reader.read_line(&mut buffer).await? == 0 {
            // EOF - keep server running (headless mode)
            println!("📡 Running in headless mode (API only)...");
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
            }
        }

        let input = buffer.trim();
        if input.is_empty() {
            continue;
        }

        let parts: Vec<&str> = input.split_whitespace().collect();
        match parts[0] {
            "help" => {
                println!("Commands:");
                println!("  snap [scope]          - Take UI snapshot");
                println!("  click <id>            - Click element by ID");
                println!("  type <text>           - Type text");
                println!("  unlock                - Unlock Write Policy");
                println!("  status                - Show system status");
                println!("  recommendations [N]   - List pending workflow recommendations");
                println!("  approve <id>          - Approve and create n8n workflow");
                println!(
                    "  approve_test <id>     - Test-only assumed approval then create workflow"
                );
                println!("  reject <id>           - Reject recommendation");
                println!(
                    "  ingest_mock_workflow  - Ingest mock workflow as pending recommendation"
                );
                println!(
                    "  ingest_handoff [cfg]  - Consume collector pending handoff into recommendation"
                );
                println!("  analyze_patterns      - Detect behavior patterns and generate recommendations");
                println!("  quality               - Show workflow quality metrics");
                println!("  telegram <msg>        - Send Telegram message");
                println!("  telegram_listen       - Start Telegram natural-language listener");
                println!("  notion <title>|<body> - Create Notion page");
                println!("  gmail list [N]        - List recent N emails");
                println!("  gmail read <id>       - Read email by ID");
                println!("  gmail send <to>|<subj>|<body> - Send email");
                println!("  gmail digest [N]      - Summarize N emails -> Notion -> Telegram");
                println!("  calendar today        - Today's events");
                println!("  calendar week         - This week's events");
                println!("  calendar add <title>|<start>|<end> - Add event");
                println!("  exit                  - Quit");
            }
            "exit" | "quit" => break,
            "unlock" => {
                policy.unlock();
                println!("[Policy] Write Lock UNLOCKED.");
            }
            "lock" => {
                policy.lock();
                println!("[Policy] Write Lock LOCKED.");
            }
            "snap" => {
                let scope = if parts.len() > 1 {
                    Some(parts[1].to_string())
                } else {
                    None
                };
                println!("[MacOS] Snapshotting...");
                #[cfg(target_os = "macos")]
                {
                    let tree = macos::accessibility::snapshot(scope);
                    println!("📄 Snapshot:\n{}", serde_json::to_string_pretty(&tree)?);
                }
            }
            "type" => {
                if parts.len() < 2 {
                    println!("Usage: type <text>");
                    continue;
                }
                let text = parts[1..].join(" ");
                // Policy Check
                match policy.check(&AgentAction::UiType { text: text.clone() }) {
                    Ok(_) => {
                        println!("✅ Policy Passed");
                        #[cfg(target_os = "macos")]
                        if let Err(e) = macos::actions::type_text(&text) {
                            println!("❌ Type failed: {}", e);
                        }
                    }
                    Err(e) => println!("⛔️ Policy Blocked: {}", e),
                }
            }
            "click" => {
                if parts.len() < 2 {
                    println!("Usage: click <id>");
                    continue;
                }
                let id = parts[1];
                match policy.check(&AgentAction::UiClick {
                    element_id: id.to_string(),
                    double_click: false,
                }) {
                    Ok(_) => {
                        println!("✅ Policy Passed");
                        #[cfg(target_os = "macos")]
                        if let Err(e) = macos::actions::click_element(id) {
                            println!("❌ Click failed: {}", e);
                        }
                    }
                    Err(e) => println!("⛔️ Policy Blocked: {}", e),
                }
            }
            "exec" => {
                if parts.len() < 2 {
                    println!("Usage: exec <command>");
                    continue;
                }
                let cmd = parts[1..].join(" ");

                // [Phase 8] Security Sandboxing
                match security::CommandClassifier::classify(&cmd) {
                    security::SafetyLevel::Critical => {
                        println!("⛔️ CRITICAL WARNING: This command is flagged as DANGEROUS.");
                        println!("   Command: {}", cmd);
                        println!("   To execute, type 'CONFIRM':");

                        buffer.clear();
                        if reader.read_line(&mut buffer).await? == 0 {
                            break;
                        }
                        if buffer.trim() != "CONFIRM" {
                            println!("❌ Aborted.");
                            continue;
                        }
                    }
                    security::SafetyLevel::Warning => {
                        println!("⚠️  WARNING: This command may modify your system.");
                        println!("   Command: {}", cmd);
                        println!("   Execute? (y/n):");

                        buffer.clear();
                        if reader.read_line(&mut buffer).await? == 0 {
                            break;
                        }
                        if buffer.trim().to_lowercase() != "y" {
                            println!("❌ Aborted.");
                            continue;
                        }
                    }
                    security::SafetyLevel::Safe => {
                        // Safe to proceed automatically
                    }
                }

                let cwd = std::env::current_dir()
                    .ok()
                    .map(|p| p.to_string_lossy().to_string());
                let action = AgentAction::ShellExecution {
                    command: cmd.clone(),
                };
                match policy.check_with_context(&action, cwd.as_deref()) {
                    Ok(_) => {
                        println!("⚙️  Executing: '{}'", cmd);
                        match bash_executor::exec(&cmd) {
                            Ok(out) => println!("Output:\n{}", out),
                            Err(e) => println!("❌ Exec failed: {}", e),
                        }
                    }
                    Err(e) => {
                        if let Ok(Some(_approval)) =
                            db::find_valid_exec_approval(&cmd, cwd.as_deref())
                        {
                            println!("✅ Approved command found. Executing: '{}'", cmd);
                            match bash_executor::exec(&cmd) {
                                Ok(out) => println!("Output:\n{}", out),
                                Err(e) => println!("❌ Exec failed: {}", e),
                            }
                        } else {
                            let approval =
                                db::create_exec_approval(&cmd, cwd.as_deref(), 3600).ok();
                            if let Some(approval) = approval {
                                println!("⛔️ Policy Blocked: {}", e);
                                println!("📝 Exec approval requested: {}", approval.id);
                                println!(
                                    "   Approve once: POST /api/exec-approvals/{}/approve",
                                    approval.id
                                );
                                println!("   Approve always: POST /api/exec-approvals/{}/approve ({{\"decision\":\"allow-always\"}})", approval.id);
                            } else {
                                println!("⛔️ Policy Blocked: {}", e);
                            }
                        }
                    }
                }
            }
            "open" => {
                if parts.len() < 2 {
                    println!("Usage: open <url>");
                    continue;
                }
                let url = parts[1];
                println!("🌐 Opening URL: {}", url);
                if let Err(e) = crate::applescript::open_url(url).map(|_| ()) {
                    println!("❌ Open failed: {}", e);
                }
            }
            "fake_log" => {
                // Simulate log
                #[cfg(target_os = "macos")]
                {
                    let event = EventEnvelope {
                        schema_version: "1.0".to_string(),
                        event_id: Uuid::new_v4().to_string(),
                        ts: Utc::now().to_rfc3339(),
                        source: "debug".to_string(),
                        app: "FakeApp".to_string(),
                        event_type: "simulated".to_string(),
                        priority: "P2".to_string(),
                        resource: None,
                        payload: json!({"note": "simulated"}),
                        privacy: None,
                        pid: None,
                        window_id: None,
                        window_title: None,
                        browser_url: None,
                        raw: None,
                    };
                    if let Ok(log) = serde_json::to_string(&event) {
                        let _ = log_tx.send(log).await;
                    }
                    println!("✅ Simulated Log Sent");
                }
            }
            "routine" => {
                if let Some(brain) = &llm_client {
                    println!("🧠 Analyzing daily routine (last 24h)...");
                    match db::get_recent_events(24) {
                        Ok(logs) => {
                            if logs.is_empty() {
                                println!("   (No events found in DB to analyze)");
                            } else {
                                println!("   Found {} events. Asking LLM...", logs.len());
                                match brain.analyze_routine(&logs).await {
                                    Ok(summary) => {
                                        println!("\n📊 Routine Analysis:\n{}", summary);
                                    }
                                    Err(e) => println!("❌ Analysis failed: {}", e),
                                }
                            }
                        }
                        Err(e) => println!("❌ DB Query failed: {}", e),
                    }
                } else {
                    println!("⚠️  LLM Client not available.");
                }
            }
            "recommend" => {
                if let Some(brain) = &llm_client {
                    println!("🤖 Generating automation recommendation...");
                    match db::get_recent_events(24) {
                        Ok(logs) => {
                            if logs.is_empty() {
                                println!("   (No events found in DB)");
                            } else {
                                match brain.recommend_automation(&logs).await {
                                    Ok(script) => {
                                        println!("\n✨ Recommendation:\n{}", script);
                                        println!("\n💡 Tip: Save code to a file and run with 'exec <file>'");
                                    }
                                    Err(e) => println!("❌ Recommendation failed: {}", e),
                                }
                            }
                        }
                        Err(e) => println!("❌ DB Query failed: {}", e),
                    }
                } else {
                    println!("⚠️  LLM Client not available.");
                }
            }
            "analyze_patterns" | "detect" => {
                println!("🔍 Analyzing behavior patterns...");
                let detector = pattern_detector::PatternDetector::new();
                let patterns = detector.analyze();

                if patterns.is_empty() {
                    println!("   (No significant patterns detected yet)");
                    println!("   Keep using your computer - patterns will be detected over time.");
                } else {
                    println!("   Found {} patterns:", patterns.len());
                    for pattern in &patterns {
                        println!(
                            "   📊 {} ({} occurrences, {:.0}% similarity)",
                            pattern.description,
                            pattern.occurrences,
                            pattern.similarity_score * 100.0
                        );
                    }

                    // Generate recommendations if LLM available
                    if let Some(brain) = &llm_client {
                        println!("\n🤖 Generating workflow recommendations...");
                        for pattern in patterns {
                            if pattern.occurrences >= 3 && pattern.similarity_score >= 0.8 {
                                match brain
                                    .generate_recommendation_from_pattern(
                                        &pattern.description,
                                        &pattern.sample_events,
                                    )
                                    .await
                                {
                                    Ok(mut proposal) => {
                                        // [Explainability] Inject hard evidence manually
                                        proposal
                                            .evidence
                                            .push(format!("Pattern: {}", pattern.description));
                                        proposal.evidence.push(format!(
                                            "Frequency: {} occurrences in last 7 days",
                                            pattern.occurrences
                                        ));

                                        if proposal.confidence >= 0.7 {
                                            if let Ok(true) = db::insert_recommendation(&proposal) {
                                                println!("   ✨ New recommendation: {} (confidence: {:.0}%)", 
                                                    proposal.title, proposal.confidence * 100.0);
                                            }
                                        }
                                    }
                                    Err(e) => println!("   ⚠️  Skipped pattern: {}", e),
                                }
                            }
                        }
                        println!("\nRun 'recommendations' to see pending recommendations.");
                    }
                }
            }
            "quality" | "metrics" => {
                let collector = feedback_collector::FeedbackCollector::new();
                let metrics = collector.get_quality_metrics();
                println!("📈 Workflow Quality Metrics:");
                println!("   {}", metrics);
            }
            "status" => {
                println!("📊 System Status:");
                println!("   {}", res_mon.get_status());
                println!("   Top Apps:");
                for (name, usage) in res_mon.get_high_usage_apps() {
                    println!("   - {}: {:.1}%", name, usage);
                }
            }
            "recommendations" | "recs" => {
                let limit = parts
                    .get(1)
                    .and_then(|s| s.parse::<i64>().ok())
                    .unwrap_or(5);
                match db::list_recommendations("pending", limit) {
                    Ok(recs) => {
                        if recs.is_empty() {
                            println!("(No pending recommendations)");
                        } else {
                            println!("🧩 Pending recommendations:");
                            for rec in recs {
                                println!(
                                    "  [{}] {} (confidence {:.2})",
                                    rec.id, rec.title, rec.confidence
                                );
                                println!("       Trigger: {}", rec.trigger);
                                println!("       Summary: {}", rec.summary);
                            }
                        }
                    }
                    Err(e) => println!("❌ Failed to load recommendations: {}", e),
                }
            }
            "approve" => {
                if parts.len() < 2 {
                    println!("Usage: approve <id>");
                    continue;
                }
                let id: i64 = match parts[1].parse() {
                    Ok(v) => v,
                    Err(_) => {
                        println!("Usage: approve <id>");
                        continue;
                    }
                };
                println!("🏗️  Running approval pipeline for recommendation {}...", id);
                match recommendation_executor::approve_and_execute_recommendation(
                    id,
                    llm_client.clone(),
                )
                .await
                {
                    Ok(outcome) => {
                        if outcome.reused_existing {
                            println!(
                                "♻️  Workflow reused (already provisioned). ID: {}",
                                outcome.workflow_id
                            );
                        } else {
                            println!("✅ Workflow created! ID: {}", outcome.workflow_id);
                        }
                        if outcome.approved_now {
                            println!("📝 Recommendation {} marked as approved.", id);
                        } else {
                            println!("ℹ️ Recommendation {} was already approved.", id);
                        }
                    }
                    Err(e) => {
                        println!("❌ Approval pipeline failed: {}", e);
                    }
                }
            }
            "approve_test" => {
                if parts.len() < 2 {
                    println!("Usage: approve_test <id>");
                    continue;
                }
                let id: i64 = match parts[1].parse() {
                    Ok(v) => v,
                    Err(_) => {
                        println!("Usage: approve_test <id>");
                        continue;
                    }
                };

                if let Err(e) = recommendation_executor::maybe_assume_approved_for_test(id) {
                    println!("❌ approve_test precheck failed: {}", e);
                    continue;
                }
                match recommendation_executor::approve_and_execute_recommendation(
                    id,
                    llm_client.clone(),
                )
                .await
                {
                    Ok(outcome) => {
                        if outcome.reused_existing {
                            println!(
                                "♻️  [TEST] Workflow reused (already provisioned). ID: {}",
                                outcome.workflow_id
                            );
                        } else {
                            println!("✅ [TEST] Workflow created! ID: {}", outcome.workflow_id);
                        }
                    }
                    Err(e) => {
                        println!("❌ [TEST] Approval pipeline failed: {}", e);
                    }
                }
            }
            "ingest_mock_workflow" => {
                if let Err(e) = ingest_mock_workflow_recommendation(llm_client.clone()).await {
                    println!("❌ Mock workflow ingest failed: {}", e);
                }
            }
            "ingest_handoff" => {
                let config_override = parts.get(1).copied();
                match workflow_intake::ingest_latest_collector_handoff(config_override) {
                    Ok(outcome) => {
                        println!(
                            "📥 Handoff ingest status={} detail={}",
                            outcome.status, outcome.detail
                        );
                        if let Some(pkg) = outcome.package_id {
                            println!("   package_id={}", pkg);
                        }
                        if let Some(id) = outcome.recommendation_id {
                            println!("   recommendation_id={} inserted={}", id, outcome.inserted);
                        }
                    }
                    Err(e) => println!("❌ Handoff ingest failed: {}", e),
                }
            }
            "reject" => {
                if parts.len() < 2 {
                    println!("Usage: reject <id>");
                    continue;
                }
                let id: i64 = match parts[1].parse() {
                    Ok(v) => v,
                    Err(_) => {
                        println!("Usage: reject <id>");
                        continue;
                    }
                };
                match db::update_recommendation_review_status(id, "rejected") {
                    Ok(()) => println!("🗑️  Recommendation {} rejected.", id),
                    Err(e) => println!("❌ Failed to reject recommendation: {}", e),
                }
            }
            "control" => {
                if parts.len() < 3 {
                    println!("Usage: control <app> <action> (e.g., control Music play)");
                    continue;
                }
                let app = parts[1];
                let command = parts[2];
                println!("🎮 Controlling {} with '{}'...", app, command);
                match applescript::control_app(app, command) {
                    Ok(out) => {
                        if !out.is_empty() {
                            println!("Output: {}", out);
                        }
                        println!("✅ Command sent.");
                    }
                    Err(e) => println!("❌ Control failed: {}", e),
                }
            }
            "build_workflow" => {
                if parts.len() < 2 {
                    println!("Usage: build_workflow <prompt>");
                    continue;
                }
                let prompt = parts[1..].join(" ");
                match workflow_intake::queue_manual_workflow_recommendation(
                    &prompt,
                    "cli.build_workflow",
                ) {
                    Ok(outcome) => {
                        let rec_id = outcome.recommendation_id;
                        let inserted = outcome.inserted;
                        if inserted {
                            println!("📝 Recommendation queued [{}] as pending approval.", rec_id);
                        } else {
                            println!(
                                "📝 Existing recommendation reused [{}] as pending/approved history.",
                                rec_id
                            );
                        }
                        println!(
                            "   Approval gate enforced: run `approve {}` to create in n8n.",
                            rec_id
                        );
                        println!("   Rejection path: run `reject {}`.", rec_id);
                    }
                    Err(e) => println!("❌ Failed to queue workflow recommendation: {}", e),
                }
            }
            "telegram" => {
                if parts.len() < 2 {
                    println!("Usage: telegram <message>");
                    continue;
                }
                let message = parts[1..].join(" ");
                println!("📱 Sending to Telegram...");
                match integrations::telegram::TelegramBot::from_env() {
                    Ok(bot) => match bot.send(&message).await {
                        Ok(_) => println!("✅ Message sent!"),
                        Err(e) => println!("❌ Failed: {}", e),
                    },
                    Err(e) => println!("⚠️  Telegram not configured: {}", e),
                }
            }
            "telegram_listen" | "telegram-listen" => {
                if telegram_listener_started {
                    println!("ℹ️  Telegram listener is already running.");
                    continue;
                }
                if let Some(llm) = llm_client.clone() {
                    if let Some(bot) =
                        local_os_agent::telegram::TelegramBot::from_env(llm, Some(log_tx.clone()))
                    {
                        println!("🤖 Telegram listener started.");
                        tokio::spawn(async move {
                            std::sync::Arc::new(bot).start_polling().await;
                        });
                        telegram_listener_started = true;
                    } else {
                        println!(
                            "⚠️  Telegram listener requires TELEGRAM_BOT_TOKEN (optional: TELEGRAM_USER_ID)."
                        );
                    }
                } else {
                    println!("⚠️  LLM Client not available.");
                }
            }
            "notion" => {
                // Usage: notion <title> | <content>
                if parts.len() < 2 {
                    println!("Usage: notion <title> | <content>");
                    continue;
                }
                let full_text = parts[1..].join(" ");
                let split: Vec<&str> = full_text.splitn(2, '|').collect();
                let title = split.first().unwrap_or(&"Untitled").trim();
                let content = split.get(1).unwrap_or(&"").trim();

                let db_id = std::env::var("NOTION_DATABASE_ID").unwrap_or_default();
                if db_id.is_empty() {
                    println!("⚠️  NOTION_DATABASE_ID not set in .env");
                    continue;
                }

                println!("📝 Creating Notion page: '{}'...", title);
                match integrations::notion::NotionClient::from_env() {
                    Ok(client) => match client.create_page(&db_id, title, content).await {
                        Ok(page_id) => println!("✅ Page created! ID: {}", page_id),
                        Err(e) => println!("❌ Failed: {}", e),
                    },
                    Err(e) => println!("⚠️  Notion not configured: {}", e),
                }
            }
            "gmail" => {
                if parts.len() < 2 {
                    println!(
                        "Usage: gmail list [N] | gmail read <id> | gmail send <to>|<subj>|<body> | gmail digest [N]"
                    );
                    continue;
                }
                match parts[1] {
                    "list" => {
                        let count = parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(5);
                        println!("📧 Fetching {} recent emails...", count);
                        match integrations::gmail::GmailClient::new().await {
                            Ok(client) => match client.list_messages(count).await {
                                Ok(messages) => {
                                    if messages.is_empty() {
                                        println!("   (No messages found)");
                                    } else {
                                        for (id, subject, from) in messages {
                                            println!(
                                                "  📩 [{}] {} — {}",
                                                &id[..8.min(id.len())],
                                                subject,
                                                from
                                            );
                                        }
                                    }
                                }
                                Err(e) => println!("❌ Failed: {}", e),
                            },
                            Err(e) => println!("⚠️  Gmail auth failed: {}", e),
                        }
                    }
                    "read" => {
                        if parts.len() < 3 {
                            println!("Usage: gmail read <id>");
                            continue;
                        }
                        let id = parts[2];
                        println!("📖 Reading email {}...", id);
                        match integrations::gmail::GmailClient::new().await {
                            Ok(client) => match client.get_message(id).await {
                                Ok(content) => println!("\n{}", content),
                                Err(e) => println!("❌ Failed: {}", e),
                            },
                            Err(e) => println!("⚠️  Gmail auth failed: {}", e),
                        }
                    }
                    "send" => {
                        let full_text = parts[2..].join(" ");
                        let split: Vec<&str> = full_text.splitn(3, '|').collect();
                        if split.len() < 3 {
                            println!("Usage: gmail send <to>|<subject>|<body>");
                            continue;
                        }
                        let to = split[0].trim();
                        let subject = split[1].trim();
                        let body = split[2].trim();

                        println!("✉️  Sending email to {}...", to);
                        match integrations::gmail::GmailClient::new().await {
                            Ok(client) => match client.send_message(to, subject, body).await {
                                Ok(id) => println!("✅ Email sent! ID: {}", id),
                                Err(e) => println!("❌ Failed: {}", e),
                            },
                            Err(e) => println!("⚠️  Gmail auth failed: {}", e),
                        }
                    }
                    "digest" | "digest5" => {
                        let count = parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(5);
                        println!("🧠 Running gmail digest pipeline ({} mails)...", count);
                        match run_gmail_digest_pipeline(count, llm_client.as_deref()).await {
                            Ok(()) => {}
                            Err(e) => println!("❌ Digest failed: {}", e),
                        }
                    }
                    _ => println!("Unknown gmail subcommand. Use: list, read, send, digest"),
                }
            }
            "calendar" => {
                if parts.len() < 2 {
                    println!("Usage: calendar today | week | add <title>|<start>|<end>");
                    continue;
                }
                match parts[1] {
                    "today" => {
                        println!("📅 Fetching today's events...");
                        match integrations::calendar::CalendarClient::new().await {
                            Ok(client) => match client.list_today().await {
                                Ok(events) => {
                                    if events.is_empty() {
                                        println!("   (No events today)");
                                    } else {
                                        for (_, summary, time) in events {
                                            println!("  🗓️  {} — {}", time, summary);
                                        }
                                    }
                                }
                                Err(e) => println!("❌ Failed: {}", e),
                            },
                            Err(e) => println!("⚠️  Calendar auth failed: {}", e),
                        }
                    }
                    "week" => {
                        println!("📅 Fetching this week's events...");
                        match integrations::calendar::CalendarClient::new().await {
                            Ok(client) => match client.list_week().await {
                                Ok(events) => {
                                    if events.is_empty() {
                                        println!("   (No events this week)");
                                    } else {
                                        for (_, summary, time) in events {
                                            println!("  🗓️  {} — {}", time, summary);
                                        }
                                    }
                                }
                                Err(e) => println!("❌ Failed: {}", e),
                            },
                            Err(e) => println!("⚠️  Calendar auth failed: {}", e),
                        }
                    }
                    "add" => {
                        let full_text = parts[2..].join(" ");
                        let split: Vec<&str> = full_text.splitn(3, '|').collect();
                        if split.len() < 3 {
                            println!("Usage: calendar add <title>|<start ISO>|<end ISO>");
                            println!("Example: calendar add Meeting|2026-01-25T14:00:00+09:00|2026-01-25T15:00:00+09:00");
                            continue;
                        }
                        let title = split[0].trim();
                        let start = split[1].trim();
                        let end = split[2].trim();

                        info!("➕ Adding event: '{}'...", title);
                        match integrations::calendar::CalendarClient::new().await {
                            Ok(client) => match client.create_event(title, start, end).await {
                                Ok(id) => info!("✅ Event created! ID: {}", id),
                                Err(e) => error!("❌ Failed: {}", e),
                            },
                            Err(e) => warn!("⚠️  Calendar auth failed: {}", e),
                        }
                    }
                    _ => warn!("Unknown calendar subcommand. Use: today, week, add"),
                }
            }
            "surf" => {
                if parts.len() < 2 {
                    warn!("Usage: surf <goal>");
                    continue;
                }
                let goal = parts[1..].join(" ");

                if let Some(brain) = &llm_client {
                    let planner =
                        local_os_agent::controller::planner::Planner::new(brain.clone(), None);
                    // Run concurrently to allow Ctrl+C? For now blocking is fine as it has internal timeout/loop
                    match planner.run_goal_tracked(&goal, None).await {
                        Ok(outcome) => info!(
                            "✅ Surf completed (run_id={}, planner={}, execution={}, business={})",
                            outcome.run_id,
                            outcome.planner_complete,
                            outcome.execution_complete,
                            outcome.business_complete
                        ),
                        Err(e) => error!("❌ Surf failed: {}", e),
                    }
                } else {
                    warn!("⚠️  LLM Client not available.");
                }
            }
            // Super Agent Mode (Unified Orchestrator)
            _ => {
                if let Ok(orch) = orchestrator::Orchestrator::new().await {
                    info!("🤖 Super Agent: Processing '{}'...", input);
                    match orch.handle_request(input).await {
                        Ok(resp) => info!("{}", resp),
                        Err(e) => error!("❌ Super Agent Error: {}", e),
                    }
                } else {
                    warn!("⚠️  Orchestrator could not initialization.");
                }
            }
        }
    }

    Ok(())
}
