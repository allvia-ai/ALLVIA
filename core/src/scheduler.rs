use crate::llm_gateway::LLMClient;
use crate::{db, workflow_intake};
use cron::Schedule;
use std::collections::HashSet;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::time::{self, Duration};

static SCHEDULER_STARTED: AtomicBool = AtomicBool::new(false);

struct RoutineClaimGuard {
    routine_id: i64,
    owner: String,
}

impl Drop for RoutineClaimGuard {
    fn drop(&mut self) {
        let _ = db::release_routine_execution(self.routine_id, Some(self.owner.as_str()));
    }
}

pub struct Scheduler {
    llm: Arc<dyn LLMClient>,
}

impl Scheduler {
    pub fn new(llm: Arc<dyn LLMClient>) -> Self {
        Self { llm }
    }

    pub fn start(&self) {
        let allow_multi_scheduler = std::env::var("STEER_ALLOW_MULTI_SCHEDULER")
            .ok()
            .map(|v| {
                matches!(
                    v.trim().to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes" | "on"
                )
            })
            .unwrap_or(false);
        if !allow_multi_scheduler
            && SCHEDULER_STARTED
                .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                .is_err()
        {
            println!("⏭️ Scheduler already started. Ignoring duplicate start().");
            crate::diagnostic_events::emit(
                "scheduler.start.skipped",
                serde_json::json!({
                    "reason": "already_started"
                }),
            );
            return;
        }

        let llm = self.llm.clone();
        let active_routines = Arc::new(tokio::sync::Mutex::new(HashSet::<i64>::new()));
        let claim_owner = format!(
            "scheduler:{}:{}",
            std::process::id(),
            chrono::Utc::now().timestamp_millis()
        );

        tokio::spawn(async move {
            println!("⏰ Routine Scheduler started (Tick: 60s)");
            crate::diagnostic_events::emit(
                "scheduler.start",
                serde_json::json!({
                    "tick_seconds": 60,
                    "claim_owner": claim_owner
                }),
            );
            let max_retries: u32 = std::env::var("ROUTINE_MAX_RETRIES")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(1);
            let retry_delay_secs: u64 = std::env::var("ROUTINE_RETRY_DELAY_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(30);
            let collector_handoff_auto_consume =
                std::env::var("STEER_COLLECTOR_HANDOFF_AUTOCONSUME")
                    .ok()
                    .map(|v| {
                        matches!(
                            v.trim().to_lowercase().as_str(),
                            "1" | "true" | "yes" | "on"
                        )
                    })
                    .unwrap_or(true);
            let workflow_reconcile_enabled = std::env::var("STEER_WORKFLOW_RECONCILE_ENABLED")
                .ok()
                .map(|v| {
                    matches!(
                        v.trim().to_lowercase().as_str(),
                        "1" | "true" | "yes" | "on"
                    )
                })
                .unwrap_or(true);
            let workflow_reconcile_limit = std::env::var("STEER_WORKFLOW_RECONCILE_LIMIT")
                .ok()
                .and_then(|v| v.trim().parse::<i64>().ok())
                .filter(|v| *v >= 1)
                .unwrap_or(20);

            loop {
                // Check every 60 seconds
                time::sleep(Duration::from_secs(60)).await;

                if collector_handoff_auto_consume {
                    match workflow_intake::ingest_latest_collector_handoff(None) {
                        Ok(outcome) => {
                            if outcome.status != "noop" {
                                println!(
                                    "📥 Collector handoff ingest: status={} detail={}",
                                    outcome.status, outcome.detail
                                );
                            }
                        }
                        Err(e) => {
                            eprintln!("⚠️ Collector handoff ingest error: {}", e);
                        }
                    }
                }

                if workflow_reconcile_enabled {
                    match db::reconcile_workflow_provision_ops(workflow_reconcile_limit) {
                        Ok(outcomes) => {
                            if !outcomes.is_empty() {
                                println!(
                                    "🧩 Workflow provision reconcile processed {} op(s)",
                                    outcomes.len()
                                );
                                for line in outcomes.iter().take(5) {
                                    println!("   - {}", line);
                                }
                            }
                        }
                        Err(e) => {
                            eprintln!("⚠️ Workflow provision reconcile error: {}", e);
                        }
                    }
                }

                // --- Proactive Pattern Check (Every 10 mins approx) ---
                // Ideally use a timestamp check, but for MVP checking random chance or counter
                // Let's rely on a separate spawn for pattern capability

                // Get due routines
                let due = match db::get_due_routines() {
                    Ok(r) => r,
                    Err(e) => {
                        eprintln!("⚠️ Scheduler DB Error: {}", e);
                        continue;
                    }
                };

                if !due.is_empty() {
                    println!("⏰ Found {} due routines!", due.len());
                }

                // Limit concurrency to prevent resource explosion
                let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(5));

                for routine in due {
                    {
                        let mut active = active_routines.lock().await;
                        if active.contains(&routine.id) {
                            println!(
                                "⏭️ Skipping Routine #{} (already running): {}",
                                routine.id, routine.name
                            );
                            continue;
                        }
                        active.insert(routine.id);
                    }

                    match db::claim_routine_execution(routine.id, &claim_owner) {
                        Ok(true) => {}
                        Ok(false) => {
                            println!(
                                "⏭️ Skipping Routine #{} (claimed by another runner): {}",
                                routine.id, routine.name
                            );
                            let mut active = active_routines.lock().await;
                            active.remove(&routine.id);
                            continue;
                        }
                        Err(e) => {
                            eprintln!(
                                "⚠️ Failed to claim Routine #{} before execution: {}",
                                routine.id, e
                            );
                            let mut active = active_routines.lock().await;
                            active.remove(&routine.id);
                            continue;
                        }
                    }

                    let permit = match semaphore.clone().acquire_owned().await {
                        Ok(p) => p,
                        Err(e) => {
                            eprintln!("⚠️ Semaphore acquire failed: {}", e);
                            let _ = db::release_routine_execution(routine.id, Some(&claim_owner));
                            let mut active = active_routines.lock().await;
                            active.remove(&routine.id);
                            continue;
                        }
                    };
                    println!("⏰ Executing Routine #{}: {}", routine.id, routine.name);
                    let routine_id = routine.id;
                    let run_id = db::create_routine_run(routine.id).ok();

                    // Calculate next run FIRST... (omitted lines 43-53 remain same, but inside loop)
                    if let Ok(schedule) = Schedule::from_str(&routine.cron_expression) {
                        if let Some(next) = schedule.upcoming(chrono::Utc).next() {
                            let _ =
                                db::update_routine_execution(routine.id, Some(next.to_rfc3339()));
                        }
                    }

                    let prompt = routine.prompt.clone();
                    let llm_clone = llm.clone();
                    let active_routines_for_task = active_routines.clone();
                    let claim_owner_for_task = claim_owner.clone();

                    tokio::spawn(async move {
                        let _claim_guard = RoutineClaimGuard {
                            routine_id,
                            owner: claim_owner_for_task,
                        };
                        let _permit = permit; // Drop permit when task finishes
                        println!("   ▶️ running routine logic: '{}'...", prompt);

                        // Instantiate Executor on the fly (lightweight enough)
                        let planner =
                            crate::controller::planner::Planner::new(llm_clone.clone(), None);
                        let mut attempt: u32 = 0;
                        loop {
                            match planner.run_goal_tracked(&prompt, None).await {
                                Ok(outcome) => {
                                    if attempt > 0 {
                                        println!(
                                            "✅ Routine '{}' Recovered after {} retries (run_id={})",
                                            prompt, attempt, outcome.run_id
                                        );
                                    } else {
                                        println!(
                                            "✅ Routine '{}' Completed (run_id={})",
                                            prompt, outcome.run_id
                                        );
                                    }
                                    if let Some(id) = run_id {
                                        let _ = db::finish_routine_run(id, "success", None);
                                    }
                                    break;
                                }
                                Err(e) => {
                                    attempt += 1;
                                    let err_msg = e.to_string();
                                    let err_type = classify_error(&err_msg);
                                    let stored_error = format!("[{}] {}", err_type, err_msg);
                                    if attempt > max_retries {
                                        eprintln!(
                                            "❌ Routine '{}' Failed after {} attempts: {}",
                                            prompt, attempt, stored_error
                                        );
                                        if let Some(id) = run_id {
                                            let _ = db::finish_routine_run(
                                                id,
                                                "failed",
                                                Some(&stored_error),
                                            );
                                        }
                                        break;
                                    }
                                    println!(
                                        "⚠️ Routine '{}' attempt {} failed. Retrying in {}s...",
                                        prompt, attempt, retry_delay_secs
                                    );
                                    time::sleep(Duration::from_secs(
                                        retry_delay_secs * attempt as u64,
                                    ))
                                    .await;
                                }
                            }
                        }
                        let mut active = active_routines_for_task.lock().await;
                        active.remove(&routine_id);
                    });
                }
            }
        });

        // Separate loop for Passive Analysis (Background Brain)
        let llm_for_analysis = self.llm.clone();
        tokio::spawn(async move {
            loop {
                // Analysis runs every 5 minutes
                time::sleep(Duration::from_secs(300)).await;

                println!("🧠 [Background] Analyzing recent behavior patterns...");
                let detector = crate::pattern_detector::PatternDetector::new();
                let patterns = detector.analyze();

                for pattern in patterns {
                    // High confidence/occurrence only for auto-notification
                    if pattern.occurrences >= 5 && pattern.similarity_score >= 0.85 {
                        let brain = &llm_for_analysis;
                        if let Ok(proposal) = brain
                            .generate_recommendation_from_pattern(
                                &pattern.description,
                                &pattern.sample_events,
                            )
                            .await
                        {
                            if proposal.confidence >= 0.8 {
                                // Check if already recommended to avoid spam
                                if let Ok(true) = db::insert_recommendation(&proposal) {
                                    let _ = crate::notifier::send(
                                        "💡 New Workflow Idea",
                                        &format!(
                                            "I noticed you do '{}' a lot. Shall I automate it?",
                                            proposal.title
                                        ),
                                    );
                                }
                            }
                        }
                    }
                }
            }
        });
    }
}

fn classify_error(message: &str) -> &'static str {
    let msg = message.to_lowercase();
    if msg.contains("permission") || msg.contains("access") || msg.contains("denied") {
        "permission"
    } else if msg.contains("timeout") || msg.contains("timed out") {
        "timeout"
    } else if msg.contains("network") || msg.contains("connection") || msg.contains("dns") {
        "network"
    } else if msg.contains("not found") || msg.contains("missing") {
        "missing"
    } else {
        "execution"
    }
}
