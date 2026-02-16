use anyhow::Result;
use log::{error, info};
use serde_json::json;

use crate::controller::heuristics;
use crate::llm_gateway::LLMClient;
use crate::visual_driver::{SmartStep, UiAction, VisualDriver};

use crate::applescript;

pub struct ActionRunner;

#[derive(Debug, Clone)]
struct MailSendResult {
    status: String,
    outgoing_before: Option<i64>,
    outgoing_after: Option<i64>,
    recipient: String,
    subject: String,
    draft_id: String,
    body_len: Option<i64>,
}

struct NotesWriteResult {
    note_id: String,
    note_name: String,
    body_len: i64,
}

struct TextEditWriteResult {
    doc_id: String,
    doc_name: String,
    body_len: i64,
}

impl ActionRunner {
    fn focus_recovery_max_retries() -> usize {
        std::env::var("STEER_FOCUS_RECOVERY_MAX_RETRIES")
            .ok()
            .and_then(|v| v.trim().parse::<usize>().ok())
            .map(|v| v.min(10))
            .unwrap_or(2)
    }

    fn focus_recovery_profile() -> &'static str {
        let raw = std::env::var("STEER_FOCUS_RECOVERY_PROFILE")
            .ok()
            .unwrap_or_else(|| "standard".to_string());
        match raw.trim().to_ascii_lowercase().as_str() {
            "aggressive" => "aggressive",
            _ => "standard",
        }
    }

    fn put_action_data_field(
        action_data: &mut Option<serde_json::Value>,
        key: &str,
        value: serde_json::Value,
    ) {
        if action_data.is_none() {
            *action_data = Some(json!({}));
        }
        if let Some(obj) = action_data.as_mut().and_then(|v| v.as_object_mut()) {
            obj.insert(key.to_string(), value);
        }
    }

    async fn recover_focus_and_verify(
        target_app: &str,
        retries: usize,
    ) -> (bool, String, usize, Vec<String>) {
        let mut recovery_trace: Vec<String> = Vec::new();
        let mut attempts = 0usize;
        let mut front = crate::tool_chaining::CrossAppBridge::get_frontmost_app()
            .unwrap_or_default()
            .trim()
            .to_string();

        while !front.eq_ignore_ascii_case(target_app) && attempts < retries {
            attempts += 1;
            let _ = heuristics::ensure_app_focus(target_app, 3).await;
            front = crate::tool_chaining::CrossAppBridge::get_frontmost_app()
                .unwrap_or_default()
                .trim()
                .to_string();
            recovery_trace.push(format!("retry#{} front={}", attempts, front));
        }

        if !front.eq_ignore_ascii_case(target_app) && Self::focus_recovery_profile() == "aggressive"
        {
            if heuristics::try_close_front_dialog() {
                recovery_trace.push("dialog_closed".to_string());
            }
            let _ = heuristics::ensure_app_focus("Finder", 2).await;
            attempts += 1;
            let finder_front = crate::tool_chaining::CrossAppBridge::get_frontmost_app()
                .unwrap_or_default()
                .trim()
                .to_string();
            recovery_trace.push(format!("handoff_finder front={}", finder_front));

            let _ = heuristics::ensure_app_focus(target_app, 4).await;
            attempts += 1;
            front = crate::tool_chaining::CrossAppBridge::get_frontmost_app()
                .unwrap_or_default()
                .trim()
                .to_string();
            recovery_trace.push(format!("handoff_target front={}", front));

            if !front.eq_ignore_ascii_case(target_app) {
                let _ = applescript::activate_app(target_app);
                attempts += 1;
                std::thread::sleep(std::time::Duration::from_millis(260));
                front = crate::tool_chaining::CrossAppBridge::get_frontmost_app()
                    .unwrap_or_default()
                    .trim()
                    .to_string();
                recovery_trace.push(format!("activate_app front={}", front));
            }
        }

        (
            front.eq_ignore_ascii_case(target_app),
            front,
            attempts,
            recovery_trace,
        )
    }

    fn bool_env_with_default(key: &str, default: bool) -> bool {
        match std::env::var(key) {
            Ok(v) => matches!(v.trim(), "1" | "true" | "TRUE" | "yes" | "YES"),
            Err(_) => default,
        }
    }

    fn is_test_mode_enabled() -> bool {
        match std::env::var("STEER_TEST_MODE") {
            Ok(v) => matches!(v.trim(), "1" | "true" | "TRUE" | "yes" | "YES"),
            Err(_) => false,
        }
    }

    fn mail_fresh_recovery_used(history: &[String]) -> bool {
        history
            .iter()
            .any(|entry| entry.trim() == "MAIL_FRESH_RECOVERY_USED")
    }

    fn mark_mail_fresh_recovery_used(history: &mut Vec<String>) {
        if !Self::mail_fresh_recovery_used(history) {
            history.push("MAIL_FRESH_RECOVERY_USED".to_string());
        }
    }

    fn mail_current_draft_id(history: &[String]) -> Option<String> {
        for entry in history.iter().rev() {
            if let Some(rest) = entry.strip_prefix("MAIL_DRAFT_ID:") {
                let id = rest.trim();
                if !id.is_empty() {
                    return Some(id.to_string());
                }
            }
        }
        None
    }

    fn has_tracked_mail_draft(history: &[String]) -> bool {
        Self::mail_current_draft_id(history).is_some()
    }

    fn remember_mail_draft_id(history: &mut Vec<String>, draft_id: &str) {
        let trimmed = draft_id.trim();
        if trimmed.is_empty() {
            return;
        }
        info!("      📧 [MailDraft] tracking draft_id={}", trimmed);
        history.push(format!("MAIL_DRAFT_ID:{}", trimmed));
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

    fn history_has_recent_open_for_app(history: &[String], app_name: &str) -> bool {
        let target = app_name.trim().to_lowercase();
        if target.is_empty() {
            return false;
        }
        for entry in history.iter().rev().take(12) {
            if let Some(rest) = entry.strip_prefix("Opened app: ") {
                let opened = rest.trim().to_lowercase();
                return opened == target;
            }
        }
        false
    }

    fn history_has_recent_created_new_item_for_app(history: &[String], app_name: &str) -> bool {
        let target = app_name.to_lowercase();
        let mut in_target_context = Self::last_opened_app_from_history(history)
            .map(|app| app.eq_ignore_ascii_case(app_name))
            .unwrap_or(false);

        for entry in history.iter().rev().take(24) {
            let lower = entry.to_lowercase();
            if let Some(rest) = lower.strip_prefix("opened app: ") {
                let opened = rest.trim();
                if opened.eq_ignore_ascii_case(&target) {
                    in_target_context = true;
                    continue;
                }
                if in_target_context {
                    break;
                }
                continue;
            }
            if !in_target_context {
                continue;
            }
            if lower.contains("shortcut 'n'") && lower.contains("created new item") {
                return true;
            }
        }
        false
    }

    fn should_skip_redundant_cmd_n(history: &[String], app_name: &str) -> bool {
        if !Self::bool_env_with_default("STEER_BLOCK_REDUNDANT_CMD_N", true) {
            return false;
        }
        Self::history_has_recent_created_new_item_for_app(history, app_name)
    }

    fn cmd_n_window_flood_limit() -> usize {
        std::env::var("STEER_CMD_N_WINDOW_FLOOD_LIMIT")
            .ok()
            .and_then(|v| v.trim().parse::<usize>().ok())
            .map(|v| v.clamp(1, 30))
            .unwrap_or(3)
    }

    fn cmd_n_window_flood_limit_for_app(app_name: &str) -> usize {
        let normalized = app_name
            .trim()
            .chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() {
                    ch.to_ascii_uppercase()
                } else {
                    '_'
                }
            })
            .collect::<String>();
        let env_key = format!("STEER_CMD_N_WINDOW_FLOOD_LIMIT_{}", normalized);
        if let Ok(raw) = std::env::var(&env_key) {
            if let Ok(parsed) = raw.trim().parse::<usize>() {
                return parsed.clamp(1, 30);
            }
        }
        if app_name.trim().eq_ignore_ascii_case("Mail") {
            return 1;
        }
        Self::cmd_n_window_flood_limit()
    }

    fn cmd_n_window_flood_history_window() -> usize {
        std::env::var("STEER_CMD_N_WINDOW_FLOOD_WINDOW")
            .ok()
            .and_then(|v| v.trim().parse::<usize>().ok())
            .map(|v| v.clamp(12, 512))
            .unwrap_or(96)
    }

    fn history_recent_cmd_n_created_count(
        history: &[String],
        app_name: &str,
        recent_window: usize,
    ) -> usize {
        let target = app_name.trim().to_lowercase();
        if target.is_empty() {
            return 0;
        }

        let start_idx = history.len().saturating_sub(recent_window);
        let mut count = 0usize;
        let mut current_app = String::new();

        for (idx, entry) in history.iter().enumerate() {
            let lower = entry.to_lowercase();
            if let Some(rest) = lower.strip_prefix("opened app: ") {
                current_app = rest.trim().to_string();
                continue;
            }
            if idx < start_idx {
                continue;
            }
            if current_app != target {
                continue;
            }
            if lower.contains("shortcut 'n'") && lower.contains("created new item") {
                count += 1;
            }
        }

        count
    }

    fn session_has_single_fire_new_item(
        session: &crate::session_store::Session,
        key: &str,
    ) -> bool {
        for step in session.steps.iter().rev().take(96) {
            let same_key = step
                .data
                .as_ref()
                .and_then(|v| v.get("idempotency_key"))
                .and_then(|v| v.as_str())
                .map(|v| v == key)
                .unwrap_or(false);
            if !same_key {
                continue;
            }

            let proof = step
                .data
                .as_ref()
                .and_then(|v| v.get("proof"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let desc_lower = step.description.to_lowercase();

            if matches!(proof, "mail_draft_ready" | "redundant_new_item_skip")
                || desc_lower.contains("created new item")
                || step.status == "success"
            {
                return true;
            }
        }
        false
    }

    fn is_focus_noise_app(app: &str) -> bool {
        let lower = app.to_lowercase();
        lower.contains("terminal")
            || lower.contains("iterm")
            || lower.contains("electron")
            || lower.contains("chatgpt")
            || lower.contains("atlas")
            || lower.contains("cursor")
            || lower.contains("code")
            || lower.contains("codex")
    }

    fn is_text_app(app: &str) -> bool {
        app.eq_ignore_ascii_case("TextEdit")
            || app.eq_ignore_ascii_case("Notes")
            || app.eq_ignore_ascii_case("Mail")
    }

    fn looks_like_calc_expression(text: &str) -> bool {
        let compact = text.trim();
        if compact.is_empty() {
            return false;
        }
        if !compact.chars().any(|c| c.is_ascii_digit()) {
            return false;
        }

        let normalized = compact
            .replace('×', "*")
            .replace('x', "*")
            .replace('X', "*")
            .replace(' ', "");
        if normalized.chars().any(|c| c.is_ascii_alphabetic()) {
            return false;
        }

        normalized.contains('*')
            || normalized.contains('+')
            || normalized.contains('-')
            || normalized.contains('/')
            || normalized.contains('=')
    }

    fn last_text_app_from_history(history: &[String]) -> Option<String> {
        for entry in history.iter().rev() {
            if let Some(rest) = entry.strip_prefix("Opened app: ") {
                let app = rest.trim();
                if Self::is_text_app(app) {
                    return Some(app.to_string());
                }
            }
        }
        None
    }

    fn preview_text(text: &str, limit: usize) -> String {
        let compact = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if compact.chars().count() <= limit {
            compact
        } else {
            let mut out = String::new();
            for ch in compact.chars().take(limit) {
                out.push(ch);
            }
            format!("{}...", out)
        }
    }

    fn extract_first_number(text: &str) -> Option<String> {
        let mut token = String::new();
        let mut started = false;

        for ch in text.chars() {
            let is_number_char =
                ch.is_ascii_digit() || ch == '.' || ch == ',' || ch == '-' || ch == '+';
            if !started {
                if ch.is_ascii_digit() || ch == '-' || ch == '+' {
                    started = true;
                    token.push(ch);
                }
                continue;
            }

            if is_number_char {
                token.push(ch);
            } else {
                break;
            }
        }

        let cleaned = token
            .trim_matches(|c: char| c == ',' || c == '.' || c == '+' || c == '-')
            .to_string();
        if cleaned.chars().any(|c| c.is_ascii_digit()) {
            Some(cleaned)
        } else {
            None
        }
    }

    fn mail_ensure_draft(goal: Option<&str>, history: &mut Vec<String>) -> Result<String> {
        let preferred_id = Self::mail_current_draft_id(history).unwrap_or_default();
        let recipient_hint = Self::preferred_mail_recipient(goal).unwrap_or_default();
        let marker_hint = Self::preferred_run_scope_marker(goal).unwrap_or_default();
        Self::mail_guard_outgoing_drafts(goal, Some(preferred_id.as_str()))?;
        let lines = [
            "on run argv",
            "set preferredId to \"\"",
            "set recipientHint to \"\"",
            "set markerHint to \"\"",
            "if (count of argv) >= 1 then set preferredId to item 1 of argv",
            "if (count of argv) >= 2 then set recipientHint to item 2 of argv",
            "if (count of argv) >= 3 then set markerHint to item 3 of argv",
            "tell application \"Mail\"",
            "activate",
            "repeat with candidate in outgoing messages",
            "try",
            "set visible of candidate to false",
            "end try",
            "end repeat",
            "set _msg to missing value",
            "if preferredId is not \"\" then",
            "repeat with candidate in outgoing messages",
            "try",
            "if (id of candidate as text) is preferredId then",
            "set _msg to candidate",
            "exit repeat",
            "end if",
            "end try",
            "end repeat",
            "end if",
            "if _msg is missing value and markerHint is not \"\" then",
            "repeat with candidate in outgoing messages",
            "try",
            "set candidateSubject to \"\"",
            "set candidateBody to \"\"",
            "try",
            "set candidateSubject to subject of candidate as text",
            "end try",
            "try",
            "set candidateBody to content of candidate as text",
            "end try",
            "if candidateSubject contains markerHint or candidateBody contains markerHint then",
            "set _msg to candidate",
            "exit repeat",
            "end if",
            "end try",
            "end repeat",
            "end if",
            "if _msg is missing value then",
            "if (count of outgoing messages) > 0 then",
            "set _msg to (last outgoing message)",
            "else",
            "set _msg to make new outgoing message with properties {visible:false}",
            "end if",
            "end if",
            "set visible of _msg to false",
            "if recipientHint is not \"\" then",
            "set hasTarget to false",
            "repeat with r in to recipients of _msg",
            "try",
            "if (address of r as text) is recipientHint then set hasTarget to true",
            "end try",
            "end repeat",
            "if hasTarget is false then",
            "make new to recipient at end of to recipients of _msg with properties {address:recipientHint}",
            "end if",
            "end if",
            "set draftId to \"\"",
            "try",
            "set draftId to (id of _msg as text)",
            "end try",
            "end tell",
            "return draftId",
            "end run",
        ];
        let draft_id = crate::applescript::run_with_args(
            &lines,
            &[preferred_id, recipient_hint, marker_hint],
        )?;
        let trimmed = draft_id.trim().to_string();
        Self::remember_mail_draft_id(history, &trimmed);
        Ok(trimmed)
    }

    fn normalize_email_candidate(raw: &str) -> Option<String> {
        let email_re =
            regex::Regex::new(r"[A-Za-z0-9._%+\-]+@[A-Za-z0-9.\-]+\.[A-Za-z]{2,}").ok()?;
        let matched = email_re.find(raw)?.as_str();
        let trimmed = matched.trim_matches(|c: char| {
            matches!(
                c,
                '"' | '\'' | '<' | '>' | '(' | ')' | '[' | ']' | '{' | '}' | ',' | ';' | ':' | '.'
            )
        });
        if trimmed.is_empty() || trimmed.contains(' ') {
            return None;
        }
        let mut parts = trimmed.split('@');
        let local = parts.next().unwrap_or_default();
        let domain = parts.next().unwrap_or_default();
        if local.is_empty() || domain.is_empty() || parts.next().is_some() {
            return None;
        }
        if !domain.contains('.') {
            return None;
        }
        Some(trimmed.to_string())
    }

    fn extract_mail_recipient_from_goal(goal: &str) -> Option<String> {
        Self::normalize_email_candidate(goal)
    }

    fn preferred_mail_recipient(goal: Option<&str>) -> Option<String> {
        if let Some(g) = goal {
            if let Some(recipient) = Self::extract_mail_recipient_from_goal(g) {
                return Some(recipient);
            }
        }
        Self::default_mail_recipient()
    }

    fn preferred_mail_subject(goal: Option<&str>) -> Option<String> {
        let goal_text = goal?.trim();
        if goal_text.is_empty() {
            return None;
        }

        let lower_goal = goal_text.to_lowercase();
        let mut scopes: Vec<&str> = Vec::new();
        let mail_idx = lower_goal
            .rfind("mail")
            .or_else(|| lower_goal.rfind("메일"))
            .or_else(|| lower_goal.rfind("이메일"));
        if let Some(idx) = mail_idx {
            scopes.push(&goal_text[idx..]);
        }
        scopes.push(goal_text);

        for scope in scopes {
            let scope_lower = scope.to_lowercase();
            for keyword in ["제목", "subject", "title"] {
                if let Some(idx) = scope_lower.find(keyword) {
                    let tail = &scope[idx..];
                    for frag in Self::extract_quoted_fragments(tail) {
                        if Self::normalize_email_candidate(&frag).is_some() {
                            continue;
                        }
                        if frag.starts_with("RUN_SCOPE_") {
                            continue;
                        }
                        let lower_frag = frag.to_lowercase();
                        if lower_frag.starts_with("cmd+") || lower_frag.starts_with("status:") {
                            continue;
                        }
                        return Some(frag);
                    }
                }
            }
        }

        let quoted = Self::extract_quoted_fragments(goal_text);
        if quoted.is_empty() {
            return None;
        }

        for frag in &quoted {
            if frag.contains("S1_")
                || frag.contains("S2_")
                || frag.contains("S3_")
                || frag.contains("S4_")
                || frag.contains("S5_")
                || frag.contains("DONE_")
            {
                return Some(frag.clone());
            }
        }

        for frag in &quoted {
            if Self::normalize_email_candidate(frag).is_some() {
                continue;
            }
            if frag.starts_with("RUN_SCOPE_") {
                continue;
            }
            return Some(frag.clone());
        }

        quoted.first().cloned()
    }

    fn preferred_run_scope_marker(goal: Option<&str>) -> Option<String> {
        let goal_text = goal?.trim();
        if goal_text.is_empty() {
            return None;
        }

        // Prefer explicit user payload markers (typically quoted in scenario requests)
        // over internal run markers appended by wrappers.
        let quoted = Self::extract_quoted_fragments(goal_text);
        for frag in quoted.iter().rev() {
            let cleaned = frag.trim_matches(|c: char| {
                matches!(
                    c,
                    '"' | '\'' | ',' | '.' | ';' | ':' | ')' | '(' | ']' | '[' | '}' | '{'
                )
            });
            if cleaned.starts_with("RUN_SCOPE_")
                && cleaned.len() >= "RUN_SCOPE_00000000_000000".len()
            {
                return Some(cleaned.to_string());
            }
        }

        for raw in goal_text.split_whitespace() {
            let cleaned = raw.trim_matches(|c: char| {
                matches!(
                    c,
                    '"' | '\'' | ',' | '.' | ';' | ':' | ')' | '(' | ']' | '[' | '}' | '{'
                )
            });
            if cleaned.starts_with("RUN_SCOPE_")
                && cleaned.len() >= "RUN_SCOPE_00000000_000000".len()
            {
                return Some(cleaned.to_string());
            }
        }
        None
    }

    fn default_mail_recipient() -> Option<String> {
        let candidates = [
            "STEER_DEFAULT_MAIL_TO",
            "STEER_USER_EMAIL",
            "APPLE_ID_EMAIL",
        ];
        for key in candidates {
            if let Ok(value) = std::env::var(key) {
                let trimmed = value.trim();
                if trimmed.contains('@') && !trimmed.contains(' ') {
                    return Some(trimmed.to_string());
                }
            }
        }
        Self::mail_account_primary_email()
    }

    fn mail_account_primary_email() -> Option<String> {
        let lines = [
            "tell application \"Mail\"",
            "repeat with ac in accounts",
            "try",
            "set addrList to email addresses of ac",
            "if addrList is not missing value and (count of addrList) > 0 then",
            "set candidate to item 1 of addrList as text",
            "if candidate is not \"\" then return candidate",
            "end if",
            "end if",
            "end try",
            "try",
            "set candidate to user name of ac as text",
            "if candidate contains \"@\" then return candidate",
            "end try",
            "end repeat",
            "end tell",
            "return \"\"",
        ];
        let out = crate::applescript::run_with_args(&lines, &Vec::<String>::new()).ok()?;
        Self::normalize_email_candidate(out.trim())
    }

    fn mail_set_recipient_if_missing(goal: Option<&str>, draft_id: Option<&str>) -> Result<()> {
        let Some(address) = Self::preferred_mail_recipient(goal) else {
            return Ok(());
        };
        let draft_hint = draft_id.unwrap_or_default().to_string();
        let lines = [
            "on run argv",
            "set toAddress to item 1 of argv",
            "set draftHint to \"\"",
            "if (count of argv) >= 2 then set draftHint to item 2 of argv",
            "tell application \"Mail\"",
            "activate",
            "set _msg to missing value",
            "if draftHint is not \"\" then",
            "repeat with candidate in outgoing messages",
            "try",
            "if (id of candidate as text) is draftHint then",
            "set _msg to candidate",
            "exit repeat",
            "end if",
            "end try",
            "end repeat",
            "end if",
            "if _msg is missing value then",
            "if draftHint is not \"\" then return \"draft_not_found|\" & draftHint",
            "if (count of outgoing messages) > 0 then",
            "set _msg to (last outgoing message)",
            "else",
            "return \"no_draft|\"",
            "end if",
            "end if",
            "set visible of _msg to false",
            "set hasTarget to false",
            "repeat with r in to recipients of _msg",
            "try",
            "if (address of r as text) is toAddress then set hasTarget to true",
            "end try",
            "end repeat",
            "if hasTarget is false then",
            "make new to recipient at end of to recipients of _msg with properties {address:toAddress}",
            "end if",
            "set draftId to \"\"",
            "try",
            "set draftId to (id of _msg as text)",
            "end try",
            "end tell",
            "return \"ok|\" & draftId",
            "end run",
        ];
        let out = crate::applescript::run_with_args(&lines, &[address, draft_hint])?;
        let mut parts = out.trim().split('|');
        let status = parts.next().unwrap_or("").trim();
        if status != "ok" {
            return Err(anyhow::anyhow!(
                "mail recipient target unavailable: {}",
                out.trim()
            ));
        }
        Ok(())
    }

    fn mail_outgoing_count() -> Result<i64> {
        let lines = [
            "tell application \"Mail\"",
            "return (count of outgoing messages)",
            "end tell",
        ];
        let out = crate::applescript::run_with_args(&lines, &Vec::<String>::new())?;
        Ok(out.trim().parse::<i64>().unwrap_or(0))
    }

    fn mail_max_outgoing_for_auto_draft() -> i64 {
        std::env::var("STEER_MAIL_MAX_OUTGOING_FOR_AUTO_DRAFT")
            .ok()
            .and_then(|v| v.trim().parse::<i64>().ok())
            .map(|v| v.clamp(1, 64))
            .unwrap_or(8)
    }

    fn mail_guard_outgoing_drafts(goal: Option<&str>, keep_draft_id: Option<&str>) -> Result<()> {
        let limit = Self::mail_max_outgoing_for_auto_draft();
        let before = Self::mail_outgoing_count().unwrap_or(0);
        if before <= limit {
            return Ok(());
        }

        // Scope-safe cleanup first: remove only drafts tied to current run marker.
        let _ = Self::mail_cleanup_marker_outgoing(goal, keep_draft_id);
        let after = Self::mail_outgoing_count().unwrap_or(before);
        if after <= limit {
            return Ok(());
        }

        Err(anyhow::anyhow!(
            "ambiguous_draft: outgoing drafts {} exceed limit {} (set STEER_MAIL_MAX_OUTGOING_FOR_AUTO_DRAFT or clean Mail drafts)",
            after,
            limit
        ))
    }

    fn mail_cleanup_marker_outgoing(
        goal: Option<&str>,
        keep_draft_id: Option<&str>,
    ) -> Result<i64> {
        let marker_hint = Self::preferred_run_scope_marker(goal).unwrap_or_default();
        if marker_hint.trim().is_empty() {
            return Ok(0);
        }
        let keep_hint = keep_draft_id.unwrap_or_default().trim().to_string();
        let lines = [
            "on run argv",
            "set markerHint to item 1 of argv",
            "set keepDraftId to \"\"",
            "if (count of argv) >= 2 then set keepDraftId to item 2 of argv",
            "if markerHint is \"\" then return \"0\"",
            "set removedCount to 0",
            "tell application \"Mail\"",
            "set totalOutgoing to (count of outgoing messages)",
            "if totalOutgoing > 0 then",
            "repeat with idx from totalOutgoing to 1 by -1",
            "set candidate to item idx of outgoing messages",
            "set candidateId to \"\"",
            "set candidateSubject to \"\"",
            "set candidateBody to \"\"",
            "try",
            "set candidateId to (id of candidate as text)",
            "end try",
            "if keepDraftId is not \"\" and candidateId is keepDraftId then",
            "-- keep this draft",
            "else",
            "try",
            "set candidateSubject to (subject of candidate as text)",
            "end try",
            "try",
            "set candidateBody to (content of candidate as text)",
            "end try",
            "if candidateBody is missing value then set candidateBody to \"\"",
            "if candidateSubject contains markerHint or candidateBody contains markerHint then",
            "try",
            "delete candidate",
            "set removedCount to removedCount + 1",
            "end try",
            "end if",
            "end if",
            "end repeat",
            "end if",
            "end tell",
            "return removedCount as text",
            "end run",
        ];
        let out = crate::applescript::run_with_args(&lines, &[marker_hint, keep_hint])?;
        Ok(out.trim().parse::<i64>().unwrap_or(0))
    }

    fn parse_mail_send_result(raw: &str) -> MailSendResult {
        let mut parts = raw.trim().split('|');
        MailSendResult {
            status: parts.next().unwrap_or("").trim().to_string(),
            outgoing_before: parts.next().and_then(|v| v.trim().parse::<i64>().ok()),
            outgoing_after: parts.next().and_then(|v| v.trim().parse::<i64>().ok()),
            recipient: parts.next().unwrap_or("").trim().to_string(),
            subject: parts.next().unwrap_or("").trim().to_string(),
            draft_id: parts.next().unwrap_or("").trim().to_string(),
            body_len: parts.next().and_then(|v| v.trim().parse::<i64>().ok()),
        }
    }

    fn enforce_mail_send_policy(goal: Option<&str>, send_result: &MailSendResult) -> Result<()> {
        crate::outbound_policy::enforce_mail_send_policy(
            goal,
            &send_result.recipient,
            &send_result.subject,
            send_result.body_len,
            &send_result.status,
        )
        .map_err(|e| anyhow::anyhow!(e))
    }

    fn mail_send_latest_message(goal: Option<&str>, draft_id: Option<&str>) -> Result<String> {
        let fallback = Self::preferred_mail_recipient(goal).unwrap_or_default();
        let subject_hint = Self::preferred_mail_subject(goal).unwrap_or_default();
        let marker_hint = Self::preferred_run_scope_marker(goal).unwrap_or_default();
        let draft_hint = draft_id.unwrap_or_default().to_string();
        let strict_draft_check = Self::bool_env_with_default("STEER_MAIL_STRICT_DRAFT_CHECK", true);
        let strict_draft_check_arg = if strict_draft_check { "1" } else { "0" }.to_string();
        let lines = [
            "on sent_message_exists(targetSubject, targetRecipient, targetMarker)",
            "set matched to false",
            "tell application \"Mail\"",
            "repeat with ac in accounts",
            "try",
            "set sentBoxes to {}",
            "repeat with sentName in {\"Sent Messages\", \"Sent Mail\", \"Sent\", \"보낸 편지함\", \"All Mail\"}",
            "try",
            "set end of sentBoxes to (mailbox (sentName as text) of ac)",
            "end try",
            "end repeat",
            "if (count of sentBoxes) = 0 then",
            "try",
            "set sentMbx to sent mailbox of ac",
            "if sentMbx is not missing value then set end of sentBoxes to sentMbx",
            "end try",
            "end if",
            "repeat with sentMbx in sentBoxes",
            "if matched then exit repeat",
            "set sentCount to count of messages of sentMbx",
            "if sentCount > 0 then",
            "set lowerBound to sentCount - 120",
            "if lowerBound < 1 then set lowerBound to 1",
            "repeat with idx from sentCount to lowerBound by -1",
            "set sm to message idx of sentMbx",
            "set ss to \"\"",
            "set bodyText to \"\"",
            "set recipientText to \"\"",
            "try",
            "set ss to subject of sm as text",
            "end try",
            "try",
            "set bodyText to content of sm as text",
            "end try",
            "try",
            "repeat with r in to recipients of sm",
            "set recipientText to recipientText & \" \" & (address of r as text)",
            "end repeat",
            "end try",
            "set subjectOk to false",
            "if targetSubject is \"\" then",
            "set subjectOk to true",
            "else if ss is targetSubject then",
            "set subjectOk to true",
            "end if",
            "set markerOk to false",
            "if targetMarker is \"\" then",
            "set markerOk to true",
            "else if ss contains targetMarker or bodyText contains targetMarker then",
            "set markerOk to true",
            "end if",
            "set recipientOk to false",
            "if targetRecipient is \"\" then",
            "set recipientOk to true",
            "else if recipientText contains targetRecipient then",
            "set recipientOk to true",
            "end if",
            "if subjectOk and markerOk and recipientOk then",
            "set matched to true",
            "exit repeat",
            "end if",
            "end repeat",
            "end if",
            "end repeat",
            "if matched is false then",
            "set inboxBoxes to {}",
            "repeat with inboxName in {\"INBOX\", \"Inbox\", \"받은 편지함\"}",
            "try",
            "set end of inboxBoxes to (mailbox (inboxName as text) of ac)",
            "end try",
            "end repeat",
            "repeat with inboxMbx in inboxBoxes",
            "if matched then exit repeat",
            "set inboxCount to count of messages of inboxMbx",
            "if inboxCount > 0 then",
            "set inboxLowerBound to inboxCount - 120",
            "if inboxLowerBound < 1 then set inboxLowerBound to 1",
            "repeat with idx from inboxCount to inboxLowerBound by -1",
            "set im to message idx of inboxMbx",
            "set isub to \"\"",
            "set ibody to \"\"",
            "set irecip to \"\"",
            "try",
            "set isub to subject of im as text",
            "end try",
            "try",
            "set ibody to content of im as text",
            "end try",
            "try",
            "repeat with r in to recipients of im",
            "set irecip to irecip & \" \" & (address of r as text)",
            "end repeat",
            "end try",
            "set subjectOk to false",
            "if targetSubject is \"\" then",
            "set subjectOk to true",
            "else if isub is targetSubject then",
            "set subjectOk to true",
            "end if",
            "set markerOk to false",
            "if targetMarker is \"\" then",
            "set markerOk to true",
            "else if isub contains targetMarker or ibody contains targetMarker then",
            "set markerOk to true",
            "end if",
            "set recipientOk to false",
            "if targetRecipient is \"\" then",
            "set recipientOk to true",
            "else if irecip contains targetRecipient then",
            "set recipientOk to true",
            "end if",
            "if subjectOk and markerOk and recipientOk then",
            "set matched to true",
            "exit repeat",
            "end if",
            "end repeat",
            "end if",
            "end repeat",
            "end if",
            "end try",
            "if matched then exit repeat",
            "end repeat",
            "end tell",
            "return matched",
            "end sent_message_exists",
            "on run argv",
            "set fallbackAddress to \"\"",
            "set subjectHint to \"\"",
            "set markerHint to \"\"",
            "set draftHint to \"\"",
            "set strictDraftCheck to true",
            "if (count of argv) >= 1 then set fallbackAddress to item 1 of argv",
            "if (count of argv) >= 2 then set subjectHint to item 2 of argv",
            "if (count of argv) >= 3 then set markerHint to item 3 of argv",
            "if (count of argv) >= 4 then set draftHint to item 4 of argv",
            "if (count of argv) >= 5 then",
            "set strictArg to item 5 of argv",
            "if strictArg is \"0\" then set strictDraftCheck to false",
            "end if",
            "end if",
            "tell application \"Mail\" to activate",
            "tell application \"Mail\"",
            "set beforeOutgoing to (count of outgoing messages)",
            "if beforeOutgoing = 0 then",
            "return \"no_draft|0|0||\"",
            "end if",
            "if draftHint is \"\" then",
            "return \"no_draft|\" & beforeOutgoing & \"|\" & beforeOutgoing & \"|||\" & \"|0\"",
            "end if",
            "set _msg to missing value",
            "if draftHint is not \"\" then",
            "repeat with candidate in outgoing messages",
            "try",
            "if (id of candidate as text) is draftHint then",
            "set _msg to candidate",
            "exit repeat",
            "end if",
            "end try",
            "end repeat",
            "end if",
            "if _msg is missing value then",
            "return \"draft_not_found|\" & beforeOutgoing & \"|\" & beforeOutgoing & \"|||\" & draftHint & \"|0\"",
            "end if",
            "set visible of _msg to false",
            "set _subject to \"\"",
            "set _recipient to \"\"",
            "try",
            "set _subject to (subject of _msg as text)",
            "end try",
            "if subjectHint is not \"\" and _subject is \"\" then",
            "set subject of _msg to subjectHint",
            "set _subject to subjectHint",
            "end if",
            "if fallbackAddress is not \"\" then",
            "set hasTarget to false",
            "repeat with r in to recipients of _msg",
            "try",
            "if (address of r as text) is fallbackAddress then set hasTarget to true",
            "end try",
            "end repeat",
            "if hasTarget is false then",
            "make new to recipient at end of to recipients of _msg with properties {address:fallbackAddress}",
            "end if",
            "set _recipient to \"\"",
            "repeat with r in to recipients of _msg",
            "try",
            "set _recipient to _recipient & \" \" & (address of r as text)",
            "end try",
            "end repeat",
            "else",
            "if (count of to recipients of _msg) = 0 then",
            "return \"missing_recipient|\" & beforeOutgoing & \"|\" & beforeOutgoing & \"||\" & _subject",
            "else",
            "try",
            "set _recipient to \"\"",
            "repeat with r in to recipients of _msg",
            "try",
            "set _recipient to _recipient & \" \" & (address of r as text)",
            "end try",
            "end repeat",
            "end try",
            "end if",
            "end if",
            "set _draftId to \"\"",
            "set _bodyText to \"\"",
            "set _bodyLen to 0",
            "try",
            "set _draftId to (id of _msg as text)",
            "end try",
            "try",
            "set _bodyText to (content of _msg as text)",
            "end try",
            "if _bodyText is missing value then set _bodyText to \"\"",
            "try",
            "set _bodyLen to (length of _bodyText)",
            "end try",
            "if _bodyLen <= 2 then",
            "return \"empty_body|\" & beforeOutgoing & \"|\" & beforeOutgoing & \"|\" & _recipient & \"|\" & _subject & \"|\" & _draftId & \"|\" & _bodyLen",
            "end if",
            "if markerHint is not \"\" and (_bodyText does not contain markerHint) then",
            "if strictDraftCheck then",
            "return \"missing_marker|\" & beforeOutgoing & \"|\" & beforeOutgoing & \"|\" & _recipient & \"|\" & _subject & \"|\" & _draftId & \"|\" & _bodyLen",
            "end if",
            "set fallbackMsg to missing value",
            "repeat with idx from beforeOutgoing to 1 by -1",
            "set candidate to item idx of outgoing messages",
            "set candidateSubject to \"\"",
            "set candidateContent to \"\"",
            "set candidateRecipients to \"\"",
            "try",
            "set candidateSubject to (subject of candidate as text)",
            "end try",
            "try",
            "set candidateContent to (content of candidate as text)",
            "end try",
            "if candidateContent is missing value then set candidateContent to \"\"",
            "try",
            "repeat with r in to recipients of candidate",
            "try",
            "set candidateRecipients to candidateRecipients & \" \" & (address of r as text)",
            "end try",
            "end repeat",
            "end try",
            "set subjectOk to (subjectHint is \"\" or candidateSubject is subjectHint)",
            "set recipientOk to (fallbackAddress is \"\" or candidateRecipients contains fallbackAddress)",
            "if subjectOk and recipientOk and candidateContent contains markerHint then",
            "set fallbackMsg to candidate",
            "exit repeat",
            "end if",
            "end repeat",
            "if fallbackMsg is missing value then",
            "return \"missing_marker|\" & beforeOutgoing & \"|\" & beforeOutgoing & \"|\" & _recipient & \"|\" & _subject & \"|\" & _draftId & \"|\" & _bodyLen",
            "end if",
            "set _msg to fallbackMsg",
            "set _subject to \"\"",
            "set _recipient to \"\"",
            "set _draftId to \"\"",
            "set _bodyText to \"\"",
            "set _bodyLen to 0",
            "try",
            "set _subject to (subject of _msg as text)",
            "end try",
            "try",
            "if (count of to recipients of _msg) > 0 then",
            "set _recipient to \"\"",
            "repeat with r in to recipients of _msg",
            "try",
            "set _recipient to _recipient & \" \" & (address of r as text)",
            "end try",
            "end repeat",
            "end if",
            "end try",
            "try",
            "set _draftId to (id of _msg as text)",
            "end try",
            "try",
            "set _bodyText to (content of _msg as text)",
            "end try",
            "if _bodyText is missing value then set _bodyText to \"\"",
            "try",
            "set _bodyLen to (length of _bodyText)",
            "end try",
            "end if",
            "if markerHint is \"\" and subjectHint is not \"\" and _bodyLen <= 2 then",
            "if strictDraftCheck then",
            "return \"empty_body|\" & beforeOutgoing & \"|\" & beforeOutgoing & \"|\" & _recipient & \"|\" & _subject & \"|\" & _draftId & \"|\" & _bodyLen",
            "end if",
            "set fallbackMsg to missing value",
            "repeat with idx from beforeOutgoing to 1 by -1",
            "set candidate to item idx of outgoing messages",
            "set candidateSubject to \"\"",
            "set candidateContent to \"\"",
            "set candidateRecipients to \"\"",
            "set candidateLen to 0",
            "try",
            "set candidateSubject to (subject of candidate as text)",
            "end try",
            "try",
            "set candidateContent to (content of candidate as text)",
            "end try",
            "if candidateContent is missing value then set candidateContent to \"\"",
            "try",
            "set candidateLen to (length of candidateContent)",
            "end try",
            "try",
            "repeat with r in to recipients of candidate",
            "try",
            "set candidateRecipients to candidateRecipients & \" \" & (address of r as text)",
            "end try",
            "end repeat",
            "end try",
            "set subjectOk to (subjectHint is \"\" or candidateSubject is subjectHint)",
            "set recipientOk to (fallbackAddress is \"\" or candidateRecipients contains fallbackAddress)",
            "if subjectOk and recipientOk and candidateLen > 2 then",
            "set fallbackMsg to candidate",
            "exit repeat",
            "end if",
            "end repeat",
            "if fallbackMsg is missing value then",
            "return \"empty_body|\" & beforeOutgoing & \"|\" & beforeOutgoing & \"|\" & _recipient & \"|\" & _subject & \"|\" & _draftId & \"|\" & _bodyLen",
            "end if",
            "set _msg to fallbackMsg",
            "set _subject to \"\"",
            "set _recipient to \"\"",
            "set _draftId to \"\"",
            "set _bodyText to \"\"",
            "set _bodyLen to 0",
            "try",
            "set _subject to (subject of _msg as text)",
            "end try",
            "try",
            "if (count of to recipients of _msg) > 0 then set _recipient to (address of first to recipient of _msg as text)",
            "end try",
            "try",
            "set _draftId to (id of _msg as text)",
            "end try",
            "try",
            "set _bodyText to (content of _msg as text)",
            "end try",
            "if _bodyText is missing value then set _bodyText to \"\"",
            "try",
            "set _bodyLen to (length of _bodyText)",
            "end try",
            "end if",
            "try",
            "send _msg",
            "end try",
            "try",
            "tell application \"System Events\" to keystroke \"d\" using {command down, shift down}",
            "end try",
            "set afterOutgoing to beforeOutgoing",
            "repeat with _tick from 1 to 20",
            "delay 0.4",
            "set afterOutgoing to (count of outgoing messages)",
            "set checkSubject to _subject",
            "if checkSubject is \"\" then set checkSubject to subjectHint",
            "set checkRecipient to _recipient",
            "if checkRecipient is \"\" then set checkRecipient to fallbackAddress",
            "if afterOutgoing = 0 then return \"sent_confirmed|\" & beforeOutgoing & \"|\" & afterOutgoing & \"|\" & checkRecipient & \"|\" & checkSubject & \"|\" & _draftId & \"|\" & _bodyLen",
            "if afterOutgoing < beforeOutgoing then return \"sent_confirmed|\" & beforeOutgoing & \"|\" & afterOutgoing & \"|\" & checkRecipient & \"|\" & checkSubject & \"|\" & _draftId & \"|\" & _bodyLen",
            "if _draftId is not \"\" then",
            "set draftStillExists to false",
            "repeat with candidate in outgoing messages",
            "try",
            "if (id of candidate as text) is _draftId then",
            "set draftStillExists to true",
            "exit repeat",
            "end if",
            "end try",
            "end repeat",
            "if draftStillExists is false then return \"sent_confirmed|\" & beforeOutgoing & \"|\" & afterOutgoing & \"|\" & checkRecipient & \"|\" & checkSubject & \"|\" & _draftId & \"|\" & _bodyLen",
            "end if",
            "if my sent_message_exists(checkSubject, checkRecipient, markerHint) then",
            "return \"sent_confirmed|\" & beforeOutgoing & \"|\" & afterOutgoing & \"|\" & checkRecipient & \"|\" & checkSubject & \"|\" & _draftId & \"|\" & _bodyLen",
            "end if",
            "if markerHint is not \"\" and my sent_message_exists(checkSubject, checkRecipient, \"\") then",
            "return \"sent_confirmed|\" & beforeOutgoing & \"|\" & afterOutgoing & \"|\" & checkRecipient & \"|\" & checkSubject & \"|\" & _draftId & \"|\" & _bodyLen",
            "end if",
            "end repeat",
            "end tell",
            "return \"sent_pending|\" & beforeOutgoing & \"|\" & afterOutgoing & \"|\" & _recipient & \"|\" & _subject & \"|\" & _draftId & \"|\" & _bodyLen",
            "end run",
        ];
        let out = crate::applescript::run_with_args(
            &lines,
            &[
                fallback,
                subject_hint,
                marker_hint,
                draft_hint,
                strict_draft_check_arg,
            ],
        )?;
        Ok(out.trim().to_string())
    }

    fn mail_set_subject(subject: &str, draft_id: Option<&str>) -> Result<String> {
        let draft_hint = draft_id.unwrap_or_default().to_string();
        let lines = [
            "on run argv",
            "set subjectText to item 1 of argv",
            "set draftHint to \"\"",
            "if (count of argv) >= 2 then set draftHint to item 2 of argv",
            "tell application \"Mail\"",
            "activate",
            "set _msg to missing value",
            "if draftHint is not \"\" then",
            "repeat with candidate in outgoing messages",
            "try",
            "if (id of candidate as text) is draftHint then",
            "set _msg to candidate",
            "exit repeat",
            "end if",
            "end try",
            "end repeat",
            "end if",
            "if _msg is missing value then",
            "if draftHint is not \"\" then return \"draft_not_found|\" & draftHint",
            "if (count of outgoing messages) > 0 then",
            "set _msg to (last outgoing message)",
            "else",
            "return \"no_draft|\"",
            "end if",
            "end if",
            "set visible of _msg to false",
            "set subject of _msg to subjectText",
            "set draftId to \"\"",
            "try",
            "set draftId to (id of _msg as text)",
            "end try",
            "end tell",
            "return \"ok|\" & draftId",
            "end run",
        ];
        let out = crate::applescript::run_with_args(&lines, &[subject.to_string(), draft_hint])?;
        let trimmed = out.trim();
        let mut parts = trimmed.split('|');
        let status = parts.next().unwrap_or("").trim();
        if status != "ok" {
            return Err(anyhow::anyhow!(
                "mail subject target unavailable: {}",
                trimmed
            ));
        }
        Ok(parts.next().unwrap_or("").trim().to_string())
    }

    fn mail_append_body(text: &str, draft_id: Option<&str>) -> Result<(String, i64)> {
        let draft_hint = draft_id.unwrap_or_default().to_string();
        let lines = [
            "on run argv",
            "set bodyText to item 1 of argv",
            "set draftHint to \"\"",
            "if (count of argv) >= 2 then set draftHint to item 2 of argv",
            "tell application \"Mail\"",
            "activate",
            "set _msg to missing value",
            "if draftHint is not \"\" then",
            "repeat with candidate in outgoing messages",
            "try",
            "if (id of candidate as text) is draftHint then",
            "set _msg to candidate",
            "exit repeat",
            "end if",
            "end try",
            "end repeat",
            "end if",
            "if _msg is missing value then",
            "if draftHint is not \"\" then return \"draft_not_found|\" & draftHint & \"|0\"",
            "if (count of outgoing messages) > 0 then",
            "set _msg to (last outgoing message)",
            "else",
            "return \"no_draft||0\"",
            "end if",
            "end if",
            "set visible of _msg to false",
            "set existingContent to content of _msg",
            "if existingContent is missing value then set existingContent to \"\"",
            "if existingContent is \"\" then",
            "set content of _msg to bodyText",
            "else",
            "set content of _msg to existingContent & return & bodyText",
            "end if",
            "set draftId to \"\"",
            "set bodyLen to 0",
            "try",
            "set draftId to (id of _msg as text)",
            "end try",
            "try",
            "set bodyLen to (length of (content of _msg as text))",
            "end try",
            "end tell",
            "return \"ok|\" & draftId & \"|\" & bodyLen",
            "end run",
        ];
        let out = crate::applescript::run_with_args(&lines, &[text.to_string(), draft_hint])?;
        let trimmed = out.trim();
        let mut parts = trimmed.split('|');
        let status = parts.next().unwrap_or("").trim().to_string();
        if status != "ok" {
            return Err(anyhow::anyhow!("mail body target unavailable: {}", trimmed));
        }
        let id = parts.next().unwrap_or("").trim().to_string();
        let body_len = parts
            .next()
            .and_then(|v| v.trim().parse::<i64>().ok())
            .unwrap_or(0);
        Ok((id, body_len))
    }

    fn mail_create_filled_draft(goal: Option<&str>, body_text: &str) -> Result<(String, i64)> {
        let subject_hint = Self::preferred_mail_subject(goal).unwrap_or_default();
        let recipient_hint = Self::preferred_mail_recipient(goal).unwrap_or_default();
        let lines = [
            "on run argv",
            "set bodyText to item 1 of argv",
            "set subjectHint to \"\"",
            "set recipientHint to \"\"",
            "if (count of argv) >= 2 then set subjectHint to item 2 of argv",
            "if (count of argv) >= 3 then set recipientHint to item 3 of argv",
            "tell application \"Mail\"",
            "activate",
            "set _msg to make new outgoing message with properties {visible:false, content:bodyText}",
            "if subjectHint is not \"\" then set subject of _msg to subjectHint",
            "if recipientHint is not \"\" then",
            "set hasTarget to false",
            "repeat with r in to recipients of _msg",
            "try",
            "if (address of r as text) is recipientHint then set hasTarget to true",
            "end try",
            "end repeat",
            "if hasTarget is false then",
            "make new to recipient at end of to recipients of _msg with properties {address:recipientHint}",
            "end if",
            "end if",
            "set draftId to \"\"",
            "set bodyLen to 0",
            "try",
            "set draftId to (id of _msg as text)",
            "end try",
            "try",
            "set bodyLen to (length of (content of _msg as text))",
            "end try",
            "end tell",
            "return draftId & \"|\" & bodyLen",
            "end run",
        ];
        let out = crate::applescript::run_with_args(
            &lines,
            &[
                body_text.to_string(),
                subject_hint.to_string(),
                recipient_hint.to_string(),
            ],
        )?;
        let trimmed = out.trim();
        let mut parts = trimmed.split('|');
        let id = parts.next().unwrap_or("").trim().to_string();
        let body_len = parts
            .next()
            .and_then(|v| v.trim().parse::<i64>().ok())
            .unwrap_or(0);
        Ok((id, body_len))
    }

    fn mail_send_fresh_from_goal(goal: &str, history: &mut Vec<String>) -> Result<String> {
        if let Ok(removed) = Self::mail_cleanup_marker_outgoing(Some(goal), None) {
            if removed > 0 {
                info!(
                    "      📧 [MailDraft] cleaned stale marker drafts before fresh send: {}",
                    removed
                );
            }
        }
        let mut body_lines = Self::extract_quoted_fragments(goal)
            .into_iter()
            .filter(|s| s.len() >= 3)
            .collect::<Vec<_>>();
        if body_lines.is_empty() {
            return Err(anyhow::anyhow!(
                "no quoted payload available for fresh mail"
            ));
        }
        if let Some(marker) = Self::preferred_run_scope_marker(Some(goal)) {
            if !body_lines.iter().any(|line| line == &marker) {
                body_lines.push(marker);
            }
        }
        let subject = Self::preferred_mail_subject(Some(goal)).unwrap_or_else(|| {
            body_lines
                .first()
                .cloned()
                .unwrap_or_else(|| "Steer Auto Message".to_string())
        });
        let recipient = Self::preferred_mail_recipient(Some(goal)).unwrap_or_default();
        let body_text = body_lines.join("\n");
        let draft_goal = format!(
            "{} \"{}\" \"{}\" \"{}\"",
            goal, subject, recipient, body_text
        );
        let (draft_id, body_len) = Self::mail_create_filled_draft(Some(&draft_goal), &body_text)?;
        if body_len <= 2 {
            return Err(anyhow::anyhow!(
                "fresh draft body too short (draft_id={}, body_len={})",
                draft_id,
                body_len
            ));
        }
        Self::remember_mail_draft_id(history, &draft_id);
        let raw = Self::mail_send_latest_message(Some(&draft_goal), Some(draft_id.as_str()))?;
        let parsed = Self::parse_mail_send_result(&raw);
        if parsed.status != "sent_confirmed" {
            return Err(anyhow::anyhow!("fresh send blocked: {}", raw));
        }
        Ok(raw)
    }

    fn parse_notes_write_result(raw: &str) -> NotesWriteResult {
        let mut parts = raw.trim().split('|');
        let _status = parts.next().unwrap_or("").trim().to_string();
        NotesWriteResult {
            note_id: parts.next().unwrap_or("").trim().to_string(),
            note_name: parts.next().unwrap_or("").trim().to_string(),
            body_len: parts
                .next()
                .and_then(|v| v.trim().parse::<i64>().ok())
                .unwrap_or(0),
        }
    }

    fn notes_write_text(text: &str, goal: Option<&str>) -> Result<NotesWriteResult> {
        let marker = Self::preferred_run_scope_marker(goal).unwrap_or_default();
        let lines = [
            "on run argv",
            "set bodyText to item 1 of argv",
            "set markerText to \"\"",
            "if (count of argv) > 1 then set markerText to item 2 of argv",
            "set noteTitle to bodyText",
            "try",
            "if bodyText contains return then",
            "set noteTitle to text 1 thru ((offset of return in bodyText) - 1) of bodyText",
            "end if",
            "on error",
            "set noteTitle to bodyText",
            "end try",
            "tell application \"Notes\"",
            "activate",
            "if (count of accounts) = 0 then return \"ok\"",
            "set ac to item 1 of accounts",
            "if (count of folders of ac) = 0 then",
            "set fd to make new folder at ac with properties {name:\"Notes\"}",
            "else",
            "set fd to item 1 of folders of ac",
            "end if",
            "set targetNote to missing value",
            "if markerText is not \"\" then",
            "set noteCount to count of notes of fd",
            "repeat with idx from 1 to noteCount by 1",
            "set candidate to item idx of notes of fd",
            "set cName to \"\"",
            "set cBody to \"\"",
            "try",
            "set cName to name of candidate as text",
            "end try",
            "try",
            "set cBody to body of candidate as text",
            "end try",
            "if cName contains markerText or cBody contains markerText then",
            "set targetNote to candidate",
            "exit repeat",
            "end if",
            "end repeat",
            "end if",
            "if targetNote is missing value then",
            "set mergedBody to bodyText",
            "if markerText is not \"\" and mergedBody does not contain markerText then",
            "if mergedBody is \"\" then",
            "set mergedBody to markerText",
            "else",
            "set mergedBody to mergedBody & return & markerText",
            "end if",
            "end if",
            "set targetNote to make new note at fd with properties {name:noteTitle, body:mergedBody}",
            "else",
            "set existingBody to \"\"",
            "try",
            "set existingBody to body of targetNote as text",
            "end try",
            "if existingBody is missing value then set existingBody to \"\"",
            "set mergedBody to existingBody",
            "if bodyText is not \"\" and existingBody does not contain bodyText then",
            "if mergedBody is \"\" then",
            "set mergedBody to bodyText",
            "else",
            "set mergedBody to mergedBody & return & bodyText",
            "end if",
            "end if",
            "if markerText is not \"\" and mergedBody does not contain markerText then",
            "if mergedBody is \"\" then",
            "set mergedBody to markerText",
            "else",
            "set mergedBody to mergedBody & return & markerText",
            "end if",
            "end if",
            "if mergedBody is not \"\" then set body of targetNote to mergedBody",
            "end if",
            "set noteId to \"\"",
            "set noteNameOut to \"\"",
            "set noteBodyLen to 0",
            "try",
            "set noteId to id of targetNote as text",
            "end try",
            "try",
            "set noteNameOut to name of targetNote as text",
            "end try",
            "try",
            "set noteBodyLen to length of (body of targetNote as text)",
            "end try",
            "end tell",
            "return \"ok|\" & noteId & \"|\" & noteNameOut & \"|\" & noteBodyLen",
            "end run",
        ];
        let out = crate::applescript::run_with_args(&lines, &[text.to_string(), marker])?;
        Ok(Self::parse_notes_write_result(&out))
    }

    fn notes_read_text(goal: Option<&str>) -> Result<String> {
        let marker = Self::preferred_run_scope_marker(goal).unwrap_or_default();
        let lines = [
            "on run argv",
            "set markerText to \"\"",
            "if (count of argv) > 0 then set markerText to item 1 of argv",
            "tell application \"Notes\"",
            "if (count of accounts) = 0 then return \"\"",
            "set ac to item 1 of accounts",
            "if (count of folders of ac) = 0 then return \"\"",
            "set fd to item 1 of folders of ac",
            "if (count of notes of fd) = 0 then return \"\"",
            "set n to missing value",
            "if markerText is not \"\" then",
            "set noteCount to count of notes of fd",
            "repeat with idx from 1 to noteCount by 1",
            "set candidate to item idx of notes of fd",
            "set cName to \"\"",
            "set cBody to \"\"",
            "try",
            "set cName to name of candidate as text",
            "end try",
            "try",
            "set cBody to body of candidate as text",
            "end try",
            "if cName contains markerText or cBody contains markerText then",
            "set n to candidate",
            "exit repeat",
            "end if",
            "end repeat",
            "end if",
            "if n is missing value then",
            "if markerText is not \"\" then return \"__STEER_MARKER_NOT_FOUND__\"",
            "set n to first note of fd",
            "end if",
            "set nName to \"\"",
            "set nBody to \"\"",
            "try",
            "set nName to name of n as text",
            "end try",
            "try",
            "set nBody to body of n as text",
            "end try",
            "if nBody is \"\" then return nName",
            "return nName & return & nBody",
            "end tell",
            "end run",
        ];
        let out = crate::applescript::run_with_args(&lines, &[marker])?;
        if out.trim() == "__STEER_MARKER_NOT_FOUND__" {
            return Err(anyhow::anyhow!("notes marker not found"));
        }
        Ok(out)
    }

    fn parse_textedit_write_result(raw: &str) -> TextEditWriteResult {
        let mut parts = raw.trim().split('|');
        let _status = parts.next().unwrap_or("").trim().to_string();
        TextEditWriteResult {
            doc_id: parts.next().unwrap_or("").trim().to_string(),
            doc_name: parts.next().unwrap_or("").trim().to_string(),
            body_len: parts
                .next()
                .and_then(|v| v.trim().parse::<i64>().ok())
                .unwrap_or(0),
        }
    }

    fn textedit_append_text(text: &str, goal: Option<&str>) -> Result<TextEditWriteResult> {
        let marker = Self::preferred_run_scope_marker(goal).unwrap_or_default();
        let isolate_unscoped = std::env::var("STEER_TEXTEDIT_ISOLATE_UNSCOPED")
            .map(|v| {
                matches!(
                    v.trim().to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes" | "on"
                )
            })
            .unwrap_or(true);
        let lines = [
            "on run argv",
            "set bodyText to item 1 of argv",
            "set markerText to \"\"",
            "if (count of argv) > 1 then set markerText to item 2 of argv",
            "set isolateUnscoped to true",
            "if (count of argv) > 2 then",
            "set isolateArg to item 3 of argv",
            "set isolateUnscoped to (isolateArg is \"1\" or isolateArg is \"true\" or isolateArg is \"yes\" or isolateArg is \"on\")",
            "end if",
            "tell application \"TextEdit\"",
            "activate",
            "if (count of documents) = 0 then make new document",
            "set targetDoc to missing value",
            "if markerText is not \"\" then",
            "set docCount to count of documents",
            "repeat with idx from docCount to 1 by -1",
            "set candidateDoc to item idx of documents",
            "set candidateText to \"\"",
            "try",
            "set candidateText to text of candidateDoc as text",
            "end try",
            "if candidateText contains markerText then",
            "set targetDoc to candidateDoc",
            "exit repeat",
            "end if",
            "end repeat",
            "end if",
            "if targetDoc is missing value then",
            "if markerText is not \"\" or isolateUnscoped then",
            "make new document",
            "set targetDoc to front document",
            "else",
            "set targetDoc to front document",
            "end if",
            "end if",
            "set existingText to \"\"",
            "try",
            "set existingText to text of targetDoc as text",
            "end try",
            "if markerText is not \"\" and existingText does not contain markerText then",
            "if existingText is \"\" then",
            "set existingText to markerText",
            "else",
            "set existingText to existingText & return & markerText",
            "end if",
            "end if",
            "if existingText is \"\" then",
            "set text of targetDoc to bodyText",
            "else",
            "set text of targetDoc to existingText & return & bodyText",
            "end if",
            "set docId to \"\"",
            "set docName to \"\"",
            "set bodyLen to 0",
            "try",
            "set docId to id of targetDoc as text",
            "end try",
            "try",
            "set docName to name of targetDoc as text",
            "end try",
            "try",
            "set bodyLen to length of (text of targetDoc as text)",
            "end try",
            "end tell",
            "return \"ok|\" & docId & \"|\" & docName & \"|\" & bodyLen",
            "end run",
        ];
        let isolate_arg = if isolate_unscoped { "1" } else { "0" };
        let out = crate::applescript::run_with_args(
            &lines,
            &[text.to_string(), marker, isolate_arg.to_string()],
        )?;
        Ok(Self::parse_textedit_write_result(&out))
    }

    fn textedit_read_text(goal: Option<&str>) -> Result<String> {
        let marker = Self::preferred_run_scope_marker(goal).unwrap_or_default();
        let lines = [
            "on run argv",
            "set markerText to \"\"",
            "if (count of argv) > 0 then set markerText to item 1 of argv",
            "tell application \"TextEdit\"",
            "if (count of documents) = 0 then return \"\"",
            "set targetDoc to front document",
            "set markerFound to false",
            "if markerText is not \"\" then",
            "set docCount to count of documents",
            "repeat with idx from docCount to 1 by -1",
            "set candidateDoc to item idx of documents",
            "set candidateText to \"\"",
            "try",
            "set candidateText to text of candidateDoc as text",
            "end try",
            "if candidateText contains markerText then",
            "set targetDoc to candidateDoc",
            "set markerFound to true",
            "exit repeat",
            "end if",
            "end repeat",
            "end if",
            "if markerText is not \"\" and markerFound is false then return \"__STEER_MARKER_NOT_FOUND__\"",
            "set outText to \"\"",
            "try",
            "set outText to text of targetDoc as text",
            "end try",
            "return outText",
            "end tell",
            "end run",
        ];
        let out = crate::applescript::run_with_args(&lines, &[marker])?;
        if out.trim() == "__STEER_MARKER_NOT_FOUND__" {
            return Err(anyhow::anyhow!("textedit marker not found"));
        }
        Ok(out)
    }

    fn goal_mentions_downloads(goal: &str) -> bool {
        let lower = goal.to_lowercase();
        lower.contains("downloads")
            || lower.contains("downloads folder")
            || lower.contains("다운로드")
    }

    fn finder_open_downloads() -> Result<()> {
        let lines = [
            "tell application \"Finder\"",
            "activate",
            "set targetFolder to (path to downloads folder)",
            "if (count of Finder windows) = 0 then",
            "set newWin to make new Finder window",
            "set target of newWin to targetFolder",
            "else",
            "set target of front Finder window to targetFolder",
            "end if",
            "end tell",
            "return \"ok\"",
        ];
        crate::applescript::run_with_args(&lines, &Vec::<String>::new())?;
        Ok(())
    }

    fn normalize_shortcut_parts(key_raw: &str, modifiers_raw: &[String]) -> (String, Vec<String>) {
        let mut key = key_raw.trim().to_lowercase();
        let mut modifiers: Vec<String> = Vec::new();

        let mut push_modifier = |token: &str| {
            let normalized = match token.trim().to_lowercase().as_str() {
                "cmd" | "command" => Some("command".to_string()),
                "shift" => Some("shift".to_string()),
                "option" | "alt" => Some("option".to_string()),
                "control" | "ctrl" => Some("control".to_string()),
                _ => None,
            };
            if let Some(m) = normalized {
                if !modifiers.contains(&m) {
                    modifiers.push(m);
                }
            }
        };

        if key.contains('+') {
            let mut combo_key: Option<String> = None;
            for part in key.split('+').filter(|p| !p.trim().is_empty()) {
                if matches!(
                    part.trim().to_lowercase().as_str(),
                    "cmd" | "command" | "shift" | "option" | "alt" | "control" | "ctrl"
                ) {
                    push_modifier(part);
                } else {
                    combo_key = Some(part.trim().to_lowercase());
                }
            }
            if let Some(k) = combo_key {
                key = k;
            }
        }

        for modifier in modifiers_raw {
            push_modifier(modifier);
        }

        match key.as_str() {
            "paste" | "붙여넣기" => {
                key = "v".to_string();
                push_modifier("command");
            }
            "copy" | "복사" => {
                key = "c".to_string();
                push_modifier("command");
            }
            "select_all" | "selectall" | "전체선택" => {
                key = "a".to_string();
                push_modifier("command");
            }
            "new" | "새로만들기" => {
                key = "n".to_string();
                push_modifier("command");
            }
            _ => {}
        }

        (key, modifiers)
    }

    fn sanitize_evidence_value(value: &str) -> String {
        value
            .replace('\n', " ")
            .replace('\r', " ")
            .replace('|', "/")
            .trim()
            .to_string()
    }

    fn log_evidence(target: &str, event: &str, fields: &[(&str, String)]) {
        let mut line = format!(
            "EVIDENCE|target={}|event={}",
            Self::sanitize_evidence_value(target),
            Self::sanitize_evidence_value(event)
        );
        for (key, value) in fields {
            line.push('|');
            line.push_str(key);
            line.push('=');
            line.push_str(&Self::sanitize_evidence_value(value));
        }
        println!("{}", line);
    }

    fn idempotency_recent_hit(
        session: &crate::session_store::Session,
        key: &str,
        recent_window: usize,
    ) -> bool {
        Self::idempotency_success_hit(session, key, Some(recent_window))
    }

    fn idempotency_success_hit(
        session: &crate::session_store::Session,
        key: &str,
        recent_window: Option<usize>,
    ) -> bool {
        let iter = session.steps.iter().rev();
        let mut inspected = 0usize;
        for step in iter {
            if let Some(limit) = recent_window {
                if inspected >= limit {
                    break;
                }
            }
            inspected += 1;
            if step.status != "success" {
                continue;
            }
            let hit = step
                .data
                .as_ref()
                .and_then(|v| v.get("idempotency_key"))
                .and_then(|v| v.as_str())
                .map(|v| v == key)
                .unwrap_or(false);
            if hit {
                return true;
            }
        }
        false
    }

    fn idempotency_recent_window() -> usize {
        std::env::var("STEER_ACTION_IDEMPOTENCY_WINDOW")
            .ok()
            .and_then(|v| v.trim().parse::<usize>().ok())
            .map(|v| v.clamp(4, 256))
            .unwrap_or(16)
    }

    fn max_cmd_n_attempts_per_app() -> usize {
        std::env::var("STEER_CMD_N_MAX_ATTEMPTS_PER_APP")
            .ok()
            .and_then(|v| v.trim().parse::<usize>().ok())
            .map(|v| v.clamp(1, 6))
            .unwrap_or(2)
    }

    fn session_cmd_n_stats(
        session: &crate::session_store::Session,
        app_name: &str,
    ) -> (usize, usize) {
        let app_lower = app_name.trim().to_lowercase();
        if app_lower.is_empty() {
            return (0, 0);
        }
        let key = format!("shortcut:{}:command+n:new_item", app_lower);
        let mut attempts = 0usize;
        let mut successes = 0usize;

        for step in &session.steps {
            let matches_key = step
                .data
                .as_ref()
                .and_then(|v| v.get("idempotency_key"))
                .and_then(|v| v.as_str())
                .map(|v| v == key)
                .unwrap_or(false);
            if !matches_key {
                continue;
            }
            attempts += 1;
            if step.status == "success" {
                successes += 1;
            }
        }

        (attempts, successes)
    }

    fn is_single_fire_idempotency_key(key: &str) -> bool {
        // New-item creation must not repeat in one run once it has succeeded.
        key.contains(":command+n:new_item")
    }

    fn action_idempotency_key(
        action_type: &str,
        plan: &serde_json::Value,
        goal: &str,
        history: &[String],
    ) -> Option<String> {
        match action_type {
            "open_app" | "switch_app" => {
                let app = plan["name"]
                    .as_str()
                    .or_else(|| plan["app"].as_str())
                    .unwrap_or("")
                    .trim()
                    .to_lowercase();
                if app.is_empty() {
                    None
                } else {
                    Some(format!("{}:{}", action_type, app))
                }
            }
            "open_url" => {
                let url = plan["url"].as_str().unwrap_or("").trim().to_lowercase();
                if url.is_empty() {
                    None
                } else {
                    Some(format!("open_url:{}", url))
                }
            }
            "mail_send" => {
                let recipient = Self::preferred_mail_recipient(Some(goal)).unwrap_or_default();
                let subject = Self::preferred_mail_subject(Some(goal)).unwrap_or_default();
                let scope = Self::preferred_run_scope_marker(Some(goal)).unwrap_or_default();
                let draft_id = plan["draft_id"].as_str().unwrap_or("").trim().to_string();
                let suffix = if !draft_id.is_empty() {
                    format!("draft={}", draft_id)
                } else if !scope.is_empty() {
                    format!("scope={}", scope.to_lowercase())
                } else if !recipient.is_empty() || !subject.is_empty() {
                    format!(
                        "recipient={}::subject={}",
                        recipient.to_lowercase(),
                        subject.to_lowercase()
                    )
                } else {
                    "generic".to_string()
                };
                Some(format!("mail_send:{}", suffix))
            }
            "shortcut" | "key" => {
                let raw_key = plan["key"].as_str().unwrap_or("").to_string();
                let raw_modifiers: Vec<String> = plan["modifiers"]
                    .as_array()
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(|s| s.to_string()))
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                let (key, modifiers) = Self::normalize_shortcut_parts(&raw_key, &raw_modifiers);
                let app_context = plan["app"]
                    .as_str()
                    .map(|v| v.trim().to_lowercase())
                    .filter(|v| !v.is_empty())
                    .or_else(|| {
                        Self::preferred_target_app_from_history("shortcut", plan, history)
                            .map(|v| v.trim().to_lowercase())
                            .filter(|v| !v.is_empty())
                    })
                    .or_else(|| heuristics::goal_primary_app(goal).map(|v| v.trim().to_lowercase()))
                    .or_else(|| {
                        Self::last_opened_app_from_history(history).map(|v| v.trim().to_lowercase())
                    })
                    .unwrap_or_else(|| "unknown".to_string());
                let has_command = modifiers.iter().any(|m| m == "command");
                let has_shift = modifiers.iter().any(|m| m == "shift");
                if key == "n" && has_command {
                    return Some(format!("shortcut:{}:command+n:new_item", app_context));
                }
                if key == "d" && has_command && has_shift {
                    return Some(format!(
                        "shortcut:{}:command+shift+d:mail_send",
                        app_context
                    ));
                }
                None
            }
            _ => None,
        }
    }

    fn extract_quoted_fragments(goal: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut in_double = false;
        let mut in_single = false;
        let mut buf = String::new();
        for ch in goal.chars() {
            if ch == '"' && !in_single {
                if in_double {
                    let v = buf.trim().to_string();
                    if !v.is_empty() {
                        out.push(v);
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
                    let v = buf.trim().to_string();
                    if !v.is_empty() {
                        out.push(v);
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

    fn mail_fallback_body_from_goal(goal: &str) -> String {
        let mut lines = Vec::new();
        for frag in Self::extract_quoted_fragments(goal) {
            if frag.len() >= 3 {
                lines.push(frag);
            }
        }
        lines.join("\n")
    }

    fn preferred_target_app_from_history(
        action_type: &str,
        plan: &serde_json::Value,
        history: &[String],
    ) -> Option<String> {
        let mut target = Self::last_opened_app_from_history(history)?;
        if target.eq_ignore_ascii_case("Calculator") {
            if action_type == "type" {
                let text = plan["text"].as_str().unwrap_or("");
                if !Self::looks_like_calc_expression(text) {
                    if let Some(text_app) = Self::last_text_app_from_history(history) {
                        target = text_app;
                    }
                }
            } else if action_type == "paste" {
                if let Some(text_app) = Self::last_text_app_from_history(history) {
                    target = text_app;
                }
            } else if action_type == "shortcut" || action_type == "key" {
                let key = plan["key"].as_str().unwrap_or("").to_lowercase();
                if key == "v" || key == "n" {
                    if let Some(text_app) = Self::last_text_app_from_history(history) {
                        target = text_app;
                    }
                }
            }
        }
        Some(target)
    }

    fn resolve_shortcut_target_app(
        action_type: &str,
        plan: &serde_json::Value,
        history: &[String],
        goal: &str,
        front_app: &str,
    ) -> String {
        if let Some(app) = plan
            .get("app")
            .and_then(|v| v.as_str())
            .map(|v| v.trim())
            .filter(|v| !v.is_empty())
        {
            return app.to_string();
        }

        if let Some(app) = Self::preferred_target_app_from_history(action_type, plan, history) {
            let trimmed = app.trim();
            if !trimmed.is_empty() {
                return trimmed.to_string();
            }
        }

        let front_trimmed = front_app.trim();
        if !front_trimmed.is_empty() && !Self::is_focus_noise_app(front_trimmed) {
            return front_trimmed.to_string();
        }

        if let Some(app) = heuristics::goal_primary_app(goal) {
            return app.to_string();
        }

        if let Some(app) = Self::last_opened_app_from_history(history) {
            let trimmed = app.trim();
            if !trimmed.is_empty() {
                return trimmed.to_string();
            }
        }

        if !front_trimmed.is_empty() {
            return front_trimmed.to_string();
        }

        "unknown".to_string()
    }

    async fn stabilize_focus_for_action(
        action_type: &str,
        plan: &serde_json::Value,
        history: &[String],
        goal: &str,
    ) {
        // Explicit app hint in plan always wins.
        if let Some(app_name) = plan
            .get("app")
            .and_then(|v| v.as_str())
            .or_else(|| plan.get("target_app").and_then(|v| v.as_str()))
        {
            let _ = heuristics::ensure_app_focus(app_name, 3).await;
            return;
        }

        // Recover focus drift for UI-sensitive actions.
        let ui_sensitive = matches!(
            action_type,
            "snapshot"
                | "click_visual"
                | "read"
                | "type"
                | "shortcut"
                | "key"
                | "scroll"
                | "paste"
                | "copy"
                | "select_all"
                | "switch_app"
        );
        if !ui_sensitive {
            return;
        }

        if let Some(target_app) =
            Self::preferred_target_app_from_history(action_type, plan, history)
        {
            let front = crate::tool_chaining::CrossAppBridge::get_frontmost_app().ok();
            let strict_focus_actions = matches!(
                action_type,
                "type"
                    | "shortcut"
                    | "key"
                    | "paste"
                    | "copy"
                    | "select_all"
                    | "read"
                    | "read_clipboard"
            );
            let need_refocus = match front.as_deref() {
                Some(front_app) => {
                    !front_app.eq_ignore_ascii_case(&target_app)
                        && (Self::is_focus_noise_app(front_app) || strict_focus_actions)
                }
                None => true,
            };

            if need_refocus {
                let _ = heuristics::ensure_app_focus(&target_app, 3).await;
            }
            return;
        }

        // Last fallback only for vision-only actions when no app context exists.
        if matches!(action_type, "snapshot" | "click_visual" | "read") {
            if let Some(target_app) = heuristics::goal_primary_app(goal) {
                let _ = heuristics::ensure_app_focus(target_app, 3).await;
            } else if heuristics::prefer_lucky_only(goal) {
                let _ = heuristics::ensure_app_focus("Safari", 2).await;
                let _ = heuristics::ensure_app_focus("Google Chrome", 2).await;
            }
        }
    }

    pub async fn execute(
        plan: &serde_json::Value,
        driver: &mut VisualDriver,
        llm: Option<&dyn LLMClient>,
        session_steps: &mut Vec<SmartStep>,
        session: &mut crate::session_store::Session,
        history: &mut Vec<String>,
        consecutive_failures: &mut usize,
        last_read_number: &mut Option<String>,
        goal: &str,
    ) -> Result<()> {
        let action_type = plan["action"].as_str().unwrap_or("fail");
        let mut description = format!("Executing {}", action_type);
        let mut action_status_override: Option<&str> = None;
        let mut action_data: Option<serde_json::Value> = None;
        let focus_guard_required = matches!(
            action_type,
            "type"
                | "paste"
                | "copy"
                | "select_all"
                | "shortcut"
                | "key"
                | "open_app"
                | "switch_app"
        );
        let mut focus_guard_target =
            if focus_guard_required && !matches!(action_type, "open_app" | "switch_app") {
                Self::preferred_target_app_from_history(action_type, plan, history)
            } else {
                None
            };
        let idempotency_key = if Self::bool_env_with_default("STEER_ACTION_IDEMPOTENCY", true) {
            Self::action_idempotency_key(action_type, plan, goal, history)
        } else {
            None
        };

        if let Some(key) = idempotency_key.as_deref() {
            let already_done = if Self::is_single_fire_idempotency_key(key) {
                Self::idempotency_success_hit(session, key, None)
            } else {
                Self::idempotency_recent_hit(session, key, Self::idempotency_recent_window())
            };
            if already_done {
                let skip_description = format!("Idempotent skip: {}", key);
                history.push(skip_description.clone());
                session.add_step(
                    action_type,
                    &skip_description,
                    "success",
                    Some(json!({
                        "proof": "idempotent_skip",
                        "idempotency_key": key
                    })),
                );
                let _ = crate::session_store::save_session(session);
                *consecutive_failures = 0;
                return Ok(());
            }
        }

        // Pre-action focus: keep action target app frontmost to prevent drift.
        Self::stabilize_focus_for_action(action_type, plan, history, goal).await;

        // Safari privacy report popover close (pre-step safeguard)
        if action_type == "snapshot" {
            if let Ok(front) = crate::tool_chaining::CrossAppBridge::get_frontmost_app() {
                if front.eq_ignore_ascii_case("Safari") {
                    let close_script = r#"
                        tell application "System Events"
                            tell process "Safari"
                                if exists window 1 then
                                    if exists pop over 1 of window 1 then
                                        try
                                            click button 1 of pop over 1 of window 1
                                        end try
                                    end if
                                end if
                            end tell
                        end tell
                    "#;
                    let _ = applescript::run(close_script);
                }
            }
        }

        match action_type {
            "snapshot" => {
                let mut browser_auto = crate::browser_automation::get_browser_automation();
                match browser_auto.take_snapshot() {
                    Ok(refs) => {
                        let summary =
                            crate::browser_automation::BrowserAutomation::summarize_refs(&refs, 20);
                        description = format!("Captured snapshot refs ({} elements)", refs.len());
                        history.push(summary.clone());
                        session.add_message("tool", &summary);
                        action_data = Some(json!({
                            "proof": "snapshot_refs",
                            "refs": refs.len()
                        }));
                    }
                    Err(e) => {
                        description = format!("snapshot failed: {}", e);
                        action_status_override = Some("failed");
                    }
                }
            }
            "click_ref" => {
                let ref_id = plan["ref"].as_str().unwrap_or("");
                if ref_id.is_empty() {
                    description = "click_ref failed: missing ref".to_string();
                    action_status_override = Some("failed");
                } else {
                    if let Some(app_name) = plan
                        .get("app")
                        .and_then(|v| v.as_str())
                        .or_else(|| plan.get("name").and_then(|v| v.as_str()))
                    {
                        let _ = heuristics::ensure_app_focus(app_name, 1).await;
                    }
                    let front_app = crate::tool_chaining::CrossAppBridge::get_frontmost_app()
                        .unwrap_or_default();
                    let finder_download_target = ref_id
                        .eq_ignore_ascii_case("LeftSidebarDownloads")
                        || Self::goal_mentions_downloads(goal);
                    if finder_download_target
                        && (front_app.eq_ignore_ascii_case("Finder")
                            || plan["app"].as_str() == Some("Finder")
                            || plan["name"].as_str() == Some("Finder"))
                    {
                        match Self::finder_open_downloads() {
                            Ok(_) => {
                                description =
                                    "Opened Downloads folder in Finder (deterministic fallback)"
                                        .to_string();
                                action_status_override = Some("success");
                            }
                            Err(e) => {
                                description = format!(
                                    "click_ref '{}' failed (finder downloads fallback): {}",
                                    ref_id, e
                                );
                                action_status_override = Some("failed");
                            }
                        }
                    } else {
                        let mut browser_auto = crate::browser_automation::get_browser_automation();
                        let click_res = browser_auto.click_by_ref(ref_id, false).or_else(|_| {
                            // Retry once with a fresh snapshot if refs are stale.
                            browser_auto.take_snapshot()?;
                            browser_auto.click_by_ref(ref_id, false)
                        });
                        match click_res {
                            Ok(_) => {
                                description = format!("Clicked ref '{}'", ref_id);
                            }
                            Err(e) => {
                                description = format!("click_ref '{}' failed: {}", ref_id, e);
                                action_status_override = Some("failed");
                            }
                        }
                    }
                }
            }
            "click_visual" => {
                let desc = plan["description"].as_str().unwrap_or("element");
                let looks_like_dialog = heuristics::looks_like_dialog(desc);
                let front_app =
                    crate::tool_chaining::CrossAppBridge::get_frontmost_app().unwrap_or_default();
                let desc_lc = desc.to_lowercase();
                let looks_like_downloads_target =
                    desc_lc.contains("download") || desc.contains("다운로드");
                let looks_like_mail_body_target = desc_lc.contains("message body")
                    || desc_lc.contains("mail body")
                    || desc_lc.contains("compose body")
                    || desc_lc.contains("본문")
                    || desc_lc.contains("메시지");

                if front_app.eq_ignore_ascii_case("Mail") && looks_like_mail_body_target {
                    // Mail body writes are handled by deterministic AppleScript append in paste/type.
                    // Avoid Vision click hard-fails in compose area.
                    description =
                        "Skipped visual click (Mail body target); deterministic body append path will be used"
                            .to_string();
                    action_status_override = Some("success");
                    action_data = Some(json!({
                        "proof": "mail_body_focus_skipped",
                        "target": desc
                    }));
                } else if front_app.eq_ignore_ascii_case("Finder")
                    && Self::goal_mentions_downloads(goal)
                    && looks_like_downloads_target
                {
                    match Self::finder_open_downloads() {
                        Ok(_) => {
                            description =
                                "Opened Downloads folder in Finder (deterministic visual fallback)"
                                    .to_string();
                            action_status_override = Some("success");
                        }
                        Err(e) => {
                            description = format!("Finder downloads fallback failed: {}", e);
                            action_status_override = Some("failed");
                            *consecutive_failures += 1;
                        }
                    }
                } else if looks_like_dialog {
                    let script = r#"
                        tell application "System Events"
                            set frontApp to name of first application process whose frontmost is true
                            tell process frontApp
                                if exists sheet 1 of window 1 then
                                    if exists button "Cancel" of sheet 1 of window 1 then
                                        click button "Cancel" of sheet 1 of window 1
                                    else if exists button "취소" of sheet 1 of window 1 then
                                        click button "취소" of sheet 1 of window 1
                                    else if exists button "닫기" of sheet 1 of window 1 then
                                        click button "닫기" of sheet 1 of window 1
                                    end if
                                end if
                            end tell
                        end tell
                    "#;

                    match applescript::run(script) {
                        Ok(_) => {
                            description = "Closed dialog via button click".to_string();
                            action_status_override = Some("success");
                        }
                        Err(e) => {
                            description = format!("Dialog close failed: {}", e);
                            action_status_override = Some("failed");
                            *consecutive_failures += 1;
                        }
                    }
                } else {
                    let step = SmartStep::new(UiAction::ClickVisual(desc.to_string()), desc);
                    driver.add_step(step);
                    description = format!("Clicked '{}'", desc);
                }
            }
            "read" => {
                let query = plan["query"].as_str().unwrap_or("Describe the screen");
                let mut read_text = String::new();

                if let Some(app_name) = plan.get("app").and_then(|v| v.as_str()) {
                    let _ = heuristics::ensure_app_focus(app_name, 1).await;
                }

                if let Some(brain) = llm {
                    match VisualDriver::capture_screen() {
                        Ok((b64, _)) => match brain.analyze_screen(query, &b64).await {
                            Ok(resp) => {
                                read_text = resp.trim().to_string();
                            }
                            Err(e) => {
                                description = format!("read failed (vision): {}", e);
                                action_status_override = Some("failed");
                            }
                        },
                        Err(e) => {
                            description = format!("read failed (capture): {}", e);
                            action_status_override = Some("failed");
                        }
                    }
                } else {
                    // LLM unavailable: best effort clipboard read from current app.
                    let front_app = crate::tool_chaining::CrossAppBridge::get_frontmost_app()
                        .unwrap_or_default();
                    if Self::is_text_app(&front_app) {
                        let _ = heuristics::focus_text_area(&front_app, false);
                        let _ = std::process::Command::new("osascript")
                            .arg("-e")
                            .arg(r#"tell application "System Events" to keystroke "a" using command down"#)
                            .status();
                        std::thread::sleep(std::time::Duration::from_millis(120));
                    }
                    let _ = std::process::Command::new("osascript")
                        .arg("-e")
                        .arg(r#"tell application "System Events" to keystroke "c" using command down"#)
                        .status();
                    std::thread::sleep(std::time::Duration::from_millis(120));
                    read_text =
                        crate::tool_chaining::CrossAppBridge::get_clipboard().unwrap_or_default();
                }

                if action_status_override != Some("failed") {
                    if let Some(num) = Self::extract_first_number(&read_text) {
                        *last_read_number = Some(num.clone());
                        history.push(format!("READ_NUMBER: {}", num));
                    }
                    let preview = Self::preview_text(&read_text, 180);
                    description = format!("Read '{}' -> {}", query, preview);
                    session.add_message("tool", &format!("read: {}", preview));
                    history.push(format!("READ_RESULT: {}", preview));
                }
            }
            "type" => {
                let mut text = plan["text"].as_str().unwrap_or("").to_string();
                let mut forced_app = false;
                if let Some(app_name) = plan.get("app").and_then(|v| v.as_str()) {
                    let _ = heuristics::ensure_app_focus(app_name, 1).await;
                    forced_app = true;
                } else if let Some(target_app) =
                    Self::preferred_target_app_from_history("type", plan, history)
                {
                    let _ = heuristics::ensure_app_focus(&target_app, 2).await;
                }

                let looks_like_calc = Self::looks_like_calc_expression(&text);
                if !forced_app && looks_like_calc {
                    if let Ok(front) = crate::tool_chaining::CrossAppBridge::get_frontmost_app() {
                        if !front.eq_ignore_ascii_case("Calculator") {
                            let _ = heuristics::ensure_app_focus("Calculator", 3).await;
                        }
                    }
                }

                if let Ok(app_name) = crate::tool_chaining::CrossAppBridge::get_frontmost_app() {
                    if app_name.eq_ignore_ascii_case("Calculator") {
                        let mut cleaned = text
                            .replace('×', "*")
                            .replace('x', "*")
                            .replace('X', "*")
                            .replace(' ', "");

                        if cleaned.chars().all(|c| c.is_ascii_digit()) {
                            if let Some(num) = last_read_number.as_ref() {
                                if num.contains('.') {
                                    cleaned = num.clone();
                                }
                            }
                        }

                        if (cleaned.contains('*')
                            || cleaned.contains('+')
                            || cleaned.contains('-')
                            || cleaned.contains('/'))
                            && !cleaned.ends_with('=')
                        {
                            cleaned.push('=');
                        }
                        text = cleaned;
                    }

                    if app_name.eq_ignore_ascii_case("Mail") {
                        let draft_id = Self::mail_ensure_draft(Some(goal), history)
                            .ok()
                            .filter(|v| !v.trim().is_empty());
                        let recipient_hint = Self::preferred_mail_recipient(Some(goal));
                        if let Err(e) =
                            Self::mail_set_recipient_if_missing(Some(goal), draft_id.as_deref())
                        {
                            description = format!("Type failed (mail recipient): {}", e);
                            action_status_override = Some("failed");
                        } else if let Some(recipient) = recipient_hint {
                            let draft_for_log = draft_id.clone().unwrap_or_default();
                            Self::log_evidence(
                                "mail",
                                "write",
                                &[
                                    ("status", "confirmed".to_string()),
                                    ("recipient", recipient),
                                    ("draft_id", draft_for_log),
                                ],
                            );
                        }
                        let subject_already_set = history
                            .iter()
                            .any(|h| h.to_lowercase().contains("(mail subject)"));
                        let prefer_subject = heuristics::looks_like_subject(&text)
                            || (!subject_already_set && !text.contains('\n') && text.len() <= 120);
                        if action_status_override != Some("failed") && prefer_subject {
                            match Self::mail_set_subject(&text, draft_id.as_deref()) {
                                Ok(target_draft_id) => {
                                    Self::remember_mail_draft_id(history, &target_draft_id);
                                    description = format!("Typed '{}' (mail subject)", text);
                                    action_data = Some(json!({
                                        "proof": "mail_subject_set",
                                        "text_len": text.chars().count()
                                    }));
                                    Self::log_evidence(
                                        "mail",
                                        "write",
                                        &[
                                            ("status", "confirmed".to_string()),
                                            ("subject", text.clone()),
                                            ("draft_id", target_draft_id),
                                        ],
                                    );
                                }
                                Err(e) => {
                                    description = format!("Type failed (mail subject): {}", e);
                                    action_status_override = Some("failed");
                                }
                            }
                        } else if action_status_override != Some("failed") {
                            match Self::mail_append_body(&text, draft_id.as_deref()) {
                                Ok((target_draft_id, mut readback_len)) => {
                                    Self::remember_mail_draft_id(history, &target_draft_id);
                                    if readback_len <= 2 {
                                        let forced_text = Self::extract_quoted_fragments(goal)
                                            .into_iter()
                                            .filter(|s| s.len() >= 3)
                                            .collect::<Vec<_>>()
                                            .join("\n");
                                        if !forced_text.trim().is_empty()
                                            && forced_text.trim() != text.trim()
                                        {
                                            if let Ok((forced_draft_id, forced_len)) =
                                                Self::mail_append_body(
                                                    &forced_text,
                                                    Some(target_draft_id.as_str()),
                                                )
                                            {
                                                Self::remember_mail_draft_id(
                                                    history,
                                                    &forced_draft_id,
                                                );
                                                readback_len = forced_len;
                                            }
                                        }
                                    }
                                    if readback_len <= 2 {
                                        description =
                                            "Type failed (mail body): empty readback after append"
                                                .to_string();
                                        action_status_override = Some("failed");
                                    } else {
                                        description = format!("Typed '{}' (mail body)", text);
                                        action_data = Some(json!({
                                            "proof": "mail_body_appended",
                                            "text_len": text.chars().count(),
                                            "readback_len": readback_len
                                        }));
                                        Self::log_evidence(
                                            "mail",
                                            "write",
                                            &[
                                                ("status", "confirmed".to_string()),
                                                ("body_len", readback_len.to_string()),
                                                ("draft_id", target_draft_id),
                                            ],
                                        );
                                    }
                                }
                                Err(e) => {
                                    description = format!("Type failed (mail body): {}", e);
                                    action_status_override = Some("failed");
                                }
                            }
                        }
                        if action_status_override != Some("failed") {
                            action_status_override = Some("success");
                        }
                    } else if app_name.eq_ignore_ascii_case("Notes") {
                        let mut write_text = text.clone();
                        let run_scope_marker =
                            Self::preferred_run_scope_marker(Some(goal)).unwrap_or_default();
                        // Notes typing must remain app-local. Do not append every quoted token
                        // from the whole multi-app goal, which can contaminate note content.
                        if write_text.trim().is_empty() {
                            let quoted = Self::extract_quoted_fragments(goal)
                                .into_iter()
                                .filter(|s| {
                                    s.len() >= 3
                                        && !s.to_lowercase().contains("status:")
                                        && !s.to_lowercase().contains("cmd+")
                                })
                                .collect::<Vec<_>>();
                            if !quoted.is_empty() {
                                write_text = quoted.join("\n");
                            }
                        }
                        if !run_scope_marker.is_empty() && !write_text.contains(&run_scope_marker) {
                            if write_text.trim().is_empty() {
                                write_text = run_scope_marker.clone();
                            } else {
                                write_text =
                                    format!("{}\n{}", write_text.trim_end(), run_scope_marker);
                            }
                        }
                        match Self::notes_write_text(&write_text, Some(goal)) {
                            Ok(write_result) => {
                                description = format!(
                                    "Typed '{}' (notes body note_id={} note_name={})",
                                    write_text, write_result.note_id, write_result.note_name
                                );
                                action_status_override = Some("success");
                                action_data = Some(json!({
                                    "proof": "notes_write_text",
                                    "text_len": write_text.chars().count(),
                                    "note_id": write_result.note_id,
                                    "note_name": write_result.note_name,
                                    "note_body_len": write_result.body_len
                                }));
                                Self::log_evidence(
                                    "notes",
                                    "write",
                                    &[
                                        ("status", "confirmed".to_string()),
                                        ("note_id", write_result.note_id.clone()),
                                        ("note_name", write_result.note_name.clone()),
                                        ("body_len", write_result.body_len.to_string()),
                                    ],
                                );
                            }
                            Err(e) => {
                                description = format!("Type failed (notes body): {}", e);
                                action_status_override = Some("failed");
                            }
                        }
                    } else if app_name.eq_ignore_ascii_case("TextEdit") {
                        match Self::textedit_append_text(&text, Some(goal)) {
                            Ok(write_result) => {
                                description = format!(
                                    "Typed '{}' (textedit body doc_id={} doc_name={})",
                                    text, write_result.doc_id, write_result.doc_name
                                );
                                action_status_override = Some("success");
                                action_data = Some(json!({
                                    "proof": "textedit_append_text",
                                    "text_len": text.chars().count(),
                                    "doc_id": write_result.doc_id,
                                    "doc_name": write_result.doc_name,
                                    "doc_body_len": write_result.body_len
                                }));
                                Self::log_evidence(
                                    "textedit",
                                    "write",
                                    &[
                                        ("status", "confirmed".to_string()),
                                        ("doc_id", write_result.doc_id.clone()),
                                        ("doc_name", write_result.doc_name.clone()),
                                        ("body_len", write_result.body_len.to_string()),
                                    ],
                                );
                            }
                            Err(e) => {
                                description = format!("Type failed (textedit body): {}", e);
                                action_status_override = Some("failed");
                            }
                        }
                    }
                }

                if !description.contains("(mail subject)")
                    && !description.contains("(mail body)")
                    && !description.contains("(notes body)")
                    && !description.contains("(textedit body)")
                    && action_status_override != Some("failed")
                {
                    let step = SmartStep::new(UiAction::Type(text.to_string()), "Typing");
                    driver.add_step(step);
                    description = format!("Typed '{}'", text);
                }
            }
            "key" => {
                let key_raw = plan["key"].as_str().unwrap_or("return");
                let key_norm = key_raw.trim().to_lowercase().replace(' ', "");
                let mut shortcut_modifiers: Vec<String> = Vec::new();
                let mut shortcut_key: Option<String> = None;

                if let Some(app_name) = plan.get("app").and_then(|v| v.as_str()) {
                    let _ = crate::tool_chaining::CrossAppBridge::switch_to_app(app_name);
                    let _ = heuristics::ensure_app_focus(app_name, 5).await;
                } else if let Some(target_app) =
                    Self::preferred_target_app_from_history("key", plan, history)
                {
                    let _ = heuristics::ensure_app_focus(&target_app, 5).await;
                }

                if key_norm.contains('+') {
                    for part in key_norm.split('+').filter(|p| !p.is_empty()) {
                        match part {
                            "cmd" | "command" => shortcut_modifiers.push("command".to_string()),
                            "shift" => shortcut_modifiers.push("shift".to_string()),
                            "option" | "alt" => shortcut_modifiers.push("option".to_string()),
                            "control" | "ctrl" => shortcut_modifiers.push("control".to_string()),
                            other => shortcut_key = Some(other.to_string()),
                        }
                    }
                }

                if key_norm == "escape" || key_norm == "esc" {
                    let script = "tell application \"System Events\" to key code 53";
                    let _ = std::process::Command::new("osascript")
                        .arg("-e")
                        .arg(script)
                        .status();
                    description = "Pressed 'escape'".to_string();
                } else if !shortcut_modifiers.is_empty() && shortcut_key.is_some() {
                    let key = shortcut_key.unwrap_or_default();
                    let has_command = shortcut_modifiers
                        .iter()
                        .any(|m| m.eq_ignore_ascii_case("command"));
                    let has_shift = shortcut_modifiers
                        .iter()
                        .any(|m| m.eq_ignore_ascii_case("shift"));
                    let is_cmd_n = key == "n" && has_command;
                    let is_cmd_shift_d = key == "d" && has_command && has_shift;
                    let front_app = crate::tool_chaining::CrossAppBridge::get_frontmost_app()
                        .unwrap_or_default();
                    let cmd_n_target_app =
                        Self::resolve_shortcut_target_app("key", plan, history, goal, &front_app);
                    let cmd_n_single_fire_key = format!(
                        "shortcut:{}:command+n:new_item",
                        cmd_n_target_app.to_lowercase()
                    );
                    let cmd_n_history_skip = is_cmd_n
                        && !cmd_n_target_app.is_empty()
                        && Self::should_skip_redundant_cmd_n(history, &cmd_n_target_app);
                    let cmd_n_session_skip = is_cmd_n
                        && !cmd_n_target_app.is_empty()
                        && Self::session_has_single_fire_new_item(session, &cmd_n_single_fire_key);
                    let cmd_n_mail_draft_tracked = is_cmd_n
                        && cmd_n_target_app.eq_ignore_ascii_case("Mail")
                        && Self::has_tracked_mail_draft(history);
                    let cmd_n_redundant_skip =
                        cmd_n_history_skip || cmd_n_session_skip || cmd_n_mail_draft_tracked;
                    let (cmd_n_attempts, cmd_n_successes) = if is_cmd_n {
                        Self::session_cmd_n_stats(session, &cmd_n_target_app)
                    } else {
                        (0, 0)
                    };
                    let cmd_n_attempt_guard_hit = is_cmd_n
                        && !cmd_n_target_app.is_empty()
                        && cmd_n_successes == 0
                        && cmd_n_attempts >= Self::max_cmd_n_attempts_per_app();
                    let cmd_n_recent_created = if is_cmd_n {
                        Self::history_recent_cmd_n_created_count(
                            history,
                            &cmd_n_target_app,
                            Self::cmd_n_window_flood_history_window(),
                        )
                    } else {
                        0
                    };
                    let cmd_n_flood_limit = if is_cmd_n {
                        Self::cmd_n_window_flood_limit_for_app(&cmd_n_target_app)
                    } else {
                        Self::cmd_n_window_flood_limit()
                    };
                    let cmd_n_window_flood_hit = is_cmd_n
                        && cmd_n_recent_created >= cmd_n_flood_limit;

                    if is_cmd_n
                        && (cmd_n_target_app.is_empty()
                            || cmd_n_target_app.eq_ignore_ascii_case("unknown"))
                    {
                        description =
                            "Shortcut 'n' + [command] blocked (target app unresolved)".to_string();
                        action_status_override = Some("failed");
                        action_data = Some(json!({
                            "proof": "cmd_n_target_unknown_block",
                            "shortcut": "cmd+n",
                            "front_app": front_app
                        }));
                    } else if cmd_n_window_flood_hit {
                        description = format!(
                            "Shortcut '{}' + {:?} blocked (window flood guard, recent_created={})",
                            key, shortcut_modifiers, cmd_n_recent_created
                        );
                        action_status_override = Some("failed");
                        action_data = Some(json!({
                            "proof": "cmd_n_window_flood_block",
                            "front_app": cmd_n_target_app,
                            "shortcut": "cmd+n",
                            "recent_created": cmd_n_recent_created,
                            "flood_limit": cmd_n_flood_limit
                        }));
                    } else if cmd_n_attempt_guard_hit {
                        description = format!(
                            "Shortcut '{}' + {:?} blocked (cmd+n loop guard in {}, attempts={})",
                            key, shortcut_modifiers, cmd_n_target_app, cmd_n_attempts
                        );
                        action_status_override = Some("failed");
                        action_data = Some(json!({
                            "proof": "cmd_n_loop_guard_block",
                            "front_app": cmd_n_target_app,
                            "shortcut": "cmd+n",
                            "attempts": cmd_n_attempts
                        }));
                    } else if is_cmd_n && !cmd_n_target_app.is_empty() && cmd_n_redundant_skip {
                        let skip_reason = if cmd_n_mail_draft_tracked {
                            "mail_draft_tracked"
                        } else if cmd_n_session_skip {
                            "session_single_fire"
                        } else {
                            "history_redundant"
                        };
                        description = format!(
                            "Shortcut '{}' + {:?} skipped (redundant new item in {})",
                            key, shortcut_modifiers, cmd_n_target_app
                        );
                        action_status_override = Some("success");
                        action_data = Some(json!({
                            "proof": "redundant_new_item_skip",
                            "front_app": cmd_n_target_app,
                            "shortcut": "cmd+n",
                            "reason": skip_reason
                        }));
                    } else if is_cmd_n && cmd_n_target_app.eq_ignore_ascii_case("Mail") {
                        let _ = heuristics::ensure_app_focus("Mail", 5).await;
                        match Self::mail_ensure_draft(Some(goal), history) {
                            Ok(draft_id) => {
                                Self::remember_mail_draft_id(history, &draft_id);
                                description = format!(
                                    "Shortcut '{}' + {:?} (Created new item)",
                                    key, shortcut_modifiers
                                );
                                action_status_override = Some("success");
                                action_data = Some(json!({
                                    "proof": "mail_draft_ready",
                                    "front_app": "Mail"
                                }));
                            }
                            Err(e) => {
                                description = format!("shortcut cmd+n (mail) failed: {}", e);
                                action_status_override = Some("failed");
                            }
                        }
                    } else if is_cmd_shift_d && front_app.eq_ignore_ascii_case("Mail") {
                        let draft_id = Self::mail_current_draft_id(history);
                        info!("      📧 [MailSend] key-path draft_id={:?}", draft_id);
                        match Self::mail_send_latest_message(Some(goal), draft_id.as_deref()) {
                            Ok(raw_result) => {
                                let send_result = Self::parse_mail_send_result(&raw_result);
                                let outgoing_after = send_result
                                    .outgoing_after
                                    .unwrap_or_else(|| Self::mail_outgoing_count().unwrap_or(-1));
                                let policy_error =
                                    Self::enforce_mail_send_policy(Some(goal), &send_result).err();
                                if policy_error.is_none() && send_result.status == "sent_confirmed"
                                {
                                    description = format!(
                                        "Shortcut '{}' + {:?} (Mail sent)",
                                        key, shortcut_modifiers
                                    );
                                    action_status_override = Some("success");
                                } else if let Some(policy_err) = policy_error.as_ref() {
                                    description = format!(
                                        "Shortcut '{}' + {:?} (mail blocked by outbound policy: {})",
                                        key, shortcut_modifiers, policy_err
                                    );
                                    action_status_override = Some("failed");
                                } else {
                                    description = format!(
                                        "Shortcut '{}' + {:?} (mail send blocked: {})",
                                        key, shortcut_modifiers, raw_result
                                    );
                                    action_status_override = Some("failed");
                                }
                                action_data = Some(json!({
                                    "proof": "mail_send",
                                    "result": raw_result,
                                    "send_status": send_result.status,
                                    "outgoing_before": send_result.outgoing_before,
                                    "outgoing_after": outgoing_after,
                                    "recipient": send_result.recipient,
                                    "subject": send_result.subject,
                                    "draft_id": send_result.draft_id,
                                    "body_len": send_result.body_len,
                                    "outbound_policy_error": policy_error.as_ref().map(|e| e.to_string()).unwrap_or_default()
                                }));
                                println!(
                                    "MAIL_SEND_PROOF|status={}|recipient={}|subject={}|body_len={}|draft_id={}",
                                    send_result.status,
                                    send_result.recipient,
                                    send_result.subject,
                                    send_result.body_len.unwrap_or(-1),
                                    send_result.draft_id
                                );
                                Self::log_evidence(
                                    "mail",
                                    "send",
                                    &[
                                        ("status", send_result.status.clone()),
                                        ("recipient", send_result.recipient.clone()),
                                        ("subject", send_result.subject.clone()),
                                        (
                                            "body_len",
                                            send_result.body_len.unwrap_or(-1).to_string(),
                                        ),
                                        ("draft_id", send_result.draft_id.clone()),
                                        (
                                            "outbound_policy",
                                            policy_error
                                                .as_ref()
                                                .map(|e| format!("blocked:{}", e))
                                                .unwrap_or_else(|| "pass".to_string()),
                                        ),
                                    ],
                                );
                                if policy_error.is_none()
                                    && send_result.status == "sent_confirmed"
                                    && !send_result.draft_id.trim().is_empty()
                                {
                                    if let Ok(removed) = Self::mail_cleanup_marker_outgoing(
                                        Some(goal),
                                        Some(send_result.draft_id.as_str()),
                                    ) {
                                        if removed > 0 {
                                            Self::log_evidence(
                                                "mail",
                                                "cleanup",
                                                &[("removed", removed.to_string())],
                                            );
                                        }
                                    }
                                }
                            }
                            Err(e) => {
                                description = format!(
                                    "Shortcut '{}' + {:?} (mail send failed: {})",
                                    key, shortcut_modifiers, e
                                );
                                action_status_override = Some("failed");
                            }
                        }
                    } else {
                        let step = SmartStep::new(
                            UiAction::KeyboardShortcut(key.clone(), shortcut_modifiers.clone()),
                            "Shortcut",
                        );
                        driver.add_step(step);
                        description = format!("Shortcut '{}' + {:?}", key, shortcut_modifiers);
                    }
                } else {
                    let key_char = match key_norm.as_str() {
                        "return" | "enter" => "\r",
                        "tab" => "\t",
                        _ => key_raw,
                    };
                    let step = SmartStep::new(UiAction::Type(key_char.to_string()), "Pressing Key");
                    driver.add_step(step);
                    description = format!("Pressed '{}'", key_raw);
                }
            }
            "shortcut" => {
                let raw_key = plan["key"].as_str().unwrap_or("").to_string();
                let raw_modifiers: Vec<String> = if let Some(arr) = plan["modifiers"].as_array() {
                    arr.iter()
                        .map(|v| v.as_str().unwrap_or("").to_string())
                        .collect()
                } else {
                    Vec::new()
                };
                let (key, modifiers) = Self::normalize_shortcut_parts(&raw_key, &raw_modifiers);

                if key.is_empty() {
                    description = "shortcut failed: empty key".to_string();
                    action_status_override = Some("failed");
                } else {
                    if let Some(app_name) = plan.get("app").and_then(|v| v.as_str()) {
                        let _ = crate::tool_chaining::CrossAppBridge::switch_to_app(app_name);
                        let _ = heuristics::ensure_app_focus(app_name, 5).await;
                    } else if let Some(target_app) =
                        Self::preferred_target_app_from_history("shortcut", plan, history)
                    {
                        let _ = heuristics::ensure_app_focus(&target_app, 5).await;
                    }

                    let is_cmd_n =
                        key == "n" && modifiers.iter().any(|m| m.eq_ignore_ascii_case("command"));
                    let is_cmd_shift_d = key == "d"
                        && modifiers.iter().any(|m| m.eq_ignore_ascii_case("command"))
                        && modifiers.iter().any(|m| m.eq_ignore_ascii_case("shift"));
                    let is_cmd_s =
                        key == "s" && modifiers.iter().any(|m| m.eq_ignore_ascii_case("command"));
                    let front_app = crate::tool_chaining::CrossAppBridge::get_frontmost_app()
                        .unwrap_or_default();
                    let cmd_n_target_app = Self::resolve_shortcut_target_app(
                        "shortcut", plan, history, goal, &front_app,
                    );
                    let cmd_n_single_fire_key = format!(
                        "shortcut:{}:command+n:new_item",
                        cmd_n_target_app.to_lowercase()
                    );
                    let cmd_n_history_skip = is_cmd_n
                        && !cmd_n_target_app.is_empty()
                        && Self::should_skip_redundant_cmd_n(history, &cmd_n_target_app);
                    let cmd_n_session_skip = is_cmd_n
                        && !cmd_n_target_app.is_empty()
                        && Self::session_has_single_fire_new_item(session, &cmd_n_single_fire_key);
                    let cmd_n_mail_draft_tracked = is_cmd_n
                        && cmd_n_target_app.eq_ignore_ascii_case("Mail")
                        && Self::has_tracked_mail_draft(history);
                    let cmd_n_redundant_skip =
                        cmd_n_history_skip || cmd_n_session_skip || cmd_n_mail_draft_tracked;
                    let (cmd_n_attempts, cmd_n_successes) = if is_cmd_n {
                        Self::session_cmd_n_stats(session, &cmd_n_target_app)
                    } else {
                        (0, 0)
                    };
                    let cmd_n_attempt_guard_hit = is_cmd_n
                        && !cmd_n_target_app.is_empty()
                        && cmd_n_successes == 0
                        && cmd_n_attempts >= Self::max_cmd_n_attempts_per_app();
                    let cmd_n_recent_created = if is_cmd_n {
                        Self::history_recent_cmd_n_created_count(
                            history,
                            &cmd_n_target_app,
                            Self::cmd_n_window_flood_history_window(),
                        )
                    } else {
                        0
                    };
                    let cmd_n_flood_limit = if is_cmd_n {
                        Self::cmd_n_window_flood_limit_for_app(&cmd_n_target_app)
                    } else {
                        Self::cmd_n_window_flood_limit()
                    };
                    let cmd_n_window_flood_hit = is_cmd_n
                        && cmd_n_recent_created >= cmd_n_flood_limit;
                    if is_cmd_n
                        && (cmd_n_target_app.is_empty()
                            || cmd_n_target_app.eq_ignore_ascii_case("unknown"))
                    {
                        description =
                            "Shortcut 'n' + [command] blocked (target app unresolved)".to_string();
                        action_status_override = Some("failed");
                        action_data = Some(json!({
                            "proof": "cmd_n_target_unknown_block",
                            "shortcut": "cmd+n",
                            "front_app": front_app
                        }));
                    } else if cmd_n_window_flood_hit {
                        description = format!(
                            "Shortcut '{}' + {:?} blocked (window flood guard, recent_created={})",
                            key, modifiers, cmd_n_recent_created
                        );
                        action_status_override = Some("failed");
                        action_data = Some(json!({
                            "proof": "cmd_n_window_flood_block",
                            "front_app": cmd_n_target_app,
                            "shortcut": "cmd+n",
                            "recent_created": cmd_n_recent_created,
                            "flood_limit": cmd_n_flood_limit
                        }));
                    } else if cmd_n_attempt_guard_hit {
                        description = format!(
                            "Shortcut '{}' + {:?} blocked (cmd+n loop guard in {}, attempts={})",
                            key, modifiers, cmd_n_target_app, cmd_n_attempts
                        );
                        action_status_override = Some("failed");
                        action_data = Some(json!({
                            "proof": "cmd_n_loop_guard_block",
                            "front_app": cmd_n_target_app,
                            "shortcut": "cmd+n",
                            "attempts": cmd_n_attempts
                        }));
                    } else if is_cmd_n && !cmd_n_target_app.is_empty() && cmd_n_redundant_skip {
                        let skip_reason = if cmd_n_mail_draft_tracked {
                            "mail_draft_tracked"
                        } else if cmd_n_session_skip {
                            "session_single_fire"
                        } else {
                            "history_redundant"
                        };
                        description = format!(
                            "Shortcut '{}' + {:?} skipped (redundant new item in {})",
                            key, modifiers, cmd_n_target_app
                        );
                        action_status_override = Some("success");
                        action_data = Some(json!({
                            "proof": "redundant_new_item_skip",
                            "front_app": cmd_n_target_app,
                            "shortcut": "cmd+n",
                            "reason": skip_reason
                        }));
                    } else if is_cmd_n && cmd_n_target_app.eq_ignore_ascii_case("Mail") {
                        let _ = heuristics::ensure_app_focus("Mail", 5).await;
                        match Self::mail_ensure_draft(Some(goal), history) {
                            Ok(draft_id) => {
                                Self::remember_mail_draft_id(history, &draft_id);
                                description = format!(
                                    "Shortcut '{}' + {:?} (Created new item)",
                                    key, modifiers
                                );
                                action_status_override = Some("success");
                                action_data = Some(json!({
                                    "proof": "mail_draft_ready",
                                    "front_app": "Mail"
                                }));
                            }
                            Err(e) => {
                                description = format!("shortcut cmd+n (mail) failed: {}", e);
                                action_status_override = Some("failed");
                            }
                        }
                    } else if is_cmd_shift_d && front_app.eq_ignore_ascii_case("Mail") {
                        let draft_id = Self::mail_current_draft_id(history);
                        info!("      📧 [MailSend] shortcut-path draft_id={:?}", draft_id);
                        match Self::mail_send_latest_message(Some(goal), draft_id.as_deref()) {
                            Ok(raw_result) => {
                                let send_result = Self::parse_mail_send_result(&raw_result);
                                let outgoing_after = send_result
                                    .outgoing_after
                                    .unwrap_or_else(|| Self::mail_outgoing_count().unwrap_or(-1));
                                let policy_error =
                                    Self::enforce_mail_send_policy(Some(goal), &send_result).err();
                                if policy_error.is_none() && send_result.status == "sent_confirmed"
                                {
                                    description =
                                        format!("Shortcut '{}' + {:?} (Mail sent)", key, modifiers);
                                    action_status_override = Some("success");
                                } else if let Some(policy_err) = policy_error.as_ref() {
                                    description = format!(
                                        "Shortcut '{}' + {:?} (mail blocked by outbound policy: {})",
                                        key, modifiers, policy_err
                                    );
                                    action_status_override = Some("failed");
                                } else {
                                    description = format!(
                                        "Shortcut '{}' + {:?} (mail send blocked: {})",
                                        key, modifiers, raw_result
                                    );
                                    action_status_override = Some("failed");
                                }
                                action_data = Some(json!({
                                    "proof": "mail_send",
                                    "result": raw_result,
                                    "send_status": send_result.status,
                                    "outgoing_before": send_result.outgoing_before,
                                    "outgoing_after": outgoing_after,
                                    "recipient": send_result.recipient,
                                    "subject": send_result.subject,
                                    "draft_id": send_result.draft_id,
                                    "body_len": send_result.body_len,
                                    "outbound_policy_error": policy_error.as_ref().map(|e| e.to_string()).unwrap_or_default()
                                }));
                                println!(
                                    "MAIL_SEND_PROOF|status={}|recipient={}|subject={}|body_len={}|draft_id={}",
                                    send_result.status,
                                    send_result.recipient,
                                    send_result.subject,
                                    send_result.body_len.unwrap_or(-1),
                                    send_result.draft_id
                                );
                                Self::log_evidence(
                                    "mail",
                                    "send",
                                    &[
                                        ("status", send_result.status.clone()),
                                        ("recipient", send_result.recipient.clone()),
                                        ("subject", send_result.subject.clone()),
                                        (
                                            "body_len",
                                            send_result.body_len.unwrap_or(-1).to_string(),
                                        ),
                                        ("draft_id", send_result.draft_id.clone()),
                                        (
                                            "outbound_policy",
                                            policy_error
                                                .as_ref()
                                                .map(|e| format!("blocked:{}", e))
                                                .unwrap_or_else(|| "pass".to_string()),
                                        ),
                                    ],
                                );
                                if policy_error.is_none()
                                    && send_result.status == "sent_confirmed"
                                    && !send_result.draft_id.trim().is_empty()
                                {
                                    if let Ok(removed) = Self::mail_cleanup_marker_outgoing(
                                        Some(goal),
                                        Some(send_result.draft_id.as_str()),
                                    ) {
                                        if removed > 0 {
                                            Self::log_evidence(
                                                "mail",
                                                "cleanup",
                                                &[("removed", removed.to_string())],
                                            );
                                        }
                                    }
                                }
                            }
                            Err(e) => {
                                description = format!(
                                    "Shortcut '{}' + {:?} (mail send failed: {})",
                                    key, modifiers, e
                                );
                                action_status_override = Some("failed");
                            }
                        }
                    } else {
                        let step = SmartStep::new(
                            UiAction::KeyboardShortcut(key.clone(), modifiers.clone()),
                            "Shortcut",
                        );
                        driver.add_step(step);
                        if is_cmd_n {
                            description =
                                format!("Shortcut '{}' + {:?} (Created new item)", key, modifiers);
                        } else if is_cmd_s && front_app.eq_ignore_ascii_case("TextEdit") {
                            description =
                                format!("Shortcut '{}' + {:?} (TextEdit saved)", key, modifiers);
                            action_data = Some(json!({
                                "proof": "textedit_save",
                                "front_app": "TextEdit"
                            }));
                            Self::log_evidence(
                                "textedit",
                                "save",
                                &[("status", "confirmed".to_string())],
                            );
                        } else {
                            description = format!("Shortcut '{}' + {:?}", key, modifiers);
                        }
                    }
                }
            }
            "mail_send" => {
                let front_app =
                    crate::tool_chaining::CrossAppBridge::get_frontmost_app().unwrap_or_default();
                if !front_app.eq_ignore_ascii_case("Mail") {
                    let _ = heuristics::ensure_app_focus("Mail", 3).await;
                }
                let draft_id = Self::mail_current_draft_id(history);
                info!("      📧 [MailSend] action-path draft_id={:?}", draft_id);
                match Self::mail_send_latest_message(Some(goal), draft_id.as_deref()) {
                    Ok(raw_result) => {
                        let mut send_result = Self::parse_mail_send_result(&raw_result);
                        let mut result_raw = raw_result.clone();
                        let mut fresh_recovery_used = false;
                        let allow_fresh_recovery =
                            Self::bool_env_with_default("STEER_MAIL_ALLOW_FRESH_RECOVERY", false)
                                && (Self::is_test_mode_enabled()
                                    || Self::preferred_run_scope_marker(Some(goal)).is_some());
                        if allow_fresh_recovery
                            && send_result.status != "sent_confirmed"
                            && !Self::mail_fresh_recovery_used(history)
                            && matches!(
                                send_result.status.as_str(),
                                "missing_marker" | "empty_body" | "draft_not_found" | "no_draft"
                            )
                        {
                            Self::mark_mail_fresh_recovery_used(history);
                            if let Ok(fresh_raw) = Self::mail_send_fresh_from_goal(goal, history) {
                                result_raw = fresh_raw;
                                send_result = Self::parse_mail_send_result(&result_raw);
                                fresh_recovery_used = true;
                            }
                        }
                        let outgoing_after = send_result
                            .outgoing_after
                            .unwrap_or_else(|| Self::mail_outgoing_count().unwrap_or(-1));
                        let policy_error =
                            Self::enforce_mail_send_policy(Some(goal), &send_result).err();
                        if policy_error.is_none() && send_result.status == "sent_confirmed" {
                            description = "Mail send completed".to_string();
                            action_status_override = Some("success");
                        } else if let Some(policy_err) = policy_error.as_ref() {
                            description =
                                format!("Mail send blocked by outbound policy: {}", policy_err);
                            action_status_override = Some("failed");
                        } else {
                            description = format!("Mail send blocked: {}", result_raw);
                            action_status_override = Some("failed");
                        }
                        action_data = Some(json!({
                            "proof": "mail_send",
                            "result": result_raw,
                            "send_status": send_result.status,
                            "outgoing_before": send_result.outgoing_before,
                            "outgoing_after": outgoing_after,
                            "recipient": send_result.recipient,
                            "subject": send_result.subject,
                            "draft_id": send_result.draft_id,
                            "body_len": send_result.body_len,
                            "fresh_recovery_used": fresh_recovery_used,
                            "outbound_policy_error": policy_error.as_ref().map(|e| e.to_string()).unwrap_or_default()
                        }));
                        println!(
                            "MAIL_SEND_PROOF|status={}|recipient={}|subject={}|body_len={}|draft_id={}|fresh_recovery={}",
                            send_result.status,
                            send_result.recipient,
                            send_result.subject,
                            send_result.body_len.unwrap_or(-1),
                            send_result.draft_id,
                            fresh_recovery_used
                        );
                        Self::log_evidence(
                            "mail",
                            "send",
                            &[
                                ("status", send_result.status.clone()),
                                ("recipient", send_result.recipient.clone()),
                                ("subject", send_result.subject.clone()),
                                ("body_len", send_result.body_len.unwrap_or(-1).to_string()),
                                ("draft_id", send_result.draft_id.clone()),
                                ("fresh_recovery", fresh_recovery_used.to_string()),
                                (
                                    "outbound_policy",
                                    policy_error
                                        .as_ref()
                                        .map(|e| format!("blocked:{}", e))
                                        .unwrap_or_else(|| "pass".to_string()),
                                ),
                            ],
                        );
                        if policy_error.is_none()
                            && send_result.status == "sent_confirmed"
                            && !send_result.draft_id.trim().is_empty()
                        {
                            if let Ok(removed) = Self::mail_cleanup_marker_outgoing(
                                Some(goal),
                                Some(send_result.draft_id.as_str()),
                            ) {
                                if removed > 0 {
                                    Self::log_evidence(
                                        "mail",
                                        "cleanup",
                                        &[("removed", removed.to_string())],
                                    );
                                }
                            }
                        }
                    }
                    Err(e) => {
                        description = format!("mail_send failed: {}", e);
                        action_status_override = Some("failed");
                    }
                }
            }
            "paste" => {
                if let Some(app_name) = plan
                    .get("app")
                    .and_then(|v| v.as_str())
                    .or_else(|| plan.get("name").and_then(|v| v.as_str()))
                {
                    let _ = heuristics::ensure_app_focus(app_name, 1).await;
                }
                let front_app =
                    crate::tool_chaining::CrossAppBridge::get_frontmost_app().unwrap_or_default();
                if front_app.eq_ignore_ascii_case("Mail") {
                    let draft_id = Self::mail_ensure_draft(Some(goal), history)
                        .ok()
                        .filter(|v| !v.trim().is_empty());
                    let recipient_hint = Self::preferred_mail_recipient(Some(goal));
                    if let Err(e) =
                        Self::mail_set_recipient_if_missing(Some(goal), draft_id.as_deref())
                    {
                        description = format!("Paste failed (mail recipient): {}", e);
                        action_status_override = Some("failed");
                    } else if let Some(recipient) = recipient_hint {
                        let draft_for_log = draft_id.clone().unwrap_or_default();
                        Self::log_evidence(
                            "mail",
                            "write",
                            &[
                                ("status", "confirmed".to_string()),
                                ("recipient", recipient),
                                ("draft_id", draft_for_log),
                            ],
                        );
                    }
                    if action_status_override != Some("failed") {
                        let mut text = crate::tool_chaining::CrossAppBridge::get_clipboard()
                            .unwrap_or_else(|_| "".to_string());
                        let fallback = Self::mail_fallback_body_from_goal(goal);
                        if text.trim().len() < 6 {
                            if !fallback.is_empty() {
                                text = fallback;
                            }
                        } else {
                            let quoted = Self::extract_quoted_fragments(goal)
                                .into_iter()
                                .filter(|s| s.len() >= 3)
                                .collect::<Vec<_>>();
                            let has_any_goal_fragment =
                                quoted.iter().any(|frag| text.contains(frag));
                            if !has_any_goal_fragment && !fallback.is_empty() {
                                text = fallback;
                            }
                        }
                        let quoted = Self::extract_quoted_fragments(goal)
                            .into_iter()
                            .filter(|s| s.len() >= 3)
                            .collect::<Vec<_>>();
                        if !quoted.is_empty() {
                            let mut missing = Vec::new();
                            for frag in &quoted {
                                if !text.contains(frag) {
                                    missing.push(frag.clone());
                                }
                            }
                            if !missing.is_empty() {
                                if text.trim().is_empty() {
                                    text = missing.join("\n");
                                } else {
                                    text = format!("{}\n{}", text.trim_end(), missing.join("\n"));
                                }
                            }
                        }
                        match Self::mail_append_body(&text, draft_id.as_deref()) {
                            Ok((target_draft_id, mut readback_len)) => {
                                Self::remember_mail_draft_id(history, &target_draft_id);
                                if readback_len <= 2 {
                                    let forced_text = Self::extract_quoted_fragments(goal)
                                        .into_iter()
                                        .filter(|s| s.len() >= 3)
                                        .collect::<Vec<_>>()
                                        .join("\n");
                                    if !forced_text.trim().is_empty()
                                        && forced_text.trim() != text.trim()
                                    {
                                        if let Ok((forced_draft_id, forced_len)) =
                                            Self::mail_append_body(
                                                &forced_text,
                                                Some(target_draft_id.as_str()),
                                            )
                                        {
                                            Self::remember_mail_draft_id(history, &forced_draft_id);
                                            readback_len = forced_len;
                                        }
                                    }
                                }
                                if readback_len <= 2 {
                                    description =
                                        "Paste failed (mail body): empty readback after append"
                                            .to_string();
                                    action_status_override = Some("failed");
                                } else {
                                    description =
                                        "Pasted clipboard contents (mail body)".to_string();
                                    action_status_override = Some("success");
                                    action_data = Some(json!({
                                        "proof": "mail_body_appended",
                                        "text_len": text.chars().count(),
                                        "readback_len": readback_len
                                    }));
                                    Self::log_evidence(
                                        "mail",
                                        "write",
                                        &[
                                            ("status", "confirmed".to_string()),
                                            ("body_len", readback_len.to_string()),
                                            ("draft_id", target_draft_id),
                                        ],
                                    );
                                }
                            }
                            Err(e) => {
                                description = format!("Paste failed (mail body): {}", e);
                                action_status_override = Some("failed");
                            }
                        }
                    }
                } else if front_app.eq_ignore_ascii_case("TextEdit") {
                    let mut text = crate::tool_chaining::CrossAppBridge::get_clipboard()
                        .unwrap_or_else(|_| "".to_string());
                    let fallback = Self::mail_fallback_body_from_goal(goal);
                    if text.trim().is_empty() && !fallback.is_empty() {
                        text = fallback;
                    }
                    let quoted = Self::extract_quoted_fragments(goal)
                        .into_iter()
                        .filter(|s| s.len() >= 3)
                        .collect::<Vec<_>>();
                    if !quoted.is_empty() {
                        let mut missing = Vec::new();
                        for frag in &quoted {
                            if !text.contains(frag) {
                                missing.push(frag.clone());
                            }
                        }
                        if !missing.is_empty() {
                            if text.trim().is_empty() {
                                text = missing.join("\n");
                            } else {
                                text = format!("{}\n{}", text.trim_end(), missing.join("\n"));
                            }
                        }
                    }
                    match Self::textedit_append_text(&text, Some(goal)) {
                        Ok(write_result) => {
                            description = format!(
                                "Pasted clipboard contents (textedit body doc_id={} doc_name={})",
                                write_result.doc_id, write_result.doc_name
                            );
                            action_status_override = Some("success");
                            action_data = Some(json!({
                                "proof": "textedit_append_text",
                                "text_len": text.chars().count(),
                                "doc_id": write_result.doc_id,
                                "doc_name": write_result.doc_name,
                                "doc_body_len": write_result.body_len
                            }));
                            Self::log_evidence(
                                "textedit",
                                "write",
                                &[
                                    ("status", "confirmed".to_string()),
                                    ("doc_id", write_result.doc_id.clone()),
                                    ("doc_name", write_result.doc_name.clone()),
                                    ("body_len", write_result.body_len.to_string()),
                                ],
                            );
                        }
                        Err(e) => {
                            description = format!("Paste failed (textedit body): {}", e);
                            action_status_override = Some("failed");
                        }
                    }
                } else {
                    let step = SmartStep::new(
                        UiAction::KeyboardShortcut("v".to_string(), vec!["command".to_string()]),
                        "Paste",
                    );
                    driver.add_step(step);
                    description = "Pasted clipboard contents".to_string();
                }
            }
            "copy" => {
                let front_app =
                    crate::tool_chaining::CrossAppBridge::get_frontmost_app().unwrap_or_default();
                if front_app.eq_ignore_ascii_case("Notes") {
                    match Self::notes_read_text(Some(goal)) {
                        Ok(text) => {
                            if !text.trim().is_empty() {
                                let _ =
                                    crate::tool_chaining::CrossAppBridge::copy_to_clipboard(&text);
                            }
                            description = "Copied selection (notes scripted)".to_string();
                            action_status_override = Some("success");
                            action_data = Some(json!({
                                "proof": "notes_read_text",
                                "text_len": text.chars().count()
                            }));
                        }
                        Err(e) => {
                            description = format!("Copy failed (notes scripted): {}", e);
                            action_status_override = Some("failed");
                        }
                    }
                } else if front_app.eq_ignore_ascii_case("TextEdit") {
                    match Self::textedit_read_text(Some(goal)) {
                        Ok(text) => {
                            if !text.trim().is_empty() {
                                let _ =
                                    crate::tool_chaining::CrossAppBridge::copy_to_clipboard(&text);
                            }
                            description = "Copied selection (textedit scripted)".to_string();
                            action_status_override = Some("success");
                            action_data = Some(json!({
                                "proof": "textedit_read_text",
                                "text_len": text.chars().count()
                            }));
                        }
                        Err(e) => {
                            description = format!("Copy failed (textedit scripted): {}", e);
                            action_status_override = Some("failed");
                        }
                    }
                } else {
                    let step = SmartStep::new(
                        UiAction::KeyboardShortcut("c".to_string(), vec!["command".to_string()]),
                        "Copy",
                    );
                    driver.add_step(step);
                    description = "Copied selection".to_string();
                }
            }
            "select_all" => {
                let step = SmartStep::new(
                    UiAction::KeyboardShortcut("a".to_string(), vec!["command".to_string()]),
                    "Select All",
                );
                driver.add_step(step);
                description = "Selected all contents".to_string();
            }
            "read_clipboard" => match crate::tool_chaining::CrossAppBridge::get_clipboard() {
                Ok(text) => {
                    if let Some(num) = Self::extract_first_number(&text) {
                        *last_read_number = Some(num.clone());
                        history.push(format!("READ_NUMBER: {}", num));
                    }
                    let preview = Self::preview_text(&text, 180);
                    description = format!("Read clipboard -> {}", preview);
                    session.add_message("tool", &format!("read_clipboard: {}", preview));
                    history.push(format!("READ_RESULT: {}", preview));
                }
                Err(e) => {
                    description = format!("read_clipboard failed: {}", e);
                    action_status_override = Some("failed");
                }
            },
            "switch_app" => {
                let app_name = plan["name"]
                    .as_str()
                    .or_else(|| plan["app"].as_str())
                    .unwrap_or("");
                if app_name.is_empty() {
                    description = "switch_app failed: missing app/name".to_string();
                    action_status_override = Some("failed");
                } else {
                    let mut redundant_skip = false;
                    if Self::bool_env_with_default("STEER_BLOCK_REDUNDANT_SWITCH_APP", true) {
                        if let Ok(front_app) =
                            crate::tool_chaining::CrossAppBridge::get_frontmost_app()
                        {
                            if front_app.eq_ignore_ascii_case(app_name) {
                                description =
                                    format!("Switched to app: {} (skipped redundant)", app_name);
                                action_status_override = Some("success");
                                action_data = Some(json!({
                                    "proof": "redundant_switch_app_skip",
                                    "front_app": front_app
                                }));
                                redundant_skip = true;
                            }
                        }
                    }
                    if !redundant_skip {
                        match crate::tool_chaining::CrossAppBridge::switch_to_app(app_name) {
                            Ok(_) => {
                                let _ = heuristics::ensure_app_focus(app_name, 3).await;
                                focus_guard_target = Some(app_name.to_string());
                                description = format!("Switched to app: {}", app_name);
                            }
                            Err(e) => {
                                description = format!("switch_app failed: {}", e);
                                action_status_override = Some("failed");
                            }
                        }
                    }
                }
            }
            "scroll" => {
                let dir = plan["direction"].as_str().unwrap_or("down");
                let step = SmartStep::new(UiAction::Scroll(dir.to_string()), "Scrolling");
                driver.add_step(step);
                description = format!("Scrolled {}", dir);
            }
            "open_url" => {
                let url = plan["url"].as_str().unwrap_or("https://google.com");
                info!("      🌐 Opening URL: '{}'", url);
                if let Err(e) = applescript::open_url(url) {
                    error!("      ❌ Open URL failed: {}", e);
                    description = format!("Failed to open URL: {}", e);
                    action_status_override = Some("failed");
                    *consecutive_failures += 1;
                } else {
                    description = format!("Opened URL '{}'", url);
                    let mut browser_auto = crate::browser_automation::get_browser_automation();
                    browser_auto.reset_snapshot();
                    action_status_override = Some("success");
                }
            }
            "open_app" => {
                let name = plan["name"]
                    .as_str()
                    .or_else(|| plan["app"].as_str())
                    .unwrap_or("Finder");
                info!("      🚀 Launching/Focusing App: '{}'", name);
                let front_app = crate::tool_chaining::CrossAppBridge::get_frontmost_app().ok();
                let name = if name.eq_ignore_ascii_case("Safari") {
                    heuristics::frontmost_browser(front_app.as_deref()).unwrap_or("Safari")
                } else {
                    name
                };

                match crate::reality_check::verify_app_exists(name) {
                    Ok(canonical_name) => {
                        info!(
                            "      🚀 Launching/Focusing App: '{}' (Canonical: '{}')",
                            name, canonical_name
                        );
                        let mut redundant_skip = false;
                        if Self::bool_env_with_default("STEER_BLOCK_REDUNDANT_OPEN_APP", true) {
                            if let Ok(front_app_now) =
                                crate::tool_chaining::CrossAppBridge::get_frontmost_app()
                            {
                                if front_app_now.eq_ignore_ascii_case(&canonical_name)
                                    && Self::history_has_recent_open_for_app(
                                        history,
                                        &canonical_name,
                                    )
                                {
                                    description = format!(
                                        "Opened app: {} (skipped redundant)",
                                        canonical_name
                                    );
                                    action_status_override = Some("success");
                                    action_data = Some(json!({
                                        "proof": "redundant_open_app_skip",
                                        "front_app": front_app_now
                                    }));
                                    redundant_skip = true;
                                }
                            }
                        }
                        if !redundant_skip {
                            match crate::tool_chaining::CrossAppBridge::switch_to_app(
                                &canonical_name,
                            ) {
                                Ok(_) => {
                                    let _ = heuristics::ensure_app_focus(&canonical_name, 3).await;
                                    focus_guard_target = Some(canonical_name.clone());
                                    let step = SmartStep::new(
                                        UiAction::Type(canonical_name.clone()),
                                        "Open App",
                                    );
                                    session_steps.push(step);
                                    description = format!("Opened app: {}", canonical_name);
                                    session.add_message(
                                        "tool",
                                        &format!("open_app: {}", canonical_name),
                                    );
                                }
                                Err(e) => {
                                    error!("      ❌ App open failed: {}", e);
                                    description = format!("Open app failed: {}", e);
                                    action_status_override = Some("failed");
                                }
                            }
                        }
                    }
                    Err(e) => {
                        error!("      ❌ [Reality] REJECTED: {}", e);
                        description = format!("Failed: {}", e);
                        action_status_override = Some("failed");
                    }
                }
            }
            "wait" => {
                let secs = plan["seconds"].as_u64().unwrap_or(2);
                let step = SmartStep::new(UiAction::Wait(secs), "Waiting");
                driver.add_step(step);
                description = format!("Waited {}s", secs);
            }
            "fail" => {
                let reason = plan["reason"].as_str().unwrap_or("Unknown");
                return Err(anyhow::anyhow!("Agent failed: {}", reason));
            }
            _ => {
                description = format!("Action '{}' not implemented in ActionRunner", action_type);
                action_status_override = Some("failed");
            }
        }

        if !driver.steps.is_empty() {
            match driver.execute(llm).await {
                Ok(_) => {}
                Err(e) => {
                    description = format!("{} | driver execution failed: {}", description, e);
                    action_status_override = Some("failed");
                }
            }
            driver.steps.clear();
        }

        if action_status_override.is_none() && focus_guard_required {
            if let Some(target_app) = focus_guard_target.as_deref() {
                let retries = Self::focus_recovery_max_retries();
                let (focus_ok, front_after, attempts, recovery_trace) =
                    Self::recover_focus_and_verify(target_app, retries).await;
                if focus_ok {
                    if attempts > 0 {
                        Self::put_action_data_field(
                            &mut action_data,
                            "focus_recovery",
                            json!({
                                "status": "recovered",
                                "expected_app": target_app,
                                "front_app_after": front_after,
                                "attempts": attempts,
                                "profile": Self::focus_recovery_profile(),
                                "trace": recovery_trace
                            }),
                        );
                    }
                } else {
                    let actual = if front_after.trim().is_empty() {
                        "unknown".to_string()
                    } else {
                        front_after
                    };
                    let detail = format!(
                        "focus_recovery_failed expected={} actual={} retries={}",
                        target_app, actual, attempts
                    );
                    description = format!("{} | {}", description, detail);
                    action_status_override = Some("failed");
                    Self::put_action_data_field(
                        &mut action_data,
                        "focus_recovery",
                        json!({
                            "status": "failed",
                            "expected_app": target_app,
                            "front_app_after": actual,
                            "attempts": attempts,
                            "max_retries": retries,
                            "profile": Self::focus_recovery_profile(),
                            "trace": recovery_trace
                        }),
                    );
                }
            }
        }

        // Log to history and session
        if action_status_override.is_none() {
            let lower_desc = description.to_lowercase();
            if lower_desc.contains(" failed")
                || lower_desc.contains("blocked")
                || lower_desc.contains("error")
            {
                action_status_override = Some("failed");
            }
        }
        if action_status_override.is_none() {
            if let Some(data) = action_data.as_ref() {
                if let Some(send_status) = data.get("send_status").and_then(|v| v.as_str()) {
                    if send_status != "sent_confirmed" {
                        action_status_override = Some("failed");
                    }
                }
            }
        }

        let status = action_status_override.unwrap_or("success");
        if let Some(key) = idempotency_key {
            Self::put_action_data_field(&mut action_data, "idempotency_key", json!(key));
        }
        history.push(description.clone());
        if action_data.is_none() {
            if let Ok(front_after) = crate::tool_chaining::CrossAppBridge::get_frontmost_app() {
                action_data = Some(json!({
                    "front_app_after": front_after
                }));
            }
        }
        session.add_step(action_type, &description, status, action_data);
        let _ = crate::session_store::save_session(session);

        if status != "success" && action_type != "fail" {
            *consecutive_failures += 1;
        } else {
            *consecutive_failures = 0;
        }

        let strict_fail_all_actions = std::env::var("STEER_STRICT_ACTION_ERRORS")
            .ok()
            .map(|v| {
                matches!(
                    v.trim().to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes" | "on"
                )
            })
            .unwrap_or(false);
        let hard_fail_action = matches!(
            action_type,
            "click_visual"
                | "click_ref"
                | "open_app"
                | "switch_app"
                | "type"
                | "paste"
                | "mail_send"
        );

        if status != "success" && (hard_fail_action || strict_fail_all_actions) {
            return Err(anyhow::anyhow!(
                "Critical {} action failed: {}",
                action_type,
                description
            ));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::ActionRunner;
    use serde_json::json;
    use serial_test::serial;

    #[test]
    fn normalize_email_candidate_strips_korean_particle_suffix() {
        let got = ActionRunner::normalize_email_candidate("\"qed4950@gmail.com\"를");
        assert_eq!(got.as_deref(), Some("qed4950@gmail.com"));
    }

    #[test]
    fn extract_mail_recipient_from_goal_uses_plain_email() {
        let goal = "받는 사람에 \"qed4950@gmail.com\"를 입력하고 보내기(Cmd+Shift+D)로 발송하세요.";
        let got = ActionRunner::extract_mail_recipient_from_goal(goal);
        assert_eq!(got.as_deref(), Some("qed4950@gmail.com"));
    }

    #[test]
    fn action_idempotency_key_scopes_cmd_n_to_app_context() {
        let plan_with_app = json!({
            "action": "shortcut",
            "key": "n",
            "modifiers": ["command"],
            "app": "Mail"
        });
        let key =
            ActionRunner::action_idempotency_key("shortcut", &plan_with_app, "Mail 초안 작성", &[]);
        assert_eq!(key.as_deref(), Some("shortcut:mail:command+n:new_item"));

        let history = vec!["Opened app: Notes".to_string()];
        let plan_without_app = json!({
            "action": "shortcut",
            "key": "n",
            "modifiers": ["command"]
        });
        let key_from_history = ActionRunner::action_idempotency_key(
            "shortcut",
            &plan_without_app,
            "메모 작성",
            &history,
        );
        assert_eq!(
            key_from_history.as_deref(),
            Some("shortcut:notes:command+n:new_item")
        );

        let key_from_goal = ActionRunner::action_idempotency_key(
            "shortcut",
            &plan_without_app,
            "메일 새 창 만들어줘",
            &[],
        );
        assert_eq!(
            key_from_goal.as_deref(),
            Some("shortcut:mail:command+n:new_item")
        );
    }

    #[test]
    fn redundant_cmd_n_skip_is_scoped_to_same_app_context() {
        let history = vec![
            "Opened app: Mail".to_string(),
            "Shortcut 'n' + [\"command\"] (Created new item)".to_string(),
            "Typed 'Subject' (mail subject)".to_string(),
        ];
        assert!(ActionRunner::should_skip_redundant_cmd_n(&history, "Mail"));
        assert!(!ActionRunner::should_skip_redundant_cmd_n(
            &history, "Notes"
        ));
    }

    #[test]
    fn history_recent_cmd_n_created_count_tracks_window_flood() {
        let history = vec![
            "Opened app: Mail".to_string(),
            "Shortcut 'n' + [\"command\"] (Created new item)".to_string(),
            "Typed 'Subject A'".to_string(),
            "Shortcut 'n' + [\"command\"] (Created new item)".to_string(),
            "Typed 'Subject B'".to_string(),
            "Shortcut 'n' + [\"command\"] (Created new item)".to_string(),
        ];
        let count = ActionRunner::history_recent_cmd_n_created_count(&history, "Mail", 16);
        assert_eq!(count, 3);
    }

    #[test]
    fn history_recent_cmd_n_created_count_is_app_scoped() {
        let history = vec![
            "Opened app: Mail".to_string(),
            "Shortcut 'n' + [\"command\"] (Created new item)".to_string(),
            "Opened app: Notes".to_string(),
            "Shortcut 'n' + [\"command\"] (Created new item)".to_string(),
            "Shortcut 'n' + [\"command\"] (Created new item)".to_string(),
            "Opened app: Mail".to_string(),
            "Shortcut 'n' + [\"command\"] (Created new item)".to_string(),
        ];

        let mail_count = ActionRunner::history_recent_cmd_n_created_count(&history, "Mail", 32);
        let notes_count = ActionRunner::history_recent_cmd_n_created_count(&history, "Notes", 32);

        assert_eq!(mail_count, 2);
        assert_eq!(notes_count, 2);
    }

    #[test]
    #[serial]
    fn cmd_n_window_flood_limit_for_mail_defaults_to_one() {
        std::env::remove_var("STEER_CMD_N_WINDOW_FLOOD_LIMIT");
        std::env::remove_var("STEER_CMD_N_WINDOW_FLOOD_LIMIT_MAIL");
        let got = ActionRunner::cmd_n_window_flood_limit_for_app("Mail");
        assert_eq!(got, 1);
    }

    #[test]
    #[serial]
    fn cmd_n_window_flood_limit_for_app_specific_env_overrides_default() {
        std::env::set_var("STEER_CMD_N_WINDOW_FLOOD_LIMIT", "4");
        std::env::set_var("STEER_CMD_N_WINDOW_FLOOD_LIMIT_MAIL", "2");
        let mail = ActionRunner::cmd_n_window_flood_limit_for_app("Mail");
        let notes = ActionRunner::cmd_n_window_flood_limit_for_app("Notes");
        std::env::remove_var("STEER_CMD_N_WINDOW_FLOOD_LIMIT_MAIL");
        std::env::remove_var("STEER_CMD_N_WINDOW_FLOOD_LIMIT");
        assert_eq!(mail, 2);
        assert_eq!(notes, 4);
    }

    #[test]
    #[serial]
    fn mail_max_outgoing_for_auto_draft_defaults_and_clamps() {
        std::env::remove_var("STEER_MAIL_MAX_OUTGOING_FOR_AUTO_DRAFT");
        assert_eq!(ActionRunner::mail_max_outgoing_for_auto_draft(), 8);

        std::env::set_var("STEER_MAIL_MAX_OUTGOING_FOR_AUTO_DRAFT", "0");
        assert_eq!(ActionRunner::mail_max_outgoing_for_auto_draft(), 1);

        std::env::set_var("STEER_MAIL_MAX_OUTGOING_FOR_AUTO_DRAFT", "999");
        assert_eq!(ActionRunner::mail_max_outgoing_for_auto_draft(), 64);

        std::env::remove_var("STEER_MAIL_MAX_OUTGOING_FOR_AUTO_DRAFT");
    }

    #[test]
    fn tracked_mail_draft_is_detected_even_without_recent_cmd_n_phrase() {
        let history = vec![
            "Opened app: Mail".to_string(),
            "MAIL_DRAFT_ID:123456".to_string(),
            "Typed '안건 공유' (mail subject)".to_string(),
        ];
        assert!(ActionRunner::has_tracked_mail_draft(&history));
        assert!(!ActionRunner::should_skip_redundant_cmd_n(
            &history, "Notes"
        ));
    }

    #[test]
    fn history_recent_open_for_app_uses_latest_open_entry() {
        let history = vec![
            "Opened app: Notes".to_string(),
            "Typed 'hello' (notes body)".to_string(),
            "Opened app: Mail".to_string(),
        ];
        assert!(ActionRunner::history_has_recent_open_for_app(
            &history, "Mail"
        ));
        assert!(!ActionRunner::history_has_recent_open_for_app(
            &history, "Notes"
        ));
    }

    #[test]
    fn single_fire_cmd_n_idempotency_survives_long_step_history() {
        let mut session = crate::session_store::Session::new("mail 작성", Some("test"));
        session.add_step(
            "shortcut",
            "Shortcut 'n' + [\"command\"] (Created new item)",
            "success",
            Some(json!({
                "proof": "created_new_item",
                "idempotency_key": "shortcut:mail:command+n:new_item"
            })),
        );
        for i in 0..30usize {
            session.add_step(
                "type",
                &format!("noise step {}", i),
                "success",
                Some(json!({"idempotency_key": format!("noise:{}", i)})),
            );
        }
        assert!(ActionRunner::idempotency_success_hit(
            &session,
            "shortcut:mail:command+n:new_item",
            None
        ));
        assert!(!ActionRunner::idempotency_recent_hit(
            &session,
            "shortcut:mail:command+n:new_item",
            8
        ));
    }

    #[test]
    fn session_cmd_n_stats_counts_attempts_and_successes_per_app() {
        let mut session = crate::session_store::Session::new("mail 작성", Some("test"));
        session.add_step(
            "shortcut",
            "Shortcut 'n' + [\"command\"] failed",
            "failed",
            Some(json!({"idempotency_key": "shortcut:mail:command+n:new_item"})),
        );
        session.add_step(
            "shortcut",
            "Shortcut 'n' + [\"command\"] (Created new item)",
            "success",
            Some(json!({"idempotency_key": "shortcut:mail:command+n:new_item"})),
        );
        session.add_step(
            "shortcut",
            "Shortcut 'n' + [\"command\"] (Created new item)",
            "success",
            Some(json!({"idempotency_key": "shortcut:notes:command+n:new_item"})),
        );

        let (mail_attempts, mail_successes) = ActionRunner::session_cmd_n_stats(&session, "Mail");
        let (notes_attempts, notes_successes) =
            ActionRunner::session_cmd_n_stats(&session, "Notes");
        assert_eq!((mail_attempts, mail_successes), (2, 1));
        assert_eq!((notes_attempts, notes_successes), (1, 1));
    }
}
