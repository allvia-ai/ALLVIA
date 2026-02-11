use crate::db;
use crate::llm_gateway::LLMClient;
use cron::Schedule;
use std::str::FromStr;
use std::sync::Arc;
use tokio::time::{self, Duration};

pub struct Scheduler {
    llm: Arc<dyn LLMClient>,
}

impl Scheduler {
    pub fn new(llm: Arc<dyn LLMClient>) -> Self {
        Self { llm }
    }

    pub fn start(&self) {
        let llm = self.llm.clone();

        tokio::spawn(async move {
            println!("⏰ Routine Scheduler started (Tick: 60s)");
            let max_retries: u32 = std::env::var("ROUTINE_MAX_RETRIES")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(1);
            let retry_delay_secs: u64 = std::env::var("ROUTINE_RETRY_DELAY_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(30);

            loop {
                // Check every 60 seconds
                time::sleep(Duration::from_secs(60)).await;

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
                    let permit = match semaphore.clone().acquire_owned().await {
                        Ok(p) => p,
                        Err(e) => {
                            eprintln!("⚠️ Semaphore acquire failed: {}", e);
                            continue;
                        }
                    };
                    println!("⏰ Executing Routine #{}: {}", routine.id, routine.name);
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

                    tokio::spawn(async move {
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
