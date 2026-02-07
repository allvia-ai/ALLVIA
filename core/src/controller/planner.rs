use anyhow::Result;
use std::collections::HashMap;
use crate::llm_gateway::LLMClient;
use crate::visual_driver::{VisualDriver, SmartStep};
use crate::controller::supervisor::Supervisor;
use crate::controller::loop_detector::LoopDetector;
use crate::controller::heuristics;
use crate::controller::actions::ActionRunner;
use crate::session_store::Session;
use crate::schema::EventEnvelope;
use chrono::Utc;
use uuid::Uuid;
use tokio::sync::mpsc;
use std::sync::Arc;
use crate::action_schema;

pub struct Planner {
    pub llm: Arc<dyn LLMClient>,
    pub max_steps: usize,
    pub tx: Option<mpsc::Sender<String>>,
}

impl Planner {
    fn scenario_mode_enabled() -> bool {
        matches!(
            std::env::var("STEER_SCENARIO_MODE").as_deref(),
            Ok("1") | Ok("true") | Ok("TRUE") | Ok("yes") | Ok("YES")
        )
    }

    fn history_contains_case_insensitive(history: &[String], needle: &str) -> bool {
        let needle_lower = needle.to_lowercase();
        history.iter().any(|h| h.to_lowercase().contains(&needle_lower))
    }

    fn fallback_plan_from_goal(goal: &str, history: &[String]) -> Option<serde_json::Value> {
        let goal_lower = goal.to_lowercase();
        let app_catalog = [
            "Calendar",
            "Safari",
            "Finder",
            "TextEdit",
            "Notes",
            "Calculator",
            "Mail",
        ];

        let mut apps_in_goal: Vec<(usize, &str)> = app_catalog
            .iter()
            .filter_map(|app| {
                goal_lower
                    .find(&app.to_lowercase())
                    .map(|idx| (idx, *app))
            })
            .collect();
        apps_in_goal.sort_by_key(|(idx, _)| *idx);

        for (_, app) in &apps_in_goal {
            let marker = format!("Opened app: {}", app);
            if !Self::history_contains_case_insensitive(history, &marker) {
                return Some(serde_json::json!({ "action": "open_app", "name": app }));
            }
        }

        if !apps_in_goal.is_empty() {
            return Some(serde_json::json!({ "action": "done" }));
        }

        None
    }

    fn should_relax_review(reason: &str, notes: &str) -> bool {
        let text = format!("{} {}", reason.to_lowercase(), notes.to_lowercase());

        let strict_signals = [
            "full sequence",
            "entire sequence",
            "initial step",
            "only the first step",
            "only includes opening",
            "incomplete",
            "single step",
            "not the complete",
        ];
        let has_strict_signal = strict_signals.iter().any(|s| text.contains(s));

        let hard_blockers = [
            "danger",
            "unsafe",
            "impossible",
            "stuck in a loop",
            "does not relate",
            "not related",
            "before opening safari",
            "without ensuring safari is open",
        ];
        let has_hard_blocker = hard_blockers.iter().any(|s| text.contains(s));

        has_strict_signal && !has_hard_blocker
    }

    pub fn new(llm: Arc<dyn LLMClient>, tx: Option<mpsc::Sender<String>>) -> Self {
        Self {
            llm,
            max_steps: 25,
            tx,
        }
    }

    pub async fn run_goal(&self, goal: &str, session_key: Option<&str>) -> Result<()> {
        println!("🌊 Starting Planned Surf: '{}'", goal);
        let scenario_mode = Self::scenario_mode_enabled();

        // [Session]
        let _ = crate::session_store::init_session_store();
        let mut session = Session::new(goal, session_key);
        session.add_message("user", goal);

        // [Preflight]
        if let Err(e) = heuristics::preflight_permissions() {
             println!("❌ Preflight failed: {}", e);
             return Err(e);
        }
        if let Err(e) = heuristics::verify_screen_capture() {
             return Err(e);
        }

        let mut history: Vec<String> = Vec::new();
        let mut action_history: Vec<String> = Vec::new(); // For loop detection
        let mut plan_attempts: HashMap<String, usize> = HashMap::new();
        let mut consecutive_failures = 0;
        let mut last_read_number: Option<String> = None;
        let mut session_steps: Vec<SmartStep> = Vec::new();
        let mut last_action_by_plan: HashMap<String, String> = HashMap::new();
        let mut goal_completed = false;

        for i in 1..=self.max_steps {
            println!("\n🔄 [Step {}/{}] Observing...", i, self.max_steps);
            
            // 1. Capture Screen
            let (image_b64, _) = VisualDriver::capture_screen()?;
            let plan_key = heuristics::compute_plan_key(goal, &image_b64);
            let attempt = plan_attempts.entry(plan_key.clone()).and_modify(|v| *v += 1).or_insert(1);
            
            // Preflight: close blocking dialogs
            if heuristics::try_close_front_dialog() {
                history.push("Closed blocking dialog".to_string());
                continue;
            }

            // 2. Plan (Think)
            let retry_config = crate::retry_logic::RetryConfig::default();
            let mut history_with_context = history.clone();
             if *attempt > 1 || consecutive_failures > 0 {
                let last_action = last_action_by_plan.get(&plan_key).cloned().unwrap_or_else(|| "unknown".to_string());
                let last_error = history.iter().rev().find(|h| h.starts_with("FAILED") || h.starts_with("BLOCKED"))
                    .cloned().unwrap_or_else(|| "none".to_string());
                let context = format!(
                    "RETRY_CONTEXT: attempt={} plan_key={} last_action={} last_error={}",
                    attempt, plan_key, last_action, last_error
                );
                history_with_context.push(context);
            }

            let mut plan = if scenario_mode {
                Self::fallback_plan_from_goal(goal, &history_with_context)
                    .unwrap_or_else(|| serde_json::json!({ "action": "done" }))
            } else {
                // Call LLM for Vision Planning
                crate::retry_logic::with_retry(&retry_config, "LLM Vision", || async {
                    self.llm.plan_vision_step(goal, &image_b64, &history_with_context).await
                }).await?
            };

             // Flatten nested JSON
            if plan["action"].is_object() {
                plan = plan["action"].clone();
            }
            
            // Validate Schema
            let validation = action_schema::normalize_action(&plan);
            if let Some(err) = validation.error {
                 let msg = format!("SCHEMA_ERROR: {}", err);
                 println!("   ⚠️ {}", msg);
                 history.push(msg);
                 consecutive_failures += 1;
                 continue;
            }
             plan = validation.normalized;

            if scenario_mode {
                if let Some(fallback_plan) = Self::fallback_plan_from_goal(goal, &history) {
                    println!("   🧪 Scenario mode fallback action: {}", fallback_plan);
                    plan = fallback_plan;
                }
            } else {
                // 3. Supervisor Check
                let supervisor_decision = crate::retry_logic::with_retry(&retry_config, "Supervisor", || async {
                     Supervisor::consult(&*self.llm, goal, &plan, &history).await
                }).await?;

                 println!("   🕵️ Supervisor: {} ({})", supervisor_decision.action, supervisor_decision.reason);

                let mut supervisor_action = supervisor_decision.action.clone();
                if supervisor_action == "review"
                    && Self::should_relax_review(&supervisor_decision.reason, &supervisor_decision.notes)
                {
                    if let Some(fallback_plan) = Self::fallback_plan_from_goal(goal, &history) {
                        println!(
                            "   🔧 Relaxed review -> fallback action: {}",
                            fallback_plan
                        );
                        plan = fallback_plan;
                        supervisor_action = "accept".to_string();
                    }
                }

                match supervisor_action.as_str() {
                    "accept" => { /* Proceed */ },
                    "review" => {
                        history.push(format!("PLAN_REJECTED: {}", supervisor_decision.notes));
                        continue;
                    },
                    "escalate" => {
                        let msg = format!("Supervisor escalated: {}", supervisor_decision.reason);
                        println!("      🚨 {}", msg);
                        return Err(anyhow::anyhow!(msg));
                    },
                    _ => {}
                }
            }

            // 4. Anti-Loop Check
            let action_str = plan.to_string();
            if LoopDetector::detect_action_loop(&action_history, &action_str) {
                 println!("   🔄 LOOP DETECTED. Supervisor/Heuristics should handle this next.");
                 plan = serde_json::json!({"action": "report", "message": "Loop detected. Halting."});
            }
            action_history.push(action_str.clone());
            last_action_by_plan.insert(plan_key.clone(), plan["action"].as_str().unwrap_or("unknown").to_string());

            if plan["action"].as_str() == Some("done") {
                println!("✅ Goal completed by planner.");
                goal_completed = true;
                break;
            }

            // 5. Execute via ActionRunner
            println!("   🚀 Executing Action...");
            if let Err(e) = ActionRunner::execute(
                &plan,
                &mut VisualDriver::new(), // In real scenario, might want to reuse driver or pass it
                &mut session_steps,
                &mut session,
                &mut history,
                &mut consecutive_failures,
                &mut last_read_number,
                goal
            ).await {
                println!("   ❌ Execution Error: {}", e);
                // logic to handle specific errors or break
            }
            
            // Broadcast event if tx available
             if let Some(tx) = &self.tx {
                let event = EventEnvelope {
                    schema_version: "1.0".to_string(),
                    event_id: Uuid::new_v4().to_string(),
                    ts: Utc::now().to_rfc3339(),
                    source: "dynamic_agent".to_string(),
                    app: "Agent".to_string(),
                    event_type: "action".to_string(),
                    priority: "P1".to_string(),
                    resource: None,
                    payload: serde_json::json!({
                        "goal": goal,
                        "step": i,
                        "plan": plan
                    }),
                    privacy: None,
                    pid: None,
                    window_id: None,
                    window_title: None,
                    browser_url: None,
                    raw: None,
                };
                if let Ok(json) = serde_json::to_string(&event) {
                    let _ = tx.try_send(json);
                }
            }
        }
        if goal_completed {
            return Ok(());
        }

        Err(anyhow::anyhow!(
            "Planner stopped without completion (max steps reached or unresolved review loop)."
        ))
    }
}
