use crate::action_schema;
use crate::controller::actions::ActionRunner;
use crate::controller::heuristics;
use crate::controller::loop_detector::LoopDetector;
use crate::controller::supervisor::Supervisor;
use crate::db;
use crate::llm_gateway::LLMClient;
use crate::schema::EventEnvelope;
use crate::session_store::{Session, SessionStatus};
use crate::visual_driver::{SmartStep, VisualDriver};
use anyhow::Result;
use chrono::Utc;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::mpsc;
use uuid::Uuid;

pub struct Planner {
    pub llm: Arc<dyn LLMClient>,
    pub max_steps: usize,
    pub tx: Option<mpsc::Sender<String>>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct RunGoalOutcome {
    pub run_id: String,
    pub planner_complete: bool,
    pub execution_complete: bool,
    pub business_complete: bool,
    pub status: String,
    pub summary: Option<String>,
}

#[derive(Debug, Clone)]
struct RunGoalExecutionSummary {
    planner_complete: bool,
    execution_complete: bool,
    business_complete: bool,
    business_note: String,
    step_count: usize,
    failed_steps: usize,
    mail_send_required: bool,
    mail_send_confirmed: bool,
    notes_write_required: bool,
    notes_write_confirmed: bool,
    textedit_write_required: bool,
    textedit_write_confirmed: bool,
    textedit_save_required: bool,
    textedit_save_confirmed: bool,
}

#[derive(Debug, Default, Clone)]
struct RunGoalBusinessEvidence {
    mail_send_confirmed: bool,
    notes_write_confirmed: bool,
    textedit_write_confirmed: bool,
    textedit_save_confirmed: bool,
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
        history
            .iter()
            .any(|h| h.to_lowercase().contains(&needle_lower))
    }

    fn last_opened_app_from_history(history: &[String]) -> Option<String> {
        for entry in history.iter().rev() {
            if let Some(rest) = entry.strip_prefix("Opened app: ") {
                let app = rest.trim();
                if !app.is_empty() {
                    return Some(app.to_string());
                }
            }
        }
        None
    }

    fn goal_contains_any(goal_lower: &str, needles: &[&str]) -> bool {
        needles.iter().any(|needle| goal_lower.contains(needle))
    }

    fn goal_requires_mail_send(goal: &str) -> bool {
        let lower = goal.to_lowercase();
        let mentions_mail =
            lower.contains("mail") || lower.contains("메일") || lower.contains("이메일");
        let mentions_send =
            lower.contains("send") || lower.contains("보내") || lower.contains("발송");
        mentions_mail && mentions_send
    }

    fn goal_has_payload_tokens(goal: &str) -> bool {
        !Self::extract_quoted_fragments(goal).is_empty()
    }

    fn goal_has_write_signal(lower: &str) -> bool {
        Self::goal_contains_any(
            lower,
            &[
                "write",
                "작성",
                "입력",
                "붙여넣",
                "paste",
                "type",
                "append",
                "기록",
            ],
        )
    }

    fn goal_has_new_item_signal(lower: &str) -> bool {
        Self::goal_contains_any(
            lower,
            &[
                "new note",
                "new document",
                "새 메모",
                "새 문서",
                "cmd+n",
                "command+n",
            ],
        )
    }

    fn goal_requires_notes_write(goal: &str) -> bool {
        let lower = goal.to_lowercase();
        let mentions_notes = lower.contains("notes") || lower.contains("메모");
        mentions_notes
            && (Self::goal_has_write_signal(&lower)
                || Self::goal_has_new_item_signal(&lower)
                || Self::goal_has_payload_tokens(goal))
    }

    fn goal_requires_textedit_write(goal: &str) -> bool {
        let lower = goal.to_lowercase();
        let mentions_textedit = lower.contains("textedit") || lower.contains("텍스트에디트");
        mentions_textedit
            && (Self::goal_has_write_signal(&lower)
                || Self::goal_has_new_item_signal(&lower)
                || Self::goal_has_payload_tokens(goal))
    }

    fn goal_requires_textedit_save(goal: &str) -> bool {
        let lower = goal.to_lowercase();
        let mentions_textedit = lower.contains("textedit") || lower.contains("텍스트에디트");
        let mentions_save = Self::goal_contains_any(
            &lower,
            &[
                "save",
                "저장",
                "cmd+s",
                "command+s",
                "파일로 저장",
                "저장해",
            ],
        );
        mentions_textedit && mentions_save
    }

    fn step_data_has_proof(step: &crate::session_store::SessionStep, proof: &str) -> bool {
        step.data
            .as_ref()
            .and_then(|d| d.get("proof"))
            .and_then(|v| v.as_str())
            .map(|v| v == proof)
            .unwrap_or(false)
    }

    fn parse_app_context_from_step(step: &crate::session_store::SessionStep) -> Option<String> {
        let description = step.description.trim();
        if let Some(rest) = description.strip_prefix("Opened app: ") {
            let app = rest.trim();
            if !app.is_empty() {
                return Some(app.to_lowercase());
            }
        }
        if let Some(rest) = description.strip_prefix("Switched to app: ") {
            let app = rest.trim();
            if !app.is_empty() {
                return Some(app.to_lowercase());
            }
        }
        None
    }

    fn collect_business_evidence(session: &Session, history: &[String]) -> RunGoalBusinessEvidence {
        let mut evidence = RunGoalBusinessEvidence::default();
        let mut current_app = Self::last_opened_app_from_history(history).map(|a| a.to_lowercase());
        let mut textedit_context_seen = current_app.as_deref() == Some("textedit");

        for step in &session.steps {
            if step.status == "success" {
                if let Some(app) = Self::parse_app_context_from_step(step) {
                    if app == "textedit" {
                        textedit_context_seen = true;
                    }
                    current_app = Some(app);
                }
            }

            if step.status != "success" {
                continue;
            }

            let desc = step.description.to_lowercase();
            if Self::step_has_mail_send_confirmed(step) {
                evidence.mail_send_confirmed = true;
            }
            if Self::step_data_has_proof(step, "notes_write_text") || desc.contains("(notes body)")
            {
                evidence.notes_write_confirmed = true;
            }
            if Self::step_data_has_proof(step, "textedit_append_text")
                || desc.contains("(textedit body)")
            {
                evidence.textedit_write_confirmed = true;
                textedit_context_seen = true;
            }

            if matches!(step.action_type.as_str(), "type" | "paste") {
                match current_app.as_deref() {
                    Some("notes") => evidence.notes_write_confirmed = true,
                    Some("textedit") => {
                        evidence.textedit_write_confirmed = true;
                        textedit_context_seen = true;
                    }
                    _ => {}
                }
            }

            let is_save_shortcut = matches!(step.action_type.as_str(), "shortcut" | "key" | "save")
                && desc.contains("shortcut 's'")
                && desc.contains("command");
            if is_save_shortcut
                && (current_app.as_deref() == Some("textedit") || textedit_context_seen)
            {
                evidence.textedit_save_confirmed = true;
            }
        }

        if !evidence.mail_send_confirmed {
            evidence.mail_send_confirmed =
                Self::history_contains_case_insensitive(history, "mail send completed")
                    || Self::history_contains_case_insensitive(history, "(mail sent)");
        }
        if !evidence.notes_write_confirmed {
            evidence.notes_write_confirmed =
                Self::history_contains_case_insensitive(history, "(notes body)")
                    || (Self::history_contains_case_insensitive(history, "opened app: notes")
                        && Self::history_contains_case_insensitive(history, "typed '"));
        }
        if !evidence.textedit_write_confirmed {
            evidence.textedit_write_confirmed =
                Self::history_contains_case_insensitive(history, "(textedit body)")
                    || (Self::history_contains_case_insensitive(history, "opened app: textedit")
                        && Self::history_contains_case_insensitive(history, "typed '"));
        }
        if !evidence.textedit_save_confirmed {
            evidence.textedit_save_confirmed =
                Self::history_contains_case_insensitive(history, "opened app: textedit")
                    && Self::history_contains_shortcut(history, "s");
        }

        evidence
    }

    fn step_has_mail_send_confirmed(step: &crate::session_store::SessionStep) -> bool {
        if let Some(data) = &step.data {
            if data
                .get("send_status")
                .and_then(|v| v.as_str())
                .map(|v| v == "sent_confirmed")
                .unwrap_or(false)
            {
                return true;
            }
        }
        let desc = step.description.to_lowercase();
        desc.contains("mail send completed") || desc.contains("(mail sent)")
    }

    fn summarize_execution(
        goal: &str,
        session: &Session,
        history: &[String],
        planner_complete: bool,
    ) -> RunGoalExecutionSummary {
        let step_count = session.steps.len();
        let failed_steps = session
            .steps
            .iter()
            .filter(|s| s.status != "success")
            .count();
        let execution_complete = planner_complete && failed_steps == 0;
        let mail_send_required = Self::goal_requires_mail_send(goal);
        let notes_write_required = Self::goal_requires_notes_write(goal);
        let textedit_write_required = Self::goal_requires_textedit_write(goal);
        let textedit_save_required = Self::goal_requires_textedit_save(goal);
        let evidence = Self::collect_business_evidence(session, history);
        let mail_send_confirmed = evidence.mail_send_confirmed;
        let notes_write_confirmed = evidence.notes_write_confirmed;
        let textedit_write_confirmed = evidence.textedit_write_confirmed;
        let textedit_save_confirmed = evidence.textedit_save_confirmed;

        let (business_complete, business_note) = if !execution_complete {
            (
                false,
                format!(
                    "action execution had failures (failed_steps={} / total_steps={})",
                    failed_steps, step_count
                ),
            )
        } else {
            let mut missing_checks: Vec<&str> = Vec::new();
            if mail_send_required && !mail_send_confirmed {
                missing_checks.push("mail_send_confirmation");
            }
            if notes_write_required && !notes_write_confirmed {
                missing_checks.push("notes_write_evidence");
            }
            if textedit_write_required && !textedit_write_confirmed {
                missing_checks.push("textedit_write_evidence");
            }
            if textedit_save_required && !textedit_save_confirmed {
                missing_checks.push("textedit_save_confirmation");
            }

            if missing_checks.is_empty() {
                (
                    true,
                    "planner/execution/business checks passed from run evidence".to_string(),
                )
            } else {
                (
                    false,
                    format!("business evidence missing: {}", missing_checks.join(", ")),
                )
            }
        };

        RunGoalExecutionSummary {
            planner_complete,
            execution_complete,
            business_complete,
            business_note,
            step_count,
            failed_steps,
            mail_send_required,
            mail_send_confirmed,
            notes_write_required,
            notes_write_confirmed,
            textedit_write_required,
            textedit_write_confirmed,
            textedit_save_required,
            textedit_save_confirmed,
        }
    }

    fn is_textual_app(app: &str) -> bool {
        app.eq_ignore_ascii_case("Notes")
            || app.eq_ignore_ascii_case("TextEdit")
            || app.eq_ignore_ascii_case("Mail")
    }

    fn fallback_plan_from_goal(goal: &str, history: &[String]) -> Option<serde_json::Value> {
        let goal_lower = goal.to_lowercase();
        let wants_downloads = goal_lower.contains("downloads") || goal_lower.contains("다운로드");
        let apps_in_goal = Self::ordered_apps_in_goal(goal);
        let current_app = Self::last_opened_app_from_history(history);

        // Keep Finder->Downloads progression explicit before jumping to later apps.
        if wants_downloads {
            let finder_opened =
                Self::history_contains_case_insensitive(history, "Opened app: Finder");
            if !finder_opened {
                return Some(serde_json::json!({ "action": "open_app", "name": "Finder" }));
            }

            let downloads_opened = Self::history_contains_case_insensitive(
                history,
                "Opened Downloads folder in Finder",
            );
            if !downloads_opened {
                return Some(
                    serde_json::json!({ "action": "click_ref", "ref": "LeftSidebarDownloads", "app": "Finder" }),
                );
            }
        }

        if let Some(app_name) = current_app.as_deref() {
            let app_lower = app_name.to_lowercase();
            let mentions_new_item = Self::goal_contains_any(
                &goal_lower,
                &[
                    "cmd+n",
                    "command+n",
                    "새 메모",
                    "새 문서",
                    "새 이메일",
                    "new note",
                    "new document",
                    "new email",
                    "new draft",
                ],
            );
            if mentions_new_item
                && !Self::history_contains_shortcut(history, "n")
                && matches!(app_lower.as_str(), "notes" | "textedit" | "mail")
            {
                return Some(serde_json::json!({
                    "action": "shortcut",
                    "key": "n",
                    "modifiers": ["command"],
                    "app": app_name
                }));
            }

            if app_name.eq_ignore_ascii_case("Mail") {
                if let Some(subject) = Self::extract_mail_subject_from_goal(goal) {
                    if !Self::history_contains_case_insensitive(history, "(mail subject)") {
                        return Some(
                            serde_json::json!({ "action": "type", "text": subject, "app": "Mail" }),
                        );
                    }
                }
            }

            if Self::is_textual_app(app_name) {
                let mail_subject = Self::extract_mail_subject_from_goal(goal);
                for fragment in Self::extract_quoted_fragments(goal) {
                    let trimmed = fragment.trim();
                    let lower = trimmed.to_lowercase();
                    if trimmed.len() < 2
                        || lower.starts_with("cmd+")
                        || lower == "done"
                        || lower.starts_with("status:")
                    {
                        continue;
                    }

                    if !app_name.eq_ignore_ascii_case("Mail") {
                        if let Some(subject) = mail_subject.as_deref() {
                            if trimmed.eq_ignore_ascii_case(subject) {
                                continue;
                            }
                        }
                    }

                    if !Self::history_contains_case_insensitive(history, trimmed) {
                        return Some(serde_json::json!({
                            "action": "type",
                            "text": trimmed,
                            "app": app_name
                        }));
                    }
                }

                if Self::goal_contains_any(
                    &goal_lower,
                    &["select all", "전체 선택", "cmd+a", "command+a"],
                ) && !Self::history_contains_case_insensitive(history, "Selected all contents")
                {
                    return Some(serde_json::json!({ "action": "select_all", "app": app_name }));
                }

                if Self::goal_contains_any(&goal_lower, &["copy", "복사", "cmd+c", "command+c"])
                    && !Self::history_contains_case_insensitive(history, "Copied selection")
                {
                    return Some(serde_json::json!({ "action": "copy", "app": app_name }));
                }

                if Self::goal_contains_any(&goal_lower, &["paste", "붙여넣", "cmd+v", "command+v"])
                    && !Self::history_contains_case_insensitive(history, "Pasted")
                {
                    return Some(serde_json::json!({ "action": "paste", "app": app_name }));
                }
            }
        }

        for app in &apps_in_goal {
            let marker = format!("Opened app: {}", app);
            if !Self::history_contains_case_insensitive(history, &marker) {
                return Some(serde_json::json!({ "action": "open_app", "name": app }));
            }
        }

        if apps_in_goal.is_empty() {
            let fragments = Self::extract_quoted_fragments(goal)
                .into_iter()
                .filter(|frag| {
                    let trimmed = frag.trim();
                    let lower = trimmed.to_lowercase();
                    trimmed.len() >= 3
                        && !lower.starts_with("cmd+")
                        && lower != "done"
                        && !lower.starts_with("status:")
                })
                .collect::<Vec<_>>();
            if !fragments.is_empty() {
                if !Self::history_contains_case_insensitive(history, "Opened app: Notes") {
                    return Some(serde_json::json!({ "action": "open_app", "name": "Notes" }));
                }

                for fragment in fragments {
                    if !Self::history_contains_case_insensitive(history, &fragment) {
                        return Some(
                            serde_json::json!({ "action": "type", "text": fragment, "app": "Notes" }),
                        );
                    }
                }
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

    fn goal_has_explicit_sequence(goal: &str) -> bool {
        let lower = goal.to_lowercase();
        lower.contains(" then ")
            || lower.contains("다음")
            || lower.contains("후")
            || lower.contains("이후")
            || lower.contains("next")
    }

    fn goal_has_multi_app(goal: &str) -> bool {
        Self::ordered_apps_in_goal(goal).len() > 1
    }

    fn should_accept_text_flow_after_type(
        plan: &serde_json::Value,
        history: &[String],
        reason: &str,
        notes: &str,
    ) -> bool {
        let action = plan["action"].as_str().unwrap_or("");
        if !matches!(action, "select_all" | "copy" | "paste") {
            return false;
        }

        let recent_typed = history.iter().rev().take(10).any(|h| {
            let lower = h.to_lowercase();
            lower.starts_with("typed '")
                || lower.contains("typed \"")
                || lower.contains("typed ")
                || lower.contains("read_result:")
        });
        if !recent_typed {
            return false;
        }

        let text = format!("{} {}", reason.to_lowercase(), notes.to_lowercase());
        let confirmation_only_signals = [
            "confirmation",
            "confirm",
            "visible",
            "evidence",
            "fully entered",
            "full content",
            "입력",
            "보이",
            "확인",
        ];
        let has_confirmation_signal = confirmation_only_signals
            .iter()
            .any(|s| text.contains(&s.to_lowercase()));

        let hard_blockers = [
            "danger",
            "unsafe",
            "impossible",
            "not related",
            "does not relate",
            "wrong app",
            "before opening",
        ];
        let has_hard_blocker = hard_blockers.iter().any(|s| text.contains(s));

        has_confirmation_signal && !has_hard_blocker
    }

    fn should_accept_typing_after_new_item_shortcut(
        plan: &serde_json::Value,
        history: &[String],
        reason: &str,
        notes: &str,
    ) -> bool {
        if plan["action"].as_str() != Some("type") {
            return false;
        }

        let has_new_item_shortcut = history.iter().rev().take(8).any(|h| {
            let lower = h.to_lowercase();
            lower.contains("shortcut 'n'")
                && lower.contains("command")
                && (lower.contains("created new item") || lower.contains("shortcut"))
        });
        if !has_new_item_shortcut {
            return false;
        }

        let text = format!("{} {}", reason.to_lowercase(), notes.to_lowercase());
        let note_creation_signals = [
            "new note",
            "new item",
            "새 메모",
            "생성",
            "no evidence",
            "must ensure",
        ];
        let has_note_creation_signal = note_creation_signals
            .iter()
            .any(|s| text.contains(&s.to_lowercase()));

        let hard_blockers = [
            "danger",
            "unsafe",
            "impossible",
            "not related",
            "wrong app",
            "before opening",
        ];
        let has_hard_blocker = hard_blockers.iter().any(|s| text.contains(s));

        has_note_creation_signal && !has_hard_blocker
    }

    fn history_contains_shortcut(history: &[String], key: &str) -> bool {
        let needle = format!("shortcut '{}'", key.to_lowercase());
        history.iter().any(|h| h.to_lowercase().contains(&needle))
    }

    fn ordered_apps_in_goal(goal: &str) -> Vec<&'static str> {
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
        let mut found: Vec<(usize, &'static str)> = app_catalog
            .iter()
            .filter_map(|app| goal_lower.find(&app.to_lowercase()).map(|idx| (idx, *app)))
            .collect();
        found.sort_by_key(|(idx, _)| *idx);
        found.into_iter().map(|(_, app)| app).collect()
    }

    fn extract_quoted_fragments(text: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut in_double = false;
        let mut in_single = false;
        let mut buf = String::new();
        for ch in text.chars() {
            if ch == '"' && !in_single {
                if in_double {
                    let value = buf.trim().to_string();
                    if !value.is_empty() {
                        out.push(value);
                    }
                    buf.clear();
                    in_double = false;
                } else {
                    in_double = true;
                    buf.clear();
                }
                continue;
            }
            if ch == '\'' && !in_double {
                if in_single {
                    let value = buf.trim().to_string();
                    if !value.is_empty() {
                        out.push(value);
                    }
                    buf.clear();
                    in_single = false;
                } else {
                    in_single = true;
                    buf.clear();
                }
                continue;
            }
            if in_double || in_single {
                buf.push(ch);
            }
        }
        out
    }

    fn extract_mail_subject_from_goal(goal: &str) -> Option<String> {
        let lower = goal.to_lowercase();
        for marker in ["제목", "subject", "title"] {
            if let Some(idx) = lower.find(marker) {
                let rest = &goal[idx + marker.len()..];
                for frag in Self::extract_quoted_fragments(rest) {
                    let f = frag.trim();
                    if f.len() >= 2 {
                        return Some(f.to_string());
                    }
                }
            }
        }

        if lower.contains("mail") || lower.contains("메일") {
            for frag in Self::extract_quoted_fragments(goal) {
                let f = frag.trim();
                let lf = f.to_lowercase();
                if f.len() < 2 {
                    continue;
                }
                if lf.contains("cmd+") || lf == "done" || lf.starts_with("status:") {
                    continue;
                }
                return Some(f.to_string());
            }
        }
        None
    }

    fn history_has_mail_subject(history: &[String]) -> bool {
        history.iter().any(|h| {
            let lower = h.to_lowercase();
            lower.contains("(mail subject)") || lower.contains("mail subject")
        })
    }

    fn last_opened_app(history: &[String]) -> Option<String> {
        for entry in history.iter().rev() {
            if let Some(rest) = entry.strip_prefix("Opened app: ") {
                let app = rest.trim();
                if !app.is_empty() {
                    return Some(app.to_string());
                }
            }
        }
        None
    }

    fn has_recent_created_item(history: &[String]) -> bool {
        history
            .iter()
            .rev()
            .take(8)
            .any(|h| h.to_lowercase().contains("created new item"))
    }

    fn maybe_rewrite_mail_subject_before_paste(
        goal: &str,
        history: &[String],
        plan: &mut serde_json::Value,
    ) {
        let action = plan["action"].as_str().unwrap_or("");
        if !matches!(action, "paste" | "done" | "shortcut") {
            return;
        }

        let in_mail_context =
            match Self::last_opened_app(history) {
                Some(app) => app.eq_ignore_ascii_case("Mail"),
                None => false,
            } || Self::history_contains_case_insensitive(history, "Opened app: Mail");
        if !in_mail_context {
            return;
        }

        if Self::history_has_mail_subject(history) {
            return;
        }

        let is_cmd_n_shortcut = action == "shortcut"
            && plan["key"].as_str().map(|k| k.eq_ignore_ascii_case("n")) == Some(true)
            && plan["modifiers"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .any(|m| m.as_str().unwrap_or("").eq_ignore_ascii_case("command"))
                })
                .unwrap_or(false);
        if action == "shortcut" && !is_cmd_n_shortcut {
            return;
        }

        if let Some(subject) = Self::extract_mail_subject_from_goal(goal) {
            *plan = serde_json::json!({
                "action": "type",
                "app": "Mail",
                "text": subject
            });
            println!("   🔁 Rewrote action to set Mail subject before paste.");
        }
    }

    fn maybe_rewrite_shortcut_to_next_app(
        goal: &str,
        history: &[String],
        plan: &mut serde_json::Value,
    ) {
        if plan["action"].as_str() != Some("shortcut") {
            return;
        }
        if plan.get("app").and_then(|v| v.as_str()).is_some() {
            return;
        }

        let key = plan["key"].as_str().unwrap_or("").to_lowercase();
        let has_command = plan["modifiers"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .any(|m| m.as_str().unwrap_or("").eq_ignore_ascii_case("command"))
            })
            .unwrap_or(false);
        if key != "n" || !has_command {
            return;
        }

        let Some(last_opened) = Self::last_opened_app(history) else {
            return;
        };
        if !Self::has_recent_created_item(history) {
            return;
        }

        let ordered = Self::ordered_apps_in_goal(goal);
        let mut current_idx: Option<usize> = None;
        for (idx, app) in ordered.iter().enumerate() {
            if app.eq_ignore_ascii_case(&last_opened) {
                current_idx = Some(idx);
                break;
            }
        }
        let Some(idx) = current_idx else {
            return;
        };
        let Some(next_app) = ordered.get(idx + 1) else {
            return;
        };

        let opened_marker = format!("Opened app: {}", next_app);
        if Self::history_contains_case_insensitive(history, &opened_marker) {
            return;
        }

        *plan = serde_json::json!({ "action": "open_app", "name": next_app });
        println!(
            "   🔁 Rewrote repeated Cmd+N shortcut to next app transition: {}",
            next_app
        );
    }

    fn can_force_done_for_simple_goal(
        goal: &str,
        plan: &serde_json::Value,
        history: &[String],
    ) -> bool {
        if plan["action"].as_str() != Some("done") {
            return false;
        }

        let goal_lower = goal.to_lowercase();
        let is_note_creation_goal = goal_lower.contains("notes")
            && (goal_lower.contains("새 메모")
                || goal_lower.contains("new note")
                || goal_lower.contains("new memo"));
        if !is_note_creation_goal {
            return false;
        }

        let complex_markers = [
            "입력",
            "type",
            "복사",
            "copy",
            "붙여",
            "paste",
            "전송",
            "mail",
            "calculator",
            "textedit",
            "finder",
            "safari",
            "google",
            "검색",
        ];
        if complex_markers.iter().any(|m| goal_lower.contains(m)) {
            return false;
        }

        let opened_notes = Self::history_contains_case_insensitive(history, "Opened app: Notes");
        let made_new_note = Self::history_contains_shortcut(history, "n");
        opened_notes && made_new_note
    }

    fn env_truthy(name: &str) -> bool {
        matches!(
            std::env::var(name).as_deref(),
            Ok("1") | Ok("true") | Ok("TRUE") | Ok("yes") | Ok("YES")
        )
    }

    fn env_truthy_default(name: &str, default_value: bool) -> bool {
        match std::env::var(name) {
            Ok(raw) => matches!(raw.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"),
            Err(_) => default_value,
        }
    }

    fn record_fallback_action(history: &mut Vec<String>, reason: &str, plan: &serde_json::Value) {
        println!("   🧯 Fallback action [{}]: {}", reason, plan);
        history.push(format!("FALLBACK_ACTION: {} => {}", reason, plan));
    }

    fn env_usize(name: &str, default_value: usize) -> usize {
        std::env::var(name)
            .ok()
            .and_then(|raw| raw.parse::<usize>().ok())
            .filter(|v| *v > 0)
            .unwrap_or(default_value)
    }

    fn sanitize_filename_token(input: &str) -> String {
        let mut out = String::with_capacity(input.len());
        for ch in input.chars() {
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
                out.push(ch);
            } else {
                out.push('_');
            }
        }
        let collapsed = out.trim_matches('_');
        let mut final_token = if collapsed.is_empty() {
            "node".to_string()
        } else {
            collapsed.to_string()
        };
        if final_token.len() > 48 {
            final_token.truncate(48);
        }
        final_token
    }

    fn capture_node_evidence(
        base_dir: &Path,
        seq: usize,
        step: usize,
        phase: &str,
        plan: &serde_json::Value,
        note: &str,
    ) -> Option<PathBuf> {
        let action = plan["action"].as_str().unwrap_or("unknown");
        let action_token = Self::sanitize_filename_token(action);
        let phase_token = Self::sanitize_filename_token(phase);
        let note_token = Self::sanitize_filename_token(note);
        let file_name = format!(
            "node_{:03}_step_{:02}_{}_{}_{}.png",
            seq, step, action_token, phase_token, note_token
        );
        let full_path = base_dir.join(file_name);

        let status = std::process::Command::new("screencapture")
            .arg("-x")
            .arg(&full_path)
            .status();

        match status {
            Ok(s) if s.success() => {
                let front_app = crate::tool_chaining::CrossAppBridge::get_frontmost_app()
                    .unwrap_or_else(|_| "unknown".to_string());
                println!(
                    "   📸 Node evidence: {} | step={} action={} phase={} front_app={} note={}",
                    full_path.display(),
                    step,
                    action,
                    phase,
                    front_app,
                    note
                );
                Some(full_path)
            }
            Ok(s) => {
                println!(
                    "   ⚠️ Node evidence capture failed (exit={:?}) for step={} action={}",
                    s.code(),
                    step,
                    action
                );
                None
            }
            Err(e) => {
                println!(
                    "   ⚠️ Node evidence capture error for step={} action={}: {}",
                    step, action, e
                );
                None
            }
        }
    }

    pub fn new(llm: Arc<dyn LLMClient>, tx: Option<mpsc::Sender<String>>) -> Self {
        Self {
            llm,
            max_steps: Self::env_usize("STEER_MAX_STEPS", 30),
            tx,
        }
    }

    pub async fn run_goal_tracked(
        &self,
        goal: &str,
        session_key: Option<&str>,
    ) -> Result<RunGoalOutcome> {
        let run_id = format!(
            "surf_{}_{}",
            Utc::now().format("%Y%m%d_%H%M%S"),
            Uuid::new_v4().simple()
        );
        let normalized_goal = goal.trim();
        let _ = db::create_task_run(&run_id, "surf_goal", normalized_goal, "running");
        let _ = db::record_task_stage_run(
            &run_id,
            "planner",
            1,
            "running",
            Some("planner.run_goal started"),
        );

        match self.run_goal_with_summary(goal, session_key).await {
            Ok(exec_summary) => {
                let planner_complete = exec_summary.planner_complete;
                let execution_complete = exec_summary.execution_complete;
                let business_complete = exec_summary.business_complete;
                let status = if business_complete {
                    "business_completed"
                } else {
                    "business_failed"
                };
                let summary = if business_complete {
                    Some("surf goal execution and business checks completed".to_string())
                } else {
                    Some(format!(
                        "surf goal completed planner/execution but business check failed: {}",
                        exec_summary.business_note
                    ))
                };
                let details = serde_json::json!({
                    "source": "planner.run_goal_tracked",
                    "goal": normalized_goal,
                    "status": status,
                    "business_complete": business_complete,
                    "business_note": exec_summary.business_note,
                    "step_count": exec_summary.step_count,
                    "failed_steps": exec_summary.failed_steps,
                    "mail_send_required": exec_summary.mail_send_required,
                    "mail_send_confirmed": exec_summary.mail_send_confirmed,
                    "notes_write_required": exec_summary.notes_write_required,
                    "notes_write_confirmed": exec_summary.notes_write_confirmed,
                    "textedit_write_required": exec_summary.textedit_write_required,
                    "textedit_write_confirmed": exec_summary.textedit_write_confirmed,
                    "textedit_save_required": exec_summary.textedit_save_required,
                    "textedit_save_confirmed": exec_summary.textedit_save_confirmed
                })
                .to_string();

                let _ = db::record_task_stage_run(
                    &run_id,
                    "planner",
                    1,
                    "completed",
                    Some("planner produced done"),
                );
                let _ = db::record_task_stage_assertion(
                    &run_id,
                    "planner",
                    "planner_complete",
                    "true",
                    "true",
                    true,
                    Some("Goal completed by planner"),
                );
                let _ = db::record_task_stage_run(
                    &run_id,
                    "execution",
                    2,
                    if execution_complete {
                        "completed"
                    } else {
                        "failed"
                    },
                    Some(&format!(
                        "step_count={} failed_steps={}",
                        exec_summary.step_count, exec_summary.failed_steps
                    )),
                );
                let _ = db::record_task_stage_assertion(
                    &run_id,
                    "execution",
                    "execution_complete",
                    "true",
                    if execution_complete { "true" } else { "false" },
                    execution_complete,
                    Some("All recorded action steps must be successful"),
                );
                let _ = db::record_task_stage_run(
                    &run_id,
                    "business",
                    3,
                    if business_complete {
                        "completed"
                    } else {
                        "failed"
                    },
                    Some(&exec_summary.business_note),
                );
                let _ = db::record_task_stage_assertion(
                    &run_id,
                    "business",
                    "business_complete",
                    "true",
                    if business_complete { "true" } else { "false" },
                    business_complete,
                    Some(&format!(
                        "mail_send_required={} mail_send_confirmed={} notes_write_required={} notes_write_confirmed={} textedit_write_required={} textedit_write_confirmed={} textedit_save_required={} textedit_save_confirmed={}",
                        exec_summary.mail_send_required,
                        exec_summary.mail_send_confirmed,
                        exec_summary.notes_write_required,
                        exec_summary.notes_write_confirmed,
                        exec_summary.textedit_write_required,
                        exec_summary.textedit_write_confirmed,
                        exec_summary.textedit_save_required,
                        exec_summary.textedit_save_confirmed
                    )),
                );
                let _ = db::update_task_run_outcome(
                    &run_id,
                    planner_complete,
                    execution_complete,
                    business_complete,
                    status,
                    summary.as_deref(),
                    Some(&details),
                );

                Ok(RunGoalOutcome {
                    run_id,
                    planner_complete,
                    execution_complete,
                    business_complete,
                    status: status.to_string(),
                    summary,
                })
            }
            Err(e) => {
                let error_text = e.to_string();
                let summary = Some(format!("surf goal failed: {}", error_text));
                let details = serde_json::json!({
                    "source": "planner.run_goal_tracked",
                    "goal": normalized_goal,
                    "status": "failed",
                    "error": error_text
                })
                .to_string();

                let _ = db::record_task_stage_run(&run_id, "planner", 1, "failed", Some(&details));
                let _ = db::record_task_stage_assertion(
                    &run_id,
                    "planner",
                    "planner_complete",
                    "true",
                    "false",
                    false,
                    Some("planner did not reach done"),
                );
                let _ = db::record_task_stage_run(
                    &run_id,
                    "execution",
                    2,
                    "failed",
                    Some("execution/biz completion unavailable due planner failure"),
                );
                let _ = db::record_task_stage_assertion(
                    &run_id,
                    "execution",
                    "execution_complete",
                    "true",
                    "false",
                    false,
                    Some("run_goal returned error"),
                );
                let _ = db::record_task_stage_run(
                    &run_id,
                    "business",
                    3,
                    "failed",
                    Some("business completion failed"),
                );
                let _ = db::record_task_stage_assertion(
                    &run_id,
                    "business",
                    "business_complete",
                    "true",
                    "false",
                    false,
                    Some("run_goal returned error"),
                );
                let _ = db::update_task_run_outcome(
                    &run_id,
                    false,
                    false,
                    false,
                    "business_failed",
                    summary.as_deref(),
                    Some(&details),
                );

                Err(anyhow::anyhow!("{} (run_id={})", e, run_id))
            }
        }
    }

    pub async fn run_goal(&self, goal: &str, session_key: Option<&str>) -> Result<()> {
        let _ = self.run_goal_with_summary(goal, session_key).await?;
        Ok(())
    }

    async fn run_goal_with_summary(
        &self,
        goal: &str,
        session_key: Option<&str>,
    ) -> Result<RunGoalExecutionSummary> {
        println!("🌊 Starting Planned Surf: '{}'", goal);
        let scenario_mode = Self::scenario_mode_enabled();
        let test_context = Self::env_truthy("STEER_TEST_MODE") || Self::env_truthy("CI");
        let deterministic_fallback_requested =
            Self::env_truthy("STEER_ALLOW_DETERMINISTIC_FALLBACK");
        let review_loop_override_requested = Self::env_truthy("STEER_ALLOW_REVIEW_LOOP_OVERRIDE");
        if deterministic_fallback_requested && !test_context {
            return Err(anyhow::anyhow!(
                "STEER_ALLOW_DETERMINISTIC_FALLBACK is test-only (requires STEER_TEST_MODE=1 or CI=1)."
            ));
        }
        if review_loop_override_requested && !test_context {
            return Err(anyhow::anyhow!(
                "STEER_ALLOW_REVIEW_LOOP_OVERRIDE is test-only (requires STEER_TEST_MODE=1 or CI=1)."
            ));
        }
        let allow_deterministic_fallback = deterministic_fallback_requested && test_context;
        let allow_review_loop_override =
            review_loop_override_requested && allow_deterministic_fallback;
        let require_primary_planner =
            Self::env_truthy_default("STEER_REQUIRE_PRIMARY_PLANNER", true);
        let allow_scenario_mode = Self::env_truthy("STEER_ALLOW_SCENARIO_MODE");
        if scenario_mode && require_primary_planner && !allow_scenario_mode {
            return Err(anyhow::anyhow!(
                "Scenario mode fallback is disabled by policy (set STEER_ALLOW_SCENARIO_MODE=1 only for explicit test runs)."
            ));
        }
        let mut node_capture_enabled = Self::env_truthy("STEER_NODE_CAPTURE");
        let node_capture_all = Self::env_truthy("STEER_NODE_CAPTURE_ALL");
        let mut node_capture_seq: usize = 0;
        let mut last_opened_app: Option<String> = None;
        let mut node_capture_dir: Option<PathBuf> = None;

        if node_capture_enabled {
            let dir = std::env::var("STEER_NODE_CAPTURE_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|_| {
                    PathBuf::from(format!(
                        "scenario_results/node_evidence_{}",
                        Utc::now().format("%Y%m%d_%H%M%S")
                    ))
                });

            match std::fs::create_dir_all(&dir) {
                Ok(_) => {
                    println!("📸 Node capture enabled: {}", dir.display());
                    node_capture_dir = Some(dir);
                }
                Err(e) => {
                    println!(
                        "⚠️ Node capture disabled: failed to create dir '{}': {}",
                        dir.display(),
                        e
                    );
                    node_capture_enabled = false;
                }
            }
        }

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
            let attempt = plan_attempts
                .entry(plan_key.clone())
                .and_modify(|v| *v += 1)
                .or_insert(1);

            // Preflight: close blocking dialogs
            if heuristics::try_close_front_dialog() {
                history.push("Closed blocking dialog".to_string());
                continue;
            }

            // 2. Plan (Think)
            let retry_config = crate::retry_logic::RetryConfig::default();
            let mut history_with_context = history.clone();
            if *attempt > 1 || consecutive_failures > 0 {
                let last_action = last_action_by_plan
                    .get(&plan_key)
                    .cloned()
                    .unwrap_or_else(|| "unknown".to_string());
                let last_error = history
                    .iter()
                    .rev()
                    .find(|h| h.starts_with("FAILED") || h.starts_with("BLOCKED"))
                    .cloned()
                    .unwrap_or_else(|| "none".to_string());
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
                    self.llm
                        .plan_vision_step(goal, &image_b64, &history_with_context)
                        .await
                })
                .await?
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
            Self::maybe_rewrite_shortcut_to_next_app(goal, &history, &mut plan);
            Self::maybe_rewrite_mail_subject_before_paste(goal, &history, &mut plan);

            if scenario_mode {
                if let Some(fallback_plan) = Self::fallback_plan_from_goal(goal, &history) {
                    Self::record_fallback_action(&mut history, "scenario_mode", &fallback_plan);
                    plan = fallback_plan;
                }
            } else {
                // 3. Supervisor Check
                let supervisor_decision =
                    crate::retry_logic::with_retry(&retry_config, "Supervisor", || async {
                        Supervisor::consult(&*self.llm, goal, &plan, &history).await
                    })
                    .await?;

                println!(
                    "   🕵️ Supervisor: {} ({})",
                    supervisor_decision.action, supervisor_decision.reason
                );

                let mut supervisor_action = supervisor_decision.action.clone();
                if supervisor_action == "review"
                    && Self::can_force_done_for_simple_goal(goal, &plan, &history)
                {
                    println!(
                        "   ✅ Force-complete override: simple note creation goal already satisfied."
                    );
                    supervisor_action = "accept".to_string();
                }

                if supervisor_action == "review"
                    && Self::should_accept_text_flow_after_type(
                        &plan,
                        &history,
                        &supervisor_decision.reason,
                        &supervisor_decision.notes,
                    )
                {
                    println!(
                        "   ✅ Review override: proceeding with text-flow action after prior typing evidence."
                    );
                    supervisor_action = "accept".to_string();
                }

                if supervisor_action == "review"
                    && Self::should_accept_typing_after_new_item_shortcut(
                        &plan,
                        &history,
                        &supervisor_decision.reason,
                        &supervisor_decision.notes,
                    )
                {
                    println!(
                        "   ✅ Review override: allowing typing step after Cmd+N creation evidence."
                    );
                    supervisor_action = "accept".to_string();
                }

                if supervisor_action == "review"
                    && !Self::goal_has_multi_app(goal)
                    && !Self::goal_has_explicit_sequence(goal)
                    && allow_deterministic_fallback
                    && Self::should_relax_review(
                        &supervisor_decision.reason,
                        &supervisor_decision.notes,
                    )
                {
                    if let Some(fallback_plan) = Self::fallback_plan_from_goal(goal, &history) {
                        Self::record_fallback_action(
                            &mut history,
                            "relaxed_review",
                            &fallback_plan,
                        );
                        plan = fallback_plan;
                        supervisor_action = "accept".to_string();
                    }
                }

                if supervisor_action == "review" {
                    let recent_rejections = history
                        .iter()
                        .rev()
                        .take(16)
                        .filter(|h| h.starts_with("PLAN_REJECTED:"))
                        .count();
                    if recent_rejections >= 4 {
                        let review_text = format!(
                            "{} {}",
                            supervisor_decision.reason.to_lowercase(),
                            supervisor_decision.notes.to_lowercase()
                        );
                        let hard_blockers = [
                            "danger",
                            "unsafe",
                            "impossible",
                            "not related",
                            "does not relate",
                            "wrong app",
                        ];
                        let has_hard_blocker =
                            hard_blockers.iter().any(|s| review_text.contains(s));
                        let has_notes_content_issue = review_text.contains("notes")
                            && (review_text.contains("content")
                                || review_text.contains("exact")
                                || review_text.contains("불일치"));

                        if !has_hard_blocker && !has_notes_content_issue {
                            if allow_deterministic_fallback {
                                if allow_review_loop_override {
                                    if let Some(loop_break_plan) =
                                        Self::fallback_plan_from_goal(goal, &history)
                                    {
                                        Self::record_fallback_action(
                                            &mut history,
                                            &format!("review_loop_{}rejections", recent_rejections),
                                            &loop_break_plan,
                                        );
                                        plan = loop_break_plan;
                                        supervisor_action = "accept".to_string();
                                    } else {
                                        let action_name =
                                            plan["action"].as_str().unwrap_or("unknown");
                                        if matches!(
                                            action_name,
                                            "open_app"
                                                | "open_url"
                                                | "shortcut"
                                                | "type"
                                                | "paste"
                                                | "copy"
                                                | "select_all"
                                                | "read"
                                                | "read_clipboard"
                                                | "click_visual"
                                        ) {
                                            println!(
                                                "   🔁 Review-loop override: forcing '{}' after {} rejections.",
                                                action_name, recent_rejections
                                            );
                                            supervisor_action = "accept".to_string();
                                        }
                                    }
                                } else {
                                    history.push(
                                        "FALLBACK_BLOCKED: review-loop override requires STEER_ALLOW_REVIEW_LOOP_OVERRIDE=1"
                                            .to_string(),
                                    );
                                    let msg = "Supervisor review loop: deterministic override disabled by policy";
                                    return Err(anyhow::anyhow!(msg));
                                }
                            } else {
                                history.push(
                                    "FALLBACK_BLOCKED: review-loop deterministic fallback disabled"
                                        .to_string(),
                                );
                                let msg = "Supervisor review loop: deterministic fallback disabled by policy";
                                return Err(anyhow::anyhow!(msg));
                            }
                        }
                    }
                }

                if supervisor_action == "escalate" {
                    let recent_rejections = history
                        .iter()
                        .rev()
                        .take(16)
                        .filter(|h| h.starts_with("PLAN_REJECTED:"))
                        .count();
                    let reason_lc = supervisor_decision.reason.to_lowercase();
                    let notes_lc = supervisor_decision.notes.to_lowercase();
                    let repeated_content_escalation = (reason_lc.contains("repeated")
                        || notes_lc.contains("repeated"))
                        && (reason_lc.contains("content")
                            || notes_lc.contains("content")
                            || reason_lc.contains("notes")
                            || notes_lc.contains("notes"));
                    if repeated_content_escalation && recent_rejections >= 3 {
                        println!(
                            "   🔁 Escalation override: retrying content-repair path before hard fail."
                        );
                        supervisor_action = "review".to_string();
                    }
                }

                match supervisor_action.as_str() {
                    "accept" => { /* Proceed */ }
                    "review" => {
                        history.push(format!("PLAN_REJECTED: {}", supervisor_decision.notes));
                        continue;
                    }
                    "escalate" => {
                        let msg = format!("Supervisor escalated: {}", supervisor_decision.reason);
                        println!("      🚨 {}", msg);
                        return Err(anyhow::anyhow!(msg));
                    }
                    _ => {}
                }
            }

            // 4. Anti-Loop Check
            let action_str = plan.to_string();
            if LoopDetector::detect_action_loop(&action_history, &action_str) {
                println!(
                    "   🔄 LOOP DETECTED. Recording context and retrying with same action family."
                );
                history.push(format!("LOOP_DETECTED: repeated_plan={}", action_str));
            }
            action_history.push(action_str.clone());
            last_action_by_plan.insert(
                plan_key.clone(),
                plan["action"].as_str().unwrap_or("unknown").to_string(),
            );

            if plan["action"].as_str() == Some("done") {
                if node_capture_enabled {
                    if let Some(dir) = node_capture_dir.as_ref() {
                        node_capture_seq += 1;
                        let _ = Self::capture_node_evidence(
                            dir,
                            node_capture_seq,
                            i,
                            "goal_done",
                            &plan,
                            "planner_done",
                        );
                    }
                }
                println!("✅ Goal completed by planner.");
                goal_completed = true;
                break;
            }

            // 5. Execute via ActionRunner
            println!("   🚀 Executing Action...");
            let execute_result = ActionRunner::execute(
                &plan,
                &mut VisualDriver::new(), // In real scenario, might want to reuse driver or pass it
                Some(&*self.llm),
                &mut session_steps,
                &mut session,
                &mut history,
                &mut consecutive_failures,
                &mut last_read_number,
                goal,
            )
            .await;

            if let Err(e) = &execute_result {
                println!("   ❌ Execution Error: {}", e);
                // logic to handle specific errors or break
            }

            if node_capture_enabled {
                if let Some(dir) = node_capture_dir.as_ref() {
                    let action_name = plan["action"].as_str().unwrap_or("unknown");
                    let mut should_capture = node_capture_all;
                    let mut phase = "post_action";
                    let mut note = "action_executed".to_string();

                    if action_name == "open_app" {
                        should_capture = true;
                        phase = "app_node";
                        let current_app = plan["name"]
                            .as_str()
                            .or_else(|| plan["app"].as_str())
                            .unwrap_or("unknown")
                            .to_string();
                        note = if let Some(prev_app) = last_opened_app.as_ref() {
                            if prev_app.eq_ignore_ascii_case(&current_app) {
                                format!("app_reopen_{}", current_app)
                            } else {
                                format!("transition_{}_to_{}", prev_app, current_app)
                            }
                        } else {
                            format!("app_entry_{}", current_app)
                        };
                        last_opened_app = Some(current_app);
                    } else if execute_result.is_err() {
                        should_capture = true;
                        phase = "execution_error";
                        note = "action_failed".to_string();
                    }

                    if should_capture {
                        node_capture_seq += 1;
                        let _ = Self::capture_node_evidence(
                            dir,
                            node_capture_seq,
                            i,
                            phase,
                            &plan,
                            &note,
                        );
                    }
                }
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
            let summary = Self::summarize_execution(goal, &session, &history, true);
            session.status = if summary.business_complete {
                SessionStatus::Completed
            } else {
                SessionStatus::Failed
            };
            let _ = crate::session_store::save_session(&session);
            return Ok(summary);
        }

        session.status = SessionStatus::Failed;
        let _ = crate::session_store::save_session(&session);
        Err(anyhow::anyhow!(
            "Planner stopped without completion (max steps reached or unresolved review loop)."
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::Planner;
    use crate::session_store::Session;

    fn base_session(goal: &str) -> Session {
        Session::new(goal, Some("planner_test"))
    }

    #[test]
    fn summarize_execution_success_for_non_mail_goal() {
        let goal = "Notes를 열고 간단한 메모를 작성하고 done 하세요.";
        let mut session = base_session(goal);
        session.add_step("open_app", "Opened app: Notes", "success", None);
        session.add_step("type", "Typed '회의 준비'", "success", None);
        let history = vec![
            "Opened app: Notes".to_string(),
            "Typed '회의 준비'".to_string(),
        ];

        let summary = Planner::summarize_execution(goal, &session, &history, true);
        assert!(summary.planner_complete);
        assert!(summary.execution_complete);
        assert!(summary.business_complete);
    }

    #[test]
    fn summarize_execution_fails_if_any_step_failed() {
        let goal = "TextEdit를 열고 문서를 작성하세요.";
        let mut session = base_session(goal);
        session.add_step("open_app", "Opened app: TextEdit", "success", None);
        session.add_step("type", "Type failed: blocked by dialog", "failed", None);
        let history = vec!["Opened app: TextEdit".to_string()];

        let summary = Planner::summarize_execution(goal, &session, &history, true);
        assert!(summary.planner_complete);
        assert!(!summary.execution_complete);
        assert!(!summary.business_complete);
    }

    #[test]
    fn summarize_execution_requires_mail_send_confirmation() {
        let goal = "Mail을 열고 이메일을 보내세요.";
        let mut session = base_session(goal);
        session.add_step("open_app", "Opened app: Mail", "success", None);
        session.add_step("type", "Typed 'subject'", "success", None);
        let history = vec![
            "Opened app: Mail".to_string(),
            "Typed 'subject'".to_string(),
        ];

        let summary = Planner::summarize_execution(goal, &session, &history, true);
        assert!(summary.mail_send_required);
        assert!(!summary.mail_send_confirmed);
        assert!(!summary.business_complete);
    }

    #[test]
    fn summarize_execution_passes_when_mail_send_confirmed() {
        let goal = "Mail로 보고서를 보내세요.";
        let mut session = base_session(goal);
        session.add_step("open_app", "Opened app: Mail", "success", None);
        session.add_step(
            "mail_send",
            "Mail send completed",
            "success",
            Some(serde_json::json!({"send_status": "sent_confirmed"})),
        );
        let history = vec![
            "Opened app: Mail".to_string(),
            "Mail send completed".to_string(),
        ];

        let summary = Planner::summarize_execution(goal, &session, &history, true);
        assert!(summary.mail_send_required);
        assert!(summary.mail_send_confirmed);
        assert!(summary.business_complete);
    }

    #[test]
    fn summarize_execution_requires_textedit_save_when_goal_mentions_save() {
        let goal = "TextEdit에서 문서를 작성하고 저장하세요.";
        let mut session = base_session(goal);
        session.add_step("open_app", "Opened app: TextEdit", "success", None);
        session.add_step("type", "Typed 'status: in-progress'", "success", None);
        let history = vec![
            "Opened app: TextEdit".to_string(),
            "Typed 'status: in-progress'".to_string(),
        ];

        let summary = Planner::summarize_execution(goal, &session, &history, true);
        assert!(summary.textedit_write_required);
        assert!(summary.textedit_write_confirmed);
        assert!(summary.textedit_save_required);
        assert!(!summary.textedit_save_confirmed);
        assert!(!summary.business_complete);
    }

    #[test]
    fn summarize_execution_passes_when_textedit_save_confirmed() {
        let goal = "TextEdit에서 문서를 작성하고 저장하세요.";
        let mut session = base_session(goal);
        session.add_step("open_app", "Opened app: TextEdit", "success", None);
        session.add_step(
            "type",
            "Typed 'status: in-progress' (textedit body)",
            "success",
            Some(serde_json::json!({"proof": "textedit_append_text"})),
        );
        session.add_step("shortcut", "Shortcut 's' + [\"command\"]", "success", None);
        let history = vec![
            "Opened app: TextEdit".to_string(),
            "Typed 'status: in-progress' (textedit body)".to_string(),
            "Shortcut 's' + [\"command\"]".to_string(),
        ];

        let summary = Planner::summarize_execution(goal, &session, &history, true);
        assert!(summary.textedit_write_required);
        assert!(summary.textedit_write_confirmed);
        assert!(summary.textedit_save_required);
        assert!(summary.textedit_save_confirmed);
        assert!(summary.business_complete);
    }
}
