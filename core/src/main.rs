use local_os_agent::{
    analyzer, api_server, applescript, bash_executor, db, dependency_check, feedback_collector,
    integrations, llm_gateway, mcp_client, monitor, orchestrator, pattern_detector, policy,
    recommendation, recommendation_executor, scheduler, security,
};

use local_os_agent::env_flag;
use local_os_agent::singleton_lock;

#[cfg(target_os = "macos")]
use local_os_agent::macos;

use chrono::Utc;
use local_os_agent::schema::{AgentAction, EventEnvelope};
use serde_json::json;
use tokio::io::{self, AsyncBufReadExt, AsyncWriteExt};
use tracing::{error, info, warn};
use uuid::Uuid;

fn recommendation_fingerprint(title: &str, trigger: &str) -> String {
    format!(
        "{}::{}",
        title.trim().to_lowercase(),
        trigger.trim().to_lowercase()
    )
}

fn find_recommendation_id_by_fingerprint(target: &str) -> anyhow::Result<Option<i64>> {
    let rows = db::get_recommendations_with_filter(Some("all"))?;
    for rec in rows {
        let fp = recommendation_fingerprint(&rec.title, &rec.trigger);
        if fp == target {
            return Ok(Some(rec.id));
        }
    }
    Ok(None)
}

fn summarize_prompt(prompt: &str, max_chars: usize) -> String {
    let trimmed = prompt.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }
    let short = trimmed.chars().take(max_chars).collect::<String>();
    format!("{}...", short)
}

fn queue_manual_workflow_recommendation(prompt: &str, source: &str) -> anyhow::Result<(i64, bool)> {
    let prompt_trimmed = prompt.trim();
    if prompt_trimmed.is_empty() {
        return Err(anyhow::anyhow!("workflow prompt is empty"));
    }
    let short = summarize_prompt(prompt_trimmed, 48);
    let proposal = recommendation::AutomationProposal {
        title: format!("Manual Workflow: {}", short),
        summary: format!(
            "Manual workflow request captured from {} (approval required before creation).",
            source
        ),
        trigger: "Manual workflow request".to_string(),
        actions: vec!["n8n Workflow".to_string()],
        confidence: 0.6,
        n8n_prompt: prompt_trimmed.to_string(),
        evidence: vec![
            format!("source={}", source),
            format!("prompt={}", summarize_prompt(prompt_trimmed, 160)),
        ],
        pattern_id: None,
    };

    let fp = proposal.fingerprint();
    let inserted = db::insert_recommendation(&proposal)?;
    let rec_id = find_recommendation_id_by_fingerprint(&fp)?
        .ok_or_else(|| anyhow::anyhow!("failed to resolve recommendation id after insert"))?;
    Ok((rec_id, inserted))
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
    let fp = proposal.fingerprint();
    let inserted = db::insert_recommendation(&proposal)?;
    let rec_id = find_recommendation_id_by_fingerprint(&fp)?
        .ok_or_else(|| anyhow::anyhow!("failed to resolve recommendation id after insert"))?;

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
    let ax_check = std::process::Command::new("osascript")
        .arg("-e")
        .arg("tell application \"System Events\" to return name of first application process")
        .output();

    match ax_check {
        Ok(output) if output.status.success() => {
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

    // 2. Start Scheduler (Brain)
    if let Some(llm) = &llm_client {
        let scheduler = scheduler::Scheduler::new(llm.clone());
        scheduler.start();
        info!("🧠 Brain Routine Scheduler Active.");
    }

    // 2.5 Init MCP
    if let Err(e) = mcp_client::init_mcp() {
        warn!("⚠️ Failed to init MCP: {}", e);
    } else {
        info!("🔌 MCP System Initialized.");
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

    // 5. Start File Watcher
    // Watch Downloads folder
    let home = std::env::var("HOME").unwrap_or("/".to_string());
    let downloads = format!("{}/Downloads", home);

    // We reuse log_tx to send file events to Analyzer
    if let Err(e) = monitor::spawn_file_watcher(downloads.clone(), log_tx.clone()) {
        println!("⚠️  Failed to watch {}: {}", downloads, e);
    } else {
        println!("👀 Watching for changes in {}", downloads);
    }

    // 6. Start App Watcher (Active Window Poller)
    monitor::spawn_app_watcher(log_tx.clone());
    println!("👀 Watching for active application changes...");

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
                println!("  analyze_patterns      - Detect behavior patterns and generate recommendations");
                println!("  quality               - Show workflow quality metrics");
                println!("  telegram <msg>        - Send Telegram message");
                println!("  notion <title>|<body> - Create Notion page");
                println!("  gmail list [N]        - List recent N emails");
                println!("  gmail read <id>       - Read email by ID");
                println!("  gmail send <to>|<subj>|<body> - Send email");
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
                match queue_manual_workflow_recommendation(&prompt, "cli.build_workflow") {
                    Ok((rec_id, inserted)) => {
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
                        "Usage: gmail list [N] | gmail read <id> | gmail send <to>|<subj>|<body>"
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
                    _ => println!("Unknown gmail subcommand. Use: list, read, send"),
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
