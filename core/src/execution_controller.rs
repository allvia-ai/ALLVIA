use crate::approval_gate;
use crate::controller::heuristics;
use crate::nl_automation::{ApprovalContext, ExecutionResult, Plan, StepType};

use crate::browser_automation;
use crate::tool_chaining::CrossAppBridge;
use crate::visual_driver::{SmartStep, UiAction, VisualDriver};
use serde_json::{json, Value};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::time::Duration;

#[cfg(target_os = "macos")]
use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputCollisionPolicy {
    Ignore,
    Pause,
    Abort,
}

impl InputCollisionPolicy {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Ignore => "ignore",
            Self::Pause => "pause",
            Self::Abort => "abort",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ExecutionOptions {
    pub enforce_browser_focus: bool,
    pub input_collision_policy: InputCollisionPolicy,
}

impl Default for ExecutionOptions {
    fn default() -> Self {
        Self {
            enforce_browser_focus: false,
            input_collision_policy: InputCollisionPolicy::Ignore,
        }
    }
}

impl ExecutionOptions {
    pub fn strict() -> Self {
        Self {
            enforce_browser_focus: true,
            input_collision_policy: InputCollisionPolicy::Abort,
        }
    }

    pub fn test() -> Self {
        Self {
            enforce_browser_focus: true,
            input_collision_policy: InputCollisionPolicy::Pause,
        }
    }

    pub fn fast() -> Self {
        Self {
            enforce_browser_focus: false,
            input_collision_policy: InputCollisionPolicy::Ignore,
        }
    }
}

fn build_resume_token(plan: &Plan, resume_from: Option<usize>, reason: &str) -> Option<String> {
    let next_step = resume_from?;
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    Some(format!(
        "resume:{}:{}:{}:{}",
        plan.plan_id, next_step, reason, ts
    ))
}

fn push_run_attempt(logs: &mut Vec<String>, phase: &str, status: &str, details: &str) {
    let ts = chrono::Utc::now().to_rfc3339();
    logs.push(format!(
        "RUN_ATTEMPT|phase={}|status={}|details={}|ts={}",
        phase, status, details, ts
    ));
    let payload = json!({
        "type": "run.attempt",
        "phase": phase,
        "status": status,
        "details": details,
        "ts": ts
    });
    logs.push(format!("RUN_ATTEMPT_JSON|{}", payload));
    crate::diagnostic_events::emit(
        "run.attempt",
        json!({
            "phase": phase,
            "status": status,
            "details": details
        }),
    );
}

fn step_requires_browser_focus(step_type: &StepType) -> bool {
    matches!(
        step_type,
        StepType::Fill | StepType::Click | StepType::Select | StepType::Extract
    )
}

fn is_browser_app(app_name: &str) -> bool {
    let lower = app_name.to_lowercase();
    lower.contains("safari")
        || lower.contains("chrome")
        || lower.contains("firefox")
        || lower.contains("brave")
        || lower.contains("edge")
        || lower.contains("arc")
        || lower.contains("opera")
}

fn interrupt_guard_enabled() -> bool {
    std::env::var("STEER_INTERRUPT_GUARD")
        .ok()
        .map(|v| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(true)
}

fn should_guard_interrupt_for_step(step_type: &StepType) -> bool {
    matches!(
        step_type,
        StepType::Fill | StepType::Click | StepType::Select | StepType::Extract
    )
}

fn expected_front_app_for_step(step_data: &Value) -> Option<String> {
    step_data
        .get("app")
        .and_then(|v| v.as_str())
        .or_else(|| step_data.get("name").and_then(|v| v.as_str()))
        .map(|raw| raw.trim().to_string())
        .filter(|raw| !raw.is_empty())
}

#[derive(Debug, Default)]
struct FocusHandoffState {
    drift_events: usize,
    recovery_attempts: usize,
    recovered_events: usize,
    failed_events: usize,
}

fn parse_bool_env_with_default(name: &str, default: bool) -> bool {
    std::env::var(name)
        .ok()
        .map(|v| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(default)
}

fn focus_handoff_enabled() -> bool {
    parse_bool_env_with_default("STEER_EXEC_FOCUS_HANDOFF", true)
}

fn focus_handoff_finder_bridge_enabled() -> bool {
    parse_bool_env_with_default("STEER_EXEC_FOCUS_HANDOFF_FINDER_BRIDGE", true)
}

fn focus_handoff_retries() -> usize {
    std::env::var("STEER_EXEC_FOCUS_HANDOFF_RETRIES")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .map(|v| v.clamp(1, 6))
        .unwrap_or(2)
}

fn focus_handoff_retry_ms() -> u64 {
    std::env::var("STEER_EXEC_FOCUS_HANDOFF_RETRY_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map(|v| v.clamp(80, 1200))
        .unwrap_or(220)
}

fn user_activity_guard_enabled() -> bool {
    parse_bool_env_with_default("STEER_USER_ACTIVITY_GUARD_ENABLED", true)
}

fn user_activity_idle_resume_secs() -> u64 {
    std::env::var("STEER_USER_ACTIVITY_IDLE_RESUME_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map(|v| v.clamp(5, 600))
        .unwrap_or(60)
}

fn user_activity_poll_ms() -> u64 {
    std::env::var("STEER_USER_ACTIVITY_POLL_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map(|v| v.clamp(200, 5000))
        .unwrap_or(1000)
}

#[cfg(target_os = "macos")]
fn parse_idle_ns_from_ioreg(raw: &str) -> Option<u64> {
    for line in raw.lines() {
        if !line.contains("\"HIDIdleTime\"") {
            continue;
        }
        let value = line.split('=').nth(1)?.trim();
        if let Some(hex) = value.strip_prefix("0x") {
            if let Ok(ns) = u64::from_str_radix(hex.trim(), 16) {
                return Some(ns);
            }
        } else if let Some(first_token) = value.split_whitespace().next() {
            if let Ok(ns) = first_token.parse::<u64>() {
                return Some(ns);
            }
        }
    }
    None
}

#[cfg(target_os = "macos")]
fn system_idle_secs() -> Option<f64> {
    let output = Command::new("ioreg")
        .args(["-c", "IOHIDSystem"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    parse_idle_ns_from_ioreg(&text).map(|ns| ns as f64 / 1_000_000_000.0)
}

#[cfg(not(target_os = "macos"))]
fn system_idle_secs() -> Option<f64> {
    None
}

async fn wait_until_user_idle_if_active(
    logs: &mut Vec<String>,
    step_idx: usize,
    reason: &str,
) -> bool {
    if !user_activity_guard_enabled() {
        return false;
    }
    let resume_secs = user_activity_idle_resume_secs() as f64;
    let poll_ms = user_activity_poll_ms();
    let mut idle_secs = match system_idle_secs() {
        Some(v) => v,
        None => return false,
    };
    if idle_secs >= resume_secs {
        return false;
    }

    push_run_attempt(
        logs,
        "user_activity_pause",
        "waiting",
        &format!(
            "step={} reason={} idle_secs={:.1} resume_secs={}",
            step_idx + 1,
            reason,
            idle_secs,
            resume_secs as u64
        ),
    );
    logs.push(format!(
        "USER_ACTIVITY_PAUSE: step={} reason={} (idle {:.1}s < {}s). Waiting for user idle...",
        step_idx + 1,
        reason,
        idle_secs,
        resume_secs as u64
    ));

    let mut wait_loops = 0usize;
    loop {
        tokio::time::sleep(Duration::from_millis(poll_ms)).await;
        wait_loops += 1;
        idle_secs = system_idle_secs().unwrap_or(0.0);
        if idle_secs >= resume_secs {
            break;
        }
        if wait_loops % 10 == 0 {
            logs.push(format!(
                "USER_ACTIVITY_WAITING: step={} idle={:.1}s target={}s",
                step_idx + 1,
                idle_secs,
                resume_secs as u64
            ));
        }
    }

    push_run_attempt(
        logs,
        "user_activity_pause",
        "resumed",
        &format!(
            "step={} reason={} idle_secs={:.1}",
            step_idx + 1,
            reason,
            idle_secs
        ),
    );
    logs.push(format!(
        "USER_ACTIVITY_RESUMED: step={} reason={} (idle {:.1}s >= {}s)",
        step_idx + 1,
        reason,
        idle_secs,
        resume_secs as u64
    ));
    true
}

fn app_matches_expected(front_app: &str, expected_app: &str) -> bool {
    let front = front_app.trim();
    let expected = expected_app.trim();
    if front.is_empty() || expected.is_empty() {
        return false;
    }
    if front.eq_ignore_ascii_case(expected) {
        return true;
    }
    let front_lower = front.to_ascii_lowercase();
    let expected_lower = expected.to_ascii_lowercase();
    front_lower.contains(&expected_lower) || expected_lower.contains(&front_lower)
}

async fn recover_expected_focus(
    logs: &mut Vec<String>,
    focus_state: &mut FocusHandoffState,
    step_idx: usize,
    expected_app: &str,
    front_before: &str,
) -> (bool, String) {
    focus_state.drift_events += 1;
    let retries = focus_handoff_retries();
    let retry_ms = focus_handoff_retry_ms();
    let use_finder_bridge = focus_handoff_finder_bridge_enabled();
    push_run_attempt(
        logs,
        "focus_handoff",
        "drift_detected",
        &format!(
            "step={} expected_app={} frontmost={}",
            step_idx + 1,
            expected_app,
            front_before
        ),
    );

    let mut last_front = front_before.to_string();
    for attempt in 1..=retries {
        focus_state.recovery_attempts += 1;
        push_run_attempt(
            logs,
            "focus_handoff",
            "recovering",
            &format!(
                "step={} attempt={}/{} target={}",
                step_idx + 1,
                attempt,
                retries,
                expected_app
            ),
        );

        let _ = heuristics::ensure_app_focus(expected_app, 3).await;
        if use_finder_bridge {
            let front_now = CrossAppBridge::get_frontmost_app().unwrap_or_default();
            if !app_matches_expected(&front_now, expected_app) {
                let _ = heuristics::ensure_app_focus("Finder", 2).await;
                let _ = heuristics::ensure_app_focus(expected_app, 3).await;
            }
        }

        let front_after = CrossAppBridge::get_frontmost_app().unwrap_or_default();
        last_front = front_after.clone();
        if app_matches_expected(&front_after, expected_app) {
            focus_state.recovered_events += 1;
            push_run_attempt(
                logs,
                "focus_handoff",
                "recovered",
                &format!(
                    "step={} attempt={} expected_app={} frontmost={}",
                    step_idx + 1,
                    attempt,
                    expected_app,
                    front_after
                ),
            );
            return (true, front_after);
        }
        tokio::time::sleep(Duration::from_millis(retry_ms)).await;
    }

    focus_state.failed_events += 1;
    push_run_attempt(
        logs,
        "focus_handoff",
        "failed",
        &format!(
            "step={} expected_app={} frontmost={} retries={}",
            step_idx + 1,
            expected_app,
            last_front,
            retries
        ),
    );
    (false, last_front)
}

async fn recover_browser_focus(
    logs: &mut Vec<String>,
    focus_state: &mut FocusHandoffState,
    step_idx: usize,
    front_before: &str,
) -> (bool, String) {
    focus_state.drift_events += 1;
    let browser_candidates = [
        "Google Chrome",
        "Safari",
        "Arc",
        "Brave Browser",
        "Microsoft Edge",
        "Firefox",
    ];
    push_run_attempt(
        logs,
        "focus_handoff_browser",
        "recovering",
        &format!("step={} frontmost={}", step_idx + 1, front_before),
    );
    let mut last_front = front_before.to_string();
    for app in browser_candidates {
        focus_state.recovery_attempts += 1;
        let _ = heuristics::ensure_app_focus(app, 2).await;
        let front_after = CrossAppBridge::get_frontmost_app().unwrap_or_default();
        last_front = front_after.clone();
        if is_browser_app(&front_after) {
            focus_state.recovered_events += 1;
            push_run_attempt(
                logs,
                "focus_handoff_browser",
                "recovered",
                &format!(
                    "step={} target={} frontmost={}",
                    step_idx + 1,
                    app,
                    front_after
                ),
            );
            return (true, front_after);
        }
    }
    focus_state.failed_events += 1;
    push_run_attempt(
        logs,
        "focus_handoff_browser",
        "failed",
        &format!("step={} frontmost={}", step_idx + 1, last_front),
    );
    (false, last_front)
}

fn push_focus_handoff_summary(logs: &mut Vec<String>, focus_state: &FocusHandoffState) {
    push_run_attempt(
        logs,
        "focus_handoff_summary",
        "done",
        &format!(
            "drift_events={},recovery_attempts={},recovered_events={},failed_events={}",
            focus_state.drift_events,
            focus_state.recovery_attempts,
            focus_state.recovered_events,
            focus_state.failed_events
        ),
    );
}

pub async fn execute_plan(
    plan: &Plan,
    start_index: usize,
    options: ExecutionOptions,
) -> ExecutionResult {
    let mut logs = Vec::new();
    let mut manual_required = false;
    let mut manual_steps: Vec<String> = Vec::new();
    let mut approval_required = false;
    let mut blocked = false;
    let mut blocked_reason = "policy_blocked".to_string();
    let mut approval_context: Option<ApprovalContext> = None;
    let mut resume_from: Option<usize> = None;
    let mut focus_handoff_state = FocusHandoffState::default();

    logs.push(format!(
        "Start plan {} ({})",
        plan.plan_id,
        plan.intent.as_str()
    ));
    logs.push(summary_for_plan(plan));
    logs.push(format!(
        "Execution options: enforce_browser_focus={}, input_collision_policy={}",
        options.enforce_browser_focus,
        options.input_collision_policy.as_str()
    ));
    push_run_attempt(
        &mut logs,
        "execution_start",
        "running",
        &format!("plan_id={},start_index={}", plan.plan_id, start_index),
    );

    for (idx, step) in plan.steps.iter().enumerate().skip(start_index) {
        logs.push(format!(
            "Step {}: {} ({:?})",
            idx + 1,
            step.description,
            step.step_type
        ));

        if interrupt_guard_enabled()
            && options.input_collision_policy != InputCollisionPolicy::Ignore
            && should_guard_interrupt_for_step(&step.step_type)
        {
            if let Some(expected_app) = expected_front_app_for_step(&step.data) {
                let front_app = CrossAppBridge::get_frontmost_app().unwrap_or_default();
                let mut front_trimmed = front_app.trim().to_string();
                let mut recovered = false;
                if !front_trimmed.is_empty() && !app_matches_expected(&front_trimmed, &expected_app)
                {
                    let _ = wait_until_user_idle_if_active(
                        &mut logs,
                        idx,
                        "frontmost_mismatch_expected_app",
                    )
                    .await;
                    let front_after_idle = CrossAppBridge::get_frontmost_app().unwrap_or_default();
                    if !front_after_idle.trim().is_empty() {
                        front_trimmed = front_after_idle.trim().to_string();
                    }
                    if focus_handoff_enabled() {
                        let (ok, recovered_front) = recover_expected_focus(
                            &mut logs,
                            &mut focus_handoff_state,
                            idx,
                            &expected_app,
                            &front_trimmed,
                        )
                        .await;
                        recovered = ok;
                        if !recovered_front.trim().is_empty() {
                            front_trimmed = recovered_front;
                        }
                    }
                }
                if !front_trimmed.is_empty()
                    && !app_matches_expected(&front_trimmed, &expected_app)
                    && !recovered
                {
                    manual_required = true;
                    resume_from = Some(idx);
                    manual_steps.push(format!(
                        "Step {} 전면 앱 충돌: expected={} actual={} (수동 복구 후 Resume)",
                        idx + 1,
                        expected_app,
                        front_trimmed
                    ));
                    logs.push(format!(
                        "INTERRUPT_DETECTED: step={} expected_app={} frontmost={} policy={}",
                        idx + 1,
                        expected_app,
                        front_trimmed,
                        options.input_collision_policy.as_str()
                    ));
                    push_run_attempt(
                        &mut logs,
                        "user_interrupt",
                        "manual_required",
                        &format!(
                            "step={} expected_app={} frontmost={}",
                            idx + 1,
                            expected_app,
                            front_trimmed
                        ),
                    );
                    break;
                } else if recovered {
                    logs.push(format!(
                        "FOCUS_HANDOFF_RECOVERED: step={} expected_app={} frontmost={}",
                        idx + 1,
                        expected_app,
                        front_trimmed
                    ));
                }
            }
        }

        if options.enforce_browser_focus && step_requires_browser_focus(&step.step_type) {
            let mut front_app = CrossAppBridge::get_frontmost_app().unwrap_or_default();
            if !is_browser_app(&front_app) && focus_handoff_enabled() {
                let _ =
                    wait_until_user_idle_if_active(&mut logs, idx, "browser_focus_required").await;
                let front_after_idle = CrossAppBridge::get_frontmost_app().unwrap_or_default();
                if !front_after_idle.trim().is_empty() {
                    front_app = front_after_idle;
                }
                let (ok, recovered_front) =
                    recover_browser_focus(&mut logs, &mut focus_handoff_state, idx, &front_app)
                        .await;
                if ok {
                    front_app = recovered_front;
                }
            }
            if !is_browser_app(&front_app) {
                let collision_details = format!(
                    "step={} expected=browser frontmost={} policy={}",
                    idx + 1,
                    front_app,
                    options.input_collision_policy.as_str()
                );
                logs.push(format!(
                    "INPUT_COLLISION: UI step requires browser focus but frontmost app is '{}'",
                    front_app
                ));
                push_run_attempt(&mut logs, "input_collision", "detected", &collision_details);
                match options.input_collision_policy {
                    InputCollisionPolicy::Ignore => {
                        logs.push("Input collision ignored by execution profile".to_string());
                    }
                    InputCollisionPolicy::Pause => {
                        manual_required = true;
                        resume_from = Some(idx);
                        manual_steps.push(format!(
                            "Step {} 실행 전 브라우저를 전면으로 복구하고 Resume 하세요",
                            idx + 1
                        ));
                        logs.push(
                            "Execution paused due to input collision (manual intervention required)"
                                .to_string(),
                        );
                        break;
                    }
                    InputCollisionPolicy::Abort => {
                        blocked = true;
                        blocked_reason = "input_collision_abort".to_string();
                        resume_from = Some(idx);
                        logs.push(
                            "Execution aborted due to input collision policy (strict)".to_string(),
                        );
                        break;
                    }
                }
            }
        }

        match step.step_type {
            StepType::Navigate => {
                if let Some(url) = step.data.get("url").and_then(|v| v.as_str()) {
                    if let Err(err) = browser_automation::open_url_in_chrome(url)
                        .or_else(|_| crate::applescript::open_url(url).map(|_| ()))
                    {
                        logs.push(format!("Failed to open url {}: {}", url, err));
                        return ExecutionResult {
                            status: "error".to_string(),
                            logs,
                            approval: approval_context,
                            manual_steps,
                            resume_from,
                            resume_token: None,
                        };
                    }
                } else {
                    logs.push("Navigate step missing url".to_string());
                }
            }
            StepType::Wait => {
                let seconds = step
                    .data
                    .get("seconds")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(1);
                tokio::time::sleep(tokio::time::Duration::from_secs(seconds)).await;
            }
            StepType::Select => {
                if is_auto_step(&step.data) {
                    let applied = match plan.intent {
                        crate::nl_automation::IntentType::FlightSearch => {
                            let budget = step.data.get("budget").and_then(|v| v.as_str());
                            let time_window = step.data.get("time_window").and_then(|v| v.as_str());
                            let direct_only = step.data.get("direct_only").and_then(|v| v.as_str());
                            if budget.is_none() && time_window.is_none() && direct_only.is_none() {
                                logs.push("No flight filters to apply".to_string());
                                continue;
                            }
                            browser_automation::apply_flight_filters(
                                budget,
                                time_window,
                                direct_only,
                            )
                        }
                        crate::nl_automation::IntentType::ShoppingCompare => {
                            let brand = step.data.get("brand").and_then(|v| v.as_str());
                            let price_min = step.data.get("price_min").and_then(|v| v.as_str());
                            let price_max = step.data.get("price_max").and_then(|v| v.as_str());
                            if brand.is_none() && price_min.is_none() && price_max.is_none() {
                                logs.push("No shopping filters to apply".to_string());
                                continue;
                            }
                            browser_automation::apply_shopping_filters(brand, price_min, price_max)
                        }
                        _ => Ok(false),
                    };

                    match applied {
                        Ok(true) => logs.push("Filters applied".to_string()),
                        Ok(false) => {
                            manual_required = true;
                            manual_steps.push(step.description.clone());
                            logs.push(format!(
                                "Manual filters required for step '{}'",
                                step.description
                            ));
                        }
                        Err(err) => {
                            manual_required = true;
                            manual_steps.push(step.description.clone());
                            logs.push(format!("Filter apply failed: {}", err));
                        }
                    }
                } else {
                    manual_required = true;
                    manual_steps.push(step.description.clone());
                    logs.push(format!(
                        "Manual filters required for step '{}'",
                        step.description
                    ));
                }
            }
            StepType::Fill | StepType::Click => {
                if is_auto_step(&step.data) {
                    if let Some(action) = step.data.get("action").and_then(|v| v.as_str()) {
                        if action == "submit_search" {
                            let mut clicked = false;
                            for attempt in 0..2 {
                                match browser_automation::click_search_button() {
                                    Ok(true) => {
                                        logs.push("Clicked search button".to_string());
                                        clicked = true;
                                        break;
                                    }
                                    Ok(false) => {
                                        logs.push(format!(
                                            "Search button not found (attempt {})",
                                            attempt + 1
                                        ));
                                        if attempt == 0 {
                                            let _ = browser_automation::scroll_page(600);
                                            tokio::time::sleep(tokio::time::Duration::from_secs(1))
                                                .await;
                                        }
                                    }
                                    Err(err) => {
                                        logs.push(format!("Search click failed: {}", err));
                                        if attempt == 0 {
                                            let _ = browser_automation::scroll_page(600);
                                            tokio::time::sleep(tokio::time::Duration::from_secs(1))
                                                .await;
                                        }
                                    }
                                }
                            }
                            if !clicked {
                                if let Ok(ctx) = browser_automation::get_page_context() {
                                    logs.push(format!("Page context: {}", ctx));
                                }
                                manual_required = true;
                                manual_steps.push(step.description.clone());
                            }
                            continue;
                        }
                    }
                    if let Some(field) = step.data.get("field").and_then(|v| v.as_str()) {
                        let mut filled = false;
                        for attempt in 0..2 {
                            match try_browser_autofill(plan, field) {
                                Ok(true) => {
                                    logs.push(format!("Auto fill succeeded for {}", field));
                                    filled = true;
                                    break;
                                }
                                Ok(false) => {
                                    logs.push(format!(
                                        "Auto fill skipped (no match) for {} (attempt {})",
                                        field,
                                        attempt + 1
                                    ));
                                    if attempt == 0 {
                                        let _ = browser_automation::scroll_page(400);
                                        tokio::time::sleep(tokio::time::Duration::from_secs(1))
                                            .await;
                                    }
                                }
                                Err(err) => {
                                    logs.push(format!(
                                        "Auto fill failed: {} (attempt {})",
                                        err,
                                        attempt + 1
                                    ));
                                    if attempt == 0 {
                                        let _ = browser_automation::scroll_page(400);
                                        tokio::time::sleep(tokio::time::Duration::from_secs(1))
                                            .await;
                                    }
                                }
                            }
                        }
                        if !filled {
                            if let Ok(ctx) = browser_automation::get_page_context() {
                                logs.push(format!("Page context: {}", ctx));
                            }
                            manual_required = true;
                            manual_steps.push(step.description.clone());
                        }
                        continue;
                    }
                    if let Some(value) = step.data.get("value").and_then(|v| v.as_str()) {
                        let mut driver = VisualDriver::new();
                        driver.add_step(SmartStep::new(
                            UiAction::Type(value.to_string()),
                            "Type value",
                        ));
                        if let Err(err) = driver.execute(None).await {
                            logs.push(format!("Auto input failed: {}", err));
                            manual_required = true;
                            manual_steps.push(step.description.clone());
                        } else {
                            logs.push("Auto input attempted".to_string());
                        }
                    } else if let Some(query) = step.data.get("query").and_then(|v| v.as_str()) {
                        let mut driver = VisualDriver::new();
                        driver.add_step(SmartStep::new(
                            UiAction::Type(query.to_string()),
                            "Type query",
                        ));
                        if let Err(err) = driver.execute(None).await {
                            logs.push(format!("Auto input failed: {}", err));
                            manual_required = true;
                            manual_steps.push(step.description.clone());
                        } else {
                            logs.push("Auto input attempted".to_string());
                        }
                    } else {
                        manual_required = true;
                        manual_steps.push(step.description.clone());
                        logs.push(format!(
                            "Manual input required for step '{}'",
                            step.description
                        ));
                    }
                } else {
                    manual_required = true;
                    manual_steps.push(step.description.clone());
                    logs.push(format!(
                        "Manual input required for step '{}'",
                        step.description
                    ));
                }
            }
            StepType::Approve => {
                let action = step
                    .data
                    .get("action")
                    .and_then(|v| v.as_str())
                    .unwrap_or("approve");
                let decision = approval_gate::evaluate_approval(action, plan);
                let approval_id = format!("appr:{}:{}:{}", plan.plan_id, step.step_id, idx + 1);
                logs.push(format!(
                    "Approval check: {} (risk {}, policy {})",
                    decision.status, decision.risk_level, decision.policy
                ));
                logs.push(format!(
                    "APPROVAL_CHECKPOINT|approval_id={}|step_id={}|status={}|risk={}|policy={}",
                    approval_id,
                    step.step_id,
                    decision.status,
                    decision.risk_level,
                    decision.policy
                ));
                if decision.requires_approval || decision.status == "denied" {
                    approval_context = Some(ApprovalContext {
                        approval_id: Some(approval_id.clone()),
                        action: action.to_string(),
                        message: decision.message.clone(),
                        risk_level: decision.risk_level.clone(),
                        policy: decision.policy.clone(),
                    });
                }
                if decision.status == "denied" {
                    logs.push("Execution blocked by policy".to_string());
                    blocked = true;
                    blocked_reason = "approval_policy_blocked".to_string();
                    break;
                }
                if decision.requires_approval {
                    approval_required = true;
                    logs.push("Approval required before continuing".to_string());
                } else {
                    logs.push("Approval auto-granted".to_string());
                }
            }
            StepType::Extract => {
                if let Some(summary) = try_extract_summary(plan) {
                    logs.push(format!("Summary: {}", summary));
                } else {
                    logs.push("No summary extracted".to_string());
                }
            }
            StepType::Screenshot => {}
        }

        if manual_required || approval_required || blocked {
            resume_from = Some(idx + 1);
            break;
        }
    }

    if blocked {
        logs.push(format!(
            "Execution stopped with blocked status (reason={})",
            blocked_reason
        ));
        push_focus_handoff_summary(&mut logs, &focus_handoff_state);
        push_run_attempt(&mut logs, "execution_end", "blocked", &blocked_reason);
        let resume_token = build_resume_token(plan, resume_from, &blocked_reason);
        if let Some(token) = resume_token.as_ref() {
            logs.push(format!("RESUME_TOKEN|status=blocked|token={}", token));
        }
        return ExecutionResult {
            status: "blocked".to_string(),
            logs,
            approval: approval_context,
            manual_steps,
            resume_from,
            resume_token,
        };
    }
    if approval_required {
        logs.push("Execution paused awaiting approval".to_string());
        push_focus_handoff_summary(&mut logs, &focus_handoff_state);
        push_run_attempt(
            &mut logs,
            "execution_end",
            "approval_required",
            "awaiting_approval",
        );
        let resume_token = build_resume_token(plan, resume_from, "approval_required");
        if let Some(token) = resume_token.as_ref() {
            logs.push(format!(
                "RESUME_TOKEN|status=approval_required|token={}",
                token
            ));
        }
        return ExecutionResult {
            status: "approval_required".to_string(),
            logs,
            approval: approval_context,
            manual_steps,
            resume_from,
            resume_token,
        };
    }
    if manual_required {
        logs.push("Execution paused for manual input".to_string());
        push_focus_handoff_summary(&mut logs, &focus_handoff_state);
        push_run_attempt(
            &mut logs,
            "execution_end",
            "manual_required",
            "awaiting_manual_input",
        );
        let resume_token = build_resume_token(plan, resume_from, "manual_required");
        if let Some(token) = resume_token.as_ref() {
            logs.push(format!(
                "RESUME_TOKEN|status=manual_required|token={}",
                token
            ));
        }
        return ExecutionResult {
            status: "manual_required".to_string(),
            logs,
            approval: approval_context,
            manual_steps,
            resume_from,
            resume_token,
        };
    }

    push_focus_handoff_summary(&mut logs, &focus_handoff_state);
    push_run_attempt(
        &mut logs,
        "execution_end",
        "completed",
        "all_steps_completed",
    );
    ExecutionResult {
        status: "completed".to_string(),
        logs,
        approval: approval_context,
        manual_steps,
        resume_from,
        resume_token: None,
    }
}

fn try_browser_autofill(plan: &Plan, field: &str) -> anyhow::Result<bool> {
    match plan.intent {
        crate::nl_automation::IntentType::FlightSearch => {
            if !matches!(field, "from" | "to" | "date_start" | "date_end") {
                return Ok(false);
            }
            let from = plan.slots.get("from").map(|v| v.as_str()).unwrap_or("");
            let to = plan.slots.get("to").map(|v| v.as_str()).unwrap_or("");
            let date_start = plan
                .slots
                .get("date_start")
                .map(|v| v.as_str())
                .unwrap_or("");
            let date_end = plan.slots.get("date_end").map(|v| v.as_str());
            browser_automation::fill_flight_fields(from, to, date_start, date_end)
        }
        crate::nl_automation::IntentType::ShoppingCompare => {
            let query = plan
                .slots
                .get("product_name")
                .map(|v| v.as_str())
                .unwrap_or("");
            browser_automation::fill_search_query(query)
        }
        crate::nl_automation::IntentType::FormFill => {
            if field != "form_profile" {
                return Ok(false);
            }
            let name = std::env::var("STEER_PROFILE_NAME").ok();
            let email = std::env::var("STEER_PROFILE_EMAIL").ok();
            let phone = std::env::var("STEER_PROFILE_PHONE").ok();
            let address = std::env::var("STEER_PROFILE_ADDRESS").ok();
            browser_automation::autofill_form(
                name.as_deref(),
                email.as_deref(),
                phone.as_deref(),
                address.as_deref(),
            )
        }
        crate::nl_automation::IntentType::GenericTask => Ok(false),
    }
}

fn try_extract_summary(plan: &Plan) -> Option<String> {
    match plan.intent {
        crate::nl_automation::IntentType::FlightSearch => {
            browser_automation::extract_flight_summary().ok()
        }
        crate::nl_automation::IntentType::ShoppingCompare => {
            browser_automation::extract_shopping_summary().ok()
        }
        _ => None,
    }
}

fn is_auto_step(data: &Value) -> bool {
    data.get("auto").and_then(|v| v.as_bool()).unwrap_or(false)
}

fn summary_for_plan(plan: &Plan) -> String {
    let slots = &plan.slots;
    match plan.intent {
        crate::nl_automation::IntentType::FlightSearch => {
            let from = slots
                .get("from")
                .cloned()
                .unwrap_or_else(|| "unknown".to_string());
            let to = slots
                .get("to")
                .cloned()
                .unwrap_or_else(|| "unknown".to_string());
            let date = slots
                .get("date_start")
                .cloned()
                .unwrap_or_else(|| "unknown".to_string());
            let budget = slots
                .get("budget_max")
                .cloned()
                .unwrap_or_else(|| "no budget".to_string());
            format!(
                "Summary: search flights {} → {} on {} (budget {})",
                from, to, date, budget
            )
        }
        crate::nl_automation::IntentType::ShoppingCompare => {
            let product = slots
                .get("product_name")
                .cloned()
                .unwrap_or_else(|| "unknown".to_string());
            let max_price = slots
                .get("price_max")
                .cloned()
                .unwrap_or_else(|| "no max".to_string());
            format!(
                "Summary: compare prices for {} (max {})",
                product, max_price
            )
        }
        crate::nl_automation::IntentType::FormFill => {
            let purpose = slots
                .get("form_purpose")
                .cloned()
                .unwrap_or_else(|| "unknown".to_string());
            format!("Summary: fill form for {}", purpose)
        }
        crate::nl_automation::IntentType::GenericTask => "Summary: need more details".to_string(),
    }
}

#[cfg(test)]
mod tests {
    #[cfg(target_os = "macos")]
    use super::parse_idle_ns_from_ioreg;

    #[cfg(target_os = "macos")]
    #[test]
    fn parse_idle_ns_from_decimal_line() {
        let sample = r#"    | | |   "HIDIdleTime" = 31574920250"#;
        let parsed = parse_idle_ns_from_ioreg(sample).expect("decimal HIDIdleTime should parse");
        assert_eq!(parsed, 31_574_920_250);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn parse_idle_ns_from_hex_line() {
        let sample = r#"    | | |   "HIDIdleTime" = 0x75BCD15"#;
        let parsed = parse_idle_ns_from_ioreg(sample).expect("hex HIDIdleTime should parse");
        assert_eq!(parsed, 123_456_789);
    }
}
