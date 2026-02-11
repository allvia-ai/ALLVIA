use anyhow::Result;
use log::{error, info};
use serde_json::json;

use crate::controller::heuristics;
use crate::llm_gateway::LLMClient;
use crate::visual_driver::{SmartStep, UiAction, VisualDriver};

use crate::applescript;

pub struct ActionRunner;

impl ActionRunner {
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

    fn mail_ensure_draft() -> Result<()> {
        let lines = [
            "tell application \"Mail\"",
            "activate",
            "if (count of outgoing messages) = 0 then",
            "set _msg to make new outgoing message with properties {visible:true}",
            "else",
            "set _msg to last outgoing message",
            "set visible of _msg to true",
            "end if",
            "end tell",
            "return \"ok\"",
        ];
        crate::applescript::run_with_args(&lines, &Vec::<String>::new())?;
        let _ = Self::mail_set_recipient_if_missing();
        Ok(())
    }

    fn default_mail_recipient() -> Option<String> {
        let candidates = ["STEER_DEFAULT_MAIL_TO", "STEER_USER_EMAIL", "APPLE_ID_EMAIL"];
        for key in candidates {
            if let Ok(value) = std::env::var(key) {
                let trimmed = value.trim();
                if trimmed.contains('@') && !trimmed.contains(' ') {
                    return Some(trimmed.to_string());
                }
            }
        }
        None
    }

    fn mail_set_recipient_if_missing() -> Result<()> {
        let Some(address) = Self::default_mail_recipient() else {
            return Ok(());
        };
        let lines = [
            "on run argv",
            "set toAddress to item 1 of argv",
            "tell application \"Mail\"",
            "activate",
            "if (count of outgoing messages) = 0 then",
            "set _msg to make new outgoing message with properties {visible:true}",
            "else",
            "set _msg to last outgoing message",
            "set visible of _msg to true",
            "end if",
            "if (count of to recipients of _msg) = 0 then",
            "make new to recipient at end of to recipients of _msg with properties {address:toAddress}",
            "end if",
            "end tell",
            "return \"ok\"",
            "end run",
        ];
        crate::applescript::run_with_args(&lines, &[address])?;
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

    fn mail_send_latest_message() -> Result<String> {
        let fallback = Self::default_mail_recipient().unwrap_or_default();
        let lines = [
            "on run argv",
            "set fallbackAddress to item 1 of argv",
            "tell application \"Mail\"",
            "activate",
            "if (count of outgoing messages) = 0 then return \"no_draft\"",
            "set _msg to last outgoing message",
            "set visible of _msg to true",
            "if (count of to recipients of _msg) = 0 then",
            "if fallbackAddress is not \"\" then",
            "make new to recipient at end of to recipients of _msg with properties {address:fallbackAddress}",
            "else",
            "return \"missing_recipient\"",
            "end if",
            "end if",
            "send _msg",
            "end tell",
            "return \"sent\"",
            "end run",
        ];
        let out = crate::applescript::run_with_args(&lines, &[fallback])?;
        Ok(out.trim().to_string())
    }

    fn mail_set_subject(subject: &str) -> Result<()> {
        let lines = [
            "on run argv",
            "set subjectText to item 1 of argv",
            "tell application \"Mail\"",
            "activate",
            "if (count of outgoing messages) = 0 then",
            "set _msg to make new outgoing message with properties {visible:true}",
            "else",
            "set _msg to last outgoing message",
            "set visible of _msg to true",
            "end if",
            "set subject of _msg to subjectText",
            "end tell",
            "return \"ok\"",
            "end run",
        ];
        crate::applescript::run_with_args(&lines, &[subject.to_string()])?;
        Ok(())
    }

    fn mail_append_body(text: &str) -> Result<()> {
        let lines = [
            "on run argv",
            "set bodyText to item 1 of argv",
            "tell application \"Mail\"",
            "activate",
            "if (count of outgoing messages) = 0 then",
            "set _msg to make new outgoing message with properties {visible:true}",
            "else",
            "set _msg to last outgoing message",
            "set visible of _msg to true",
            "end if",
            "set existingContent to content of _msg",
            "if existingContent is missing value then set existingContent to \"\"",
            "if existingContent is \"\" then",
            "set content of _msg to bodyText",
            "else",
            "set content of _msg to existingContent & return & bodyText",
            "end if",
            "end tell",
            "return \"ok\"",
            "end run",
        ];
        crate::applescript::run_with_args(&lines, &[text.to_string()])?;
        Ok(())
    }

    fn notes_write_text(text: &str) -> Result<()> {
        let lines = [
            "on run argv",
            "set bodyText to item 1 of argv",
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
            "set n to make new note at fd with properties {name:noteTitle, body:bodyText}",
            "end tell",
            "return \"ok\"",
            "end run",
        ];
        crate::applescript::run_with_args(&lines, &[text.to_string()])?;
        Ok(())
    }

    fn notes_read_text() -> Result<String> {
        let lines = [
            "tell application \"Notes\"",
            "if (count of accounts) = 0 then return \"\"",
            "set ac to item 1 of accounts",
            "if (count of folders of ac) = 0 then return \"\"",
            "set fd to item 1 of folders of ac",
            "if (count of notes of fd) = 0 then return \"\"",
            "set n to last note of fd",
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
        ];
        crate::applescript::run_with_args(&lines, &Vec::<String>::new())
    }

    fn textedit_append_text(text: &str) -> Result<()> {
        let lines = [
            "on run argv",
            "set bodyText to item 1 of argv",
            "tell application \"TextEdit\"",
            "activate",
            "if (count of documents) = 0 then make new document",
            "set targetDoc to front document",
            "set existingText to \"\"",
            "try",
            "set existingText to text of targetDoc as text",
            "end try",
            "if existingText is \"\" then",
            "set text of targetDoc to bodyText",
            "else",
            "set text of targetDoc to existingText & return & bodyText",
            "end if",
            "end tell",
            "return \"ok\"",
            "end run",
        ];
        crate::applescript::run_with_args(&lines, &[text.to_string()])?;
        Ok(())
    }

    fn textedit_read_text() -> Result<String> {
        let lines = [
            "tell application \"TextEdit\"",
            "if (count of documents) = 0 then return \"\"",
            "try",
            "return text of front document as text",
            "on error",
            "return \"\"",
            "end try",
            "end tell",
        ];
        crate::applescript::run_with_args(&lines, &Vec::<String>::new())
    }

    fn goal_mentions_downloads(goal: &str) -> bool {
        let lower = goal.to_lowercase();
        lower.contains("downloads") || lower.contains("downloads folder") || lower.contains("다운로드")
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

    fn normalize_shortcut_parts(
        key_raw: &str,
        modifiers_raw: &[String],
    ) -> (String, Vec<String>) {
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
                        && (Self::is_focus_noise_app(front_app)
                            || strict_focus_actions)
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
                    let front_app =
                        crate::tool_chaining::CrossAppBridge::get_frontmost_app().unwrap_or_default();
                    if front_app.eq_ignore_ascii_case("Finder")
                        && Self::goal_mentions_downloads(goal)
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

                if front_app.eq_ignore_ascii_case("Finder")
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
                    let _ = crate::tool_chaining::CrossAppBridge::switch_to_app(app_name);
                    let _ = heuristics::ensure_app_focus(app_name, 3).await;
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
                    let _ = crate::tool_chaining::CrossAppBridge::switch_to_app(app_name);
                    let _ = heuristics::ensure_app_focus(app_name, 3).await;
                    forced_app = true;
                } else if let Some(target_app) =
                    Self::preferred_target_app_from_history("type", plan, history)
                {
                    let _ = heuristics::ensure_app_focus(&target_app, 5).await;
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
                        let _ = Self::mail_ensure_draft();
                        let subject_already_set = history
                            .iter()
                            .any(|h| h.to_lowercase().contains("(mail subject)"));
                        let prefer_subject = heuristics::looks_like_subject(&text)
                            || (!subject_already_set && !text.contains('\n') && text.len() <= 120);
                        if prefer_subject {
                            match Self::mail_set_subject(&text) {
                                Ok(_) => {
                                    description = format!("Typed '{}' (mail subject)", text);
                                    action_data = Some(json!({
                                        "proof": "mail_subject_set",
                                        "text_len": text.chars().count()
                                    }));
                                }
                                Err(e) => {
                                    description = format!("Type failed (mail subject): {}", e);
                                    action_status_override = Some("failed");
                                }
                            }
                        } else {
                            match Self::mail_append_body(&text) {
                                Ok(_) => {
                                    description = format!("Typed '{}' (mail body)", text);
                                    action_data = Some(json!({
                                        "proof": "mail_body_appended",
                                        "text_len": text.chars().count()
                                    }));
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
                        match Self::notes_write_text(&write_text) {
                            Ok(_) => {
                                description = format!("Typed '{}' (notes body)", write_text);
                                action_status_override = Some("success");
                                action_data = Some(json!({
                                    "proof": "notes_write_text",
                                    "text_len": write_text.chars().count()
                                }));
                            }
                            Err(e) => {
                                description = format!("Type failed (notes body): {}", e);
                                action_status_override = Some("failed");
                            }
                        }
                    } else if app_name.eq_ignore_ascii_case("TextEdit") {
                        match Self::textedit_append_text(&text) {
                            Ok(_) => {
                                description = format!("Typed '{}' (textedit body)", text);
                                action_status_override = Some("success");
                                action_data = Some(json!({
                                    "proof": "textedit_append_text",
                                    "text_len": text.chars().count()
                                }));
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
                    let front_app =
                        crate::tool_chaining::CrossAppBridge::get_frontmost_app().unwrap_or_default();

                    if is_cmd_n && front_app.eq_ignore_ascii_case("Mail") {
                        match Self::mail_ensure_draft() {
                            Ok(_) => {
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
                        match Self::mail_send_latest_message() {
                            Ok(result) if result == "sent" => {
                                let outgoing_after = Self::mail_outgoing_count().unwrap_or(-1);
                                description = format!(
                                    "Shortcut '{}' + {:?} (Mail sent)",
                                    key, shortcut_modifiers
                                );
                                action_status_override = Some("success");
                                action_data = Some(json!({
                                    "proof": "mail_send",
                                    "result": result,
                                    "outgoing_after": outgoing_after
                                }));
                            }
                            Ok(result) => {
                                description = format!(
                                    "Shortcut '{}' + {:?} (mail send blocked: {})",
                                    key, shortcut_modifiers, result
                                );
                                action_status_override = Some("failed");
                                action_data = Some(json!({
                                    "proof": "mail_send",
                                    "result": result
                                }));
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

                    let is_cmd_n = key == "n"
                        && modifiers
                            .iter()
                            .any(|m| m.eq_ignore_ascii_case("command"));
                    let is_cmd_shift_d = key == "d"
                        && modifiers
                            .iter()
                            .any(|m| m.eq_ignore_ascii_case("command"))
                        && modifiers
                            .iter()
                            .any(|m| m.eq_ignore_ascii_case("shift"));
                    let front_app = crate::tool_chaining::CrossAppBridge::get_frontmost_app()
                        .unwrap_or_default();
                    if is_cmd_n && front_app.eq_ignore_ascii_case("Mail") {
                        match Self::mail_ensure_draft() {
                            Ok(_) => {
                                description =
                                    format!("Shortcut '{}' + {:?} (Created new item)", key, modifiers);
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
                        match Self::mail_send_latest_message() {
                            Ok(result) if result == "sent" => {
                                let outgoing_after = Self::mail_outgoing_count().unwrap_or(-1);
                                description = format!("Shortcut '{}' + {:?} (Mail sent)", key, modifiers);
                                action_status_override = Some("success");
                                action_data = Some(json!({
                                    "proof": "mail_send",
                                    "result": result,
                                    "outgoing_after": outgoing_after
                                }));
                            }
                            Ok(result) => {
                                description = format!(
                                    "Shortcut '{}' + {:?} (mail send blocked: {})",
                                    key, modifiers, result
                                );
                                action_status_override = Some("failed");
                                action_data = Some(json!({
                                    "proof": "mail_send",
                                    "result": result
                                }));
                            }
                            Err(e) => {
                                description =
                                    format!("Shortcut '{}' + {:?} (mail send failed: {})", key, modifiers, e);
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
                        } else {
                            description = format!("Shortcut '{}' + {:?}", key, modifiers);
                        }
                    }
                }
            }
            "mail_send" => {
                let front_app = crate::tool_chaining::CrossAppBridge::get_frontmost_app()
                    .unwrap_or_default();
                if !front_app.eq_ignore_ascii_case("Mail") {
                    let _ = heuristics::ensure_app_focus("Mail", 3).await;
                }
                match Self::mail_send_latest_message() {
                    Ok(result) if result == "sent" => {
                        let outgoing_after = Self::mail_outgoing_count().unwrap_or(-1);
                        description = "Mail send completed".to_string();
                        action_status_override = Some("success");
                        action_data = Some(json!({
                            "proof": "mail_send",
                            "result": result,
                            "outgoing_after": outgoing_after
                        }));
                    }
                    Ok(result) => {
                        description = format!("Mail send blocked: {}", result);
                        action_status_override = Some("failed");
                        action_data = Some(json!({
                            "proof": "mail_send",
                            "result": result
                        }));
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
                    let _ = crate::tool_chaining::CrossAppBridge::switch_to_app(app_name);
                    let _ = heuristics::ensure_app_focus(app_name, 3).await;
                }
                let front_app =
                    crate::tool_chaining::CrossAppBridge::get_frontmost_app().unwrap_or_default();
                if front_app.eq_ignore_ascii_case("Mail") {
                    let _ = Self::mail_set_recipient_if_missing();
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
                        let has_any_goal_fragment = quoted.iter().any(|frag| text.contains(frag));
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
                    match Self::mail_append_body(&text) {
                        Ok(_) => {
                            description = "Pasted clipboard contents (mail body)".to_string();
                            action_status_override = Some("success");
                            action_data = Some(json!({
                                "proof": "mail_body_appended",
                                "text_len": text.chars().count()
                            }));
                        }
                        Err(e) => {
                            description = format!("Paste failed (mail body): {}", e);
                            action_status_override = Some("failed");
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
                    match Self::textedit_append_text(&text) {
                        Ok(_) => {
                            description = "Pasted clipboard contents (textedit body)".to_string();
                            action_status_override = Some("success");
                            action_data = Some(json!({
                                "proof": "textedit_append_text",
                                "text_len": text.chars().count()
                            }));
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
                    match Self::notes_read_text() {
                        Ok(text) => {
                            if !text.trim().is_empty() {
                                let _ = crate::tool_chaining::CrossAppBridge::copy_to_clipboard(&text);
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
                    match Self::textedit_read_text() {
                        Ok(text) => {
                            if !text.trim().is_empty() {
                                let _ = crate::tool_chaining::CrossAppBridge::copy_to_clipboard(&text);
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
                    match crate::tool_chaining::CrossAppBridge::switch_to_app(app_name) {
                        Ok(_) => {
                            let _ = heuristics::ensure_app_focus(app_name, 3).await;
                            description = format!("Switched to app: {}", app_name);
                        }
                        Err(e) => {
                            description = format!("switch_app failed: {}", e);
                            action_status_override = Some("failed");
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
                        match crate::tool_chaining::CrossAppBridge::switch_to_app(&canonical_name) {
                            Ok(_) => {
                                let _ = heuristics::ensure_app_focus(&canonical_name, 3).await;
                                let step = SmartStep::new(
                                    UiAction::Type(canonical_name.clone()),
                                    "Open App",
                                );
                                session_steps.push(step);
                                description = format!("Opened app: {}", canonical_name);
                                session
                                    .add_message("tool", &format!("open_app: {}", canonical_name));
                            }
                            Err(e) => {
                                error!("      ❌ App open failed: {}", e);
                                description = format!("Open app failed: {}", e);
                                action_status_override = Some("failed");
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

        // Log to history and session
        let status = action_status_override.unwrap_or("success");
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

        if status != "success" && matches!(action_type, "click_visual" | "click_ref") {
            return Err(anyhow::anyhow!(
                "Critical {} action failed: {}",
                action_type,
                description
            ));
        }

        Ok(())
    }
}
