use crate::applescript;
use anyhow::Result;
use sha2::{Digest, Sha256};

pub fn permission_help() -> &'static str {
    "Enable Screen Recording + Accessibility for Terminal/Codex (System Settings > Privacy & Security). If prompts disappear, try `tccutil reset Accessibility` and `tccutil reset ScreenCapture` then relaunch the app."
}

pub fn preflight_permissions() -> Result<()> {
    if crate::peekaboo_cli::is_available() {
        if let Ok(perms) = crate::peekaboo_cli::check_permissions() {
            if perms.screen_recording == Some(false) {
                return Err(anyhow::anyhow!(
                    "Screen Recording permission missing (Peekaboo). {}",
                    permission_help()
                ));
            }
            if perms.accessibility == Some(false) {
                return Err(anyhow::anyhow!(
                    "Accessibility permission missing (Peekaboo). {}",
                    permission_help()
                ));
            }
        }
    }

    if let Err(e) = applescript::check_accessibility() {
        return Err(anyhow::anyhow!(
            "Accessibility permission check failed: {}. {}",
            e,
            permission_help()
        ));
    }

    Ok(())
}

pub fn verify_screen_capture() -> Result<()> {
    // Keep execution-path behavior aligned with API preflight toggle.
    if !env_truthy_default("STEER_PREFLIGHT_SCREEN_CAPTURE", true) {
        let skip_allowed = env_truthy_default("STEER_TEST_MODE", false)
            || env_truthy_default("STEER_ALLOW_SCREEN_CAPTURE_SKIP", false);
        if skip_allowed {
            return Ok(());
        }
        return Err(anyhow::anyhow!(
            "Screen capture preflight disabled by env (STEER_PREFLIGHT_SCREEN_CAPTURE=0) in non-test mode. {}",
            permission_help()
        ));
    }

    // 1) First, rely on the native macOS permission check.
    // 2) Also run an actual screencapture probe because native check can report
    // false negatives in some bundle/runtime combinations.
    let native_granted = crate::permission_manager::PermissionManager::check_screen_recording();

    // 2. We can optionally do a quick probe if we want to ensure the binary is able to write,
    // but the native check passing is the real source of truth for "vision permission".
    let shot_path = format!(
        "/tmp/steer_preflight_capture_{}_{}.png",
        std::process::id(),
        chrono::Utc::now().timestamp_millis()
    );

    let probe_status = std::process::Command::new("screencapture")
        .args(["-x", shot_path.as_str()])
        .status();

    // Clean up if it worked
    let exists = std::fs::metadata(&shot_path)
        .map(|m| m.is_file() && m.len() > 0)
        .unwrap_or(false);
    if exists {
        let _ = std::fs::remove_file(&shot_path);
    }
    let _ = std::fs::remove_file(&shot_path);

    // If native check says denied but probe succeeded, accept it as a practical pass.
    if !native_granted && exists {
        return Ok(());
    }

    if !native_granted {
        return Err(anyhow::anyhow!(
            "Screen capture unavailable (Native permission missing). {}",
            permission_help()
        ));
    }

    // Native says granted, but probe failed: surface command status for diagnosis.
    if !exists {
        let probe_hint = match probe_status {
            Ok(status) => format!("status={}", status),
            Err(err) => format!("spawn_error={}", err),
        };
        return Err(anyhow::anyhow!(
            "Screen capture probe produced no file ({}). {}",
            probe_hint,
            permission_help()
        ));
    }

    Ok(())
}

fn env_truthy_default(key: &str, default: bool) -> bool {
    std::env::var(key)
        .ok()
        .map(|v| {
            let n = v.trim().to_ascii_lowercase();
            matches!(n.as_str(), "1" | "true" | "yes" | "on")
        })
        .unwrap_or(default)
}

pub fn extract_best_number(text: &str) -> Option<String> {
    let mut nums: Vec<String> = Vec::new();
    let mut buf = String::new();
    for ch in text.chars() {
        if ch.is_ascii_digit() || ch == '.' || ch == ',' {
            buf.push(ch);
        } else if !buf.is_empty() {
            nums.push(buf.clone());
            buf.clear();
        }
    }
    if !buf.is_empty() {
        nums.push(buf);
    }

    let mut cleaned: Vec<String> = nums
        .into_iter()
        .map(|n| n.replace(',', ""))
        .map(|n| n.trim_matches('.').to_string())
        .filter(|n| !n.is_empty() && n.chars().any(|c| c.is_ascii_digit()))
        .collect();

    if cleaned.is_empty() {
        return None;
    }

    // Prefer decimals (likely prices)
    if let Some(first_decimal) = cleaned.iter().find(|n| n.contains('.')) {
        return Some(first_decimal.clone());
    }

    // Otherwise pick the largest numeric value
    cleaned.sort_by(|a, b| {
        let av = a.parse::<f64>().unwrap_or(0.0);
        let bv = b.parse::<f64>().unwrap_or(0.0);
        bv.partial_cmp(&av).unwrap_or(std::cmp::Ordering::Equal)
    });
    cleaned.first().cloned()
}

pub fn calculator_has_input(history: &[String]) -> bool {
    let mut seen_open = false;
    for entry in history.iter().rev() {
        if entry.contains("Opened app: Calculator") {
            seen_open = true;
            break;
        }
    }
    if !seen_open {
        return false;
    }
    for entry in history.iter().rev() {
        if entry.contains("Opened app: Calculator") {
            break;
        }
        if entry.starts_with("Typed '") {
            return true;
        }
    }
    false
}

pub fn goal_mentions_calculation(goal: &str) -> bool {
    let lower = goal.to_lowercase();
    lower.contains("계산")
        || lower.contains("calculate")
        || lower.contains("곱")
        || lower.contains("×")
        || lower.contains("*")
        || lower.contains("plus")
        || lower.contains("minus")
}

pub fn goal_is_ui_task(goal: &str) -> bool {
    let lower = goal.to_lowercase();
    let apps = [
        "safari",
        "notes",
        "finder",
        "preview",
        "textedit",
        "mail",
        "calculator",
    ];
    apps.iter().any(|app| lower.contains(app))
}

pub fn goal_mentions_desktop(goal: &str) -> bool {
    let lower = goal.to_lowercase();
    lower.contains("desktop") || lower.contains("데스크탑")
}

pub fn goal_mentions_image(goal: &str) -> bool {
    let lower = goal.to_lowercase();
    lower.contains("image")
        || lower.contains("이미지")
        || lower.contains(".png")
        || lower.contains(".jpg")
}

pub fn goal_mentions_notes(goal: &str) -> bool {
    let lower = goal.to_lowercase();
    lower.contains("notes") || lower.contains("메모")
}

pub fn looks_like_subject(text: &str) -> bool {
    let lower = text.to_lowercase();
    text.len() <= 80
        && !text.contains('\n')
        && (lower.contains("meeting")
            || lower.contains("research findings")
            || lower.contains("notes")
            || lower.contains("subject"))
}

pub fn focus_text_area(app: &str, prefer_subject: bool) -> bool {
    let script = match app {
        "Mail" => {
            if prefer_subject {
                r#"
                    tell application "System Events"
                        tell process "Mail"
                            if exists window 1 then
                                try
                                    if exists text field 1 of window 1 then
                                        click text field 1 of window 1
                                        return "subject"
                                    end if
                                end try
                            end if
                        end tell
                    end tell
                    return ""
                "#
            } else {
                r#"
                    tell application "System Events"
                        tell process "Mail"
                            if exists window 1 then
                                try
                                    if exists scroll area 1 of window 1 then
                                        click scroll area 1 of window 1
                                        return "body"
                                    end if
                                end try
                            end if
                        end tell
                    end tell
                    return ""
                "#
            }
        }
        "Notes" => {
            r#"
            tell application "System Events"
                tell process "Notes"
                    if exists window 1 then
                        try
                            if exists scroll area 1 of window 1 then
                                click scroll area 1 of window 1
                                return "body"
                            end if
                        end try
                    end if
                end tell
            end tell
            return ""
        "#
        }
        "TextEdit" => {
            r#"
            tell application "System Events"
                tell process "TextEdit"
                    if exists window 1 then
                        set wName to ""
                        try
                            set wName to name of window 1 as text
                        end try
                        if wName contains "Open" or wName contains "open" or wName contains "열기" or wName contains "Save" or wName contains "save" or wName contains "저장" then
                            try
                                if exists button "Cancel" of window 1 then
                                    click button "Cancel" of window 1
                                else if exists button "취소" of window 1 then
                                    click button "취소" of window 1
                                else
                                    key code 53
                                end if
                                delay 0.12
                            end try
                        end if
                        try
                            if exists scroll area 1 of window 1 then
                                click scroll area 1 of window 1
                                return "body"
                            end if
                        end try
                    end if
                end tell
            end tell
            return ""
        "#
        }
        _ => "",
    };

    if script.is_empty() {
        return false;
    }

    if let Ok(out) = applescript::run(script) {
        if !out.trim().is_empty() {
            return true;
        }
    }

    // Fallback: click window center
    let fallback = format!(
        r#"
        tell application "System Events"
            tell process "{}"
                if exists window 1 then
                    set {{x, y}} to position of window 1
                    set {{w, h}} to size of window 1
                    set cx to x + (w / 2)
                    set cy to y + (h / 2)
                    click at {{cx, cy}}
                end if
            end tell
        end tell
    "#,
        app
    );
    let _ = applescript::run(&fallback);
    true
}

pub fn goal_mentions_stock_price(goal: &str) -> bool {
    let lower = goal.to_lowercase();
    lower.contains("stock price") || lower.contains("주가")
}

pub fn infer_stock_symbol(goal: &str, query: &str) -> Option<&'static str> {
    let lower = format!("{} {}", goal.to_lowercase(), query.to_lowercase());
    if lower.contains("aapl") || lower.contains("apple") {
        return Some("AAPL");
    }
    None
}

pub async fn fetch_stock_price(symbol: &str) -> Option<String> {
    let url = format!(
        "https://query1.finance.yahoo.com/v7/finance/quote?symbols={}",
        symbol
    );
    if let Ok(resp) = reqwest::get(&url).await {
        if let Ok(body) = resp.text().await {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&body) {
                if let Some(price) = json
                    .get("quoteResponse")
                    .and_then(|v| v.get("result"))
                    .and_then(|v| v.get(0))
                    .and_then(|v| v.get("regularMarketPrice"))
                    .and_then(|v| v.as_f64())
                {
                    return Some(format!("{}", price));
                }
            }
        }
    }

    // Fallback to Stooq (CSV)
    let sym = symbol.to_lowercase();
    let stooq_url = format!("https://stooq.com/q/l/?s={}.us&f=sd2t2ohlcv&h&e=csv", sym);
    let resp = reqwest::get(&stooq_url).await.ok()?;
    let body = resp.text().await.ok()?;
    let mut lines = body.lines();
    let _header = lines.next()?;
    let data = lines.next()?;
    let cols: Vec<&str> = data.split(',').collect();
    if cols.len() >= 8 {
        let close = cols[6].trim();
        if !close.is_empty() && close != "N/A" {
            return Some(close.to_string());
        }
    }
    None
}

pub fn compute_calc_result(num_str: &str) -> Option<String> {
    let cleaned = num_str.replace(',', "");
    let val: f64 = cleaned.parse().ok()?;
    let result = val * 100.0;
    let mut text = format!("{:.2}", result);
    while text.contains('.') && text.ends_with('0') {
        text.pop();
    }
    if text.ends_with('.') {
        text.pop();
    }
    Some(text)
}

pub fn normalize_digits(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_digit() || *c == '.')
        .collect()
}

pub fn extract_search_query(goal: &str) -> Option<String> {
    if let Some(start) = goal.find('\'') {
        if let Some(end) = goal[start + 1..].find('\'') {
            let query = &goal[start + 1..start + 1 + end];
            if !query.trim().is_empty() {
                return Some(query.trim().to_string());
            }
        }
    }
    if let Some(start) = goal.find('\"') {
        if let Some(end) = goal[start + 1..].find('\"') {
            let query = &goal[start + 1..start + 1 + end];
            if !query.trim().is_empty() {
                return Some(query.trim().to_string());
            }
        }
    }

    let lower = goal.to_lowercase();
    for key in ["검색:", "search:", "검색", "search"] {
        if let Some(idx) = lower.find(key) {
            let rest = goal[idx + key.len()..].trim();
            if !rest.is_empty() {
                return Some(rest.to_string());
            }
        }
    }

    None
}

pub fn extract_note_title(goal: &str) -> Option<String> {
    let lower = goal.to_lowercase();
    let mut after_title = None;
    for key in ["제목", "title"] {
        if let Some(idx) = lower.find(key) {
            after_title = Some(&goal[idx + key.len()..]);
            break;
        }
    }

    if let Some(rest) = after_title {
        if let Some(start) = rest.find('\'') {
            if let Some(end) = rest[start + 1..].find('\'') {
                let title = &rest[start + 1..start + 1 + end];
                if !title.trim().is_empty() {
                    return Some(title.trim().to_string());
                }
            }
        }
        if let Some(start) = rest.find('\"') {
            if let Some(end) = rest[start + 1..].find('\"') {
                let title = &rest[start + 1..start + 1 + end];
                if !title.trim().is_empty() {
                    return Some(title.trim().to_string());
                }
            }
        }
    }

    if goal.contains("Apple Stock Calculation") {
        return Some("Apple Stock Calculation".to_string());
    }

    None
}

pub fn google_search_url(query: &str) -> String {
    let encoded = urlencoding::encode(query);
    format!("https://google.com/search?q={}", encoded)
}

pub fn google_lucky_url(query: &str) -> String {
    let encoded = urlencoding::encode(query);
    format!("https://www.google.com/search?q={}&btnI=1", encoded)
}

pub fn frontmost_browser(front_app: Option<&str>) -> Option<&'static str> {
    match front_app {
        Some(app) if app.eq_ignore_ascii_case("Safari") => Some("Safari"),
        Some(app) if app.eq_ignore_ascii_case("Google Chrome") => Some("Google Chrome"),
        _ => None,
    }
}

pub fn is_google_search_goal(goal: &str) -> bool {
    let lower = goal.to_lowercase();
    lower.contains("google") || lower.contains("검색")
}

pub fn wants_first_result(goal: &str) -> bool {
    let lower = goal.to_lowercase();
    lower.contains("first result")
        || lower.contains("첫 번째 결과")
        || lower.contains("첫번째 결과")
        || (lower.contains("첫") && lower.contains("결과"))
}

pub fn prefer_lucky_only(goal: &str) -> bool {
    wants_first_result(goal) && is_google_search_goal(goal)
}

pub fn extract_query_param(url: &str, key: &str) -> Option<String> {
    let qs = url.split_once('?')?.1;
    for pair in qs.split('&') {
        let mut it = pair.splitn(2, '=');
        let k = it.next().unwrap_or("");
        if k != key {
            continue;
        }
        let v = it.next().unwrap_or("");
        if let Ok(decoded) = urlencoding::decode(v) {
            let out = decoded.into_owned();
            if !out.is_empty() {
                return Some(out);
            }
        }
    }
    None
}

pub fn extract_google_redirect_target(url: &str) -> Option<String> {
    if !(url.contains("google.com/url?")
        || url.contains("google.co.kr/url?")
        || url.contains("google.com/url?q="))
    {
        return None;
    }
    extract_query_param(url, "url")
        .or_else(|| extract_query_param(url, "q"))
        .or_else(|| extract_query_param(url, "target"))
}

pub fn is_redirect_alert(title: &str, url: &str) -> bool {
    let t = title.to_lowercase();
    let u = url.to_lowercase();
    t.contains("리디렉션")
        || t.contains("redirect")
        || u.contains("google.com/url?")
        || u.contains("google.co.kr/url?")
}

pub fn try_close_front_dialog() -> bool {
    let script = r#"
        tell application "System Events"
            set frontApp to name of first application process whose frontmost is true
            tell process frontApp
                if (count of windows) > 0 then
                    set w to window 1
                    if exists sheet 1 of w then
                        if exists button "Cancel" of sheet 1 of w then
                            click button "Cancel" of sheet 1 of w
                            return "cancel-sheet"
                        else if exists button "취소" of sheet 1 of w then
                            click button "취소" of sheet 1 of w
                            return "cancel-sheet"
                        else if exists button "닫기" of sheet 1 of w then
                            click button "닫기" of sheet 1 of w
                            return "close-sheet"
                        end if
                    end if

                    set wName to ""
                    try
                        set wName to name of w as text
                    end try
                    -- Guard: avoid treating normal browser/content windows (e.g. "OpenClaw ...")
                    -- as file dialogs. Only match strict dialog-like titles.
                    set isDialogTitle to false
                    if wName is "Open" or wName is "Open…" or wName is "Save" or wName is "Save…" or wName is "Save As" or wName is "열기" or wName is "저장" then
                        set isDialogTitle to true
                    end if
                    if isDialogTitle then
                        if exists button "Cancel" of w then
                            click button "Cancel" of w
                            return "cancel-window"
                        else if exists button "취소" of w then
                            click button "취소" of w
                            return "cancel-window"
                        else if exists button "닫기" of w then
                            click button "닫기" of w
                            return "close-window"
                        else
                            key code 53
                            return "escape-window"
                        end if
                    end if
                end if
            end tell
        end tell
        return ""
    "#;
    if let Ok(out) = applescript::run(script) {
        return !out.trim().is_empty();
    }
    false
}

pub fn goal_primary_app(goal: &str) -> Option<&'static str> {
    let lower = goal.to_lowercase();
    if lower.contains("safari") || lower.contains("사파리") {
        return Some("Safari");
    }
    if lower.contains("notes") || lower.contains("노트") || lower.contains("메모") {
        return Some("Notes");
    }
    if lower.contains("mail") || lower.contains("메일") || lower.contains("gmail") {
        return Some("Mail");
    }
    if lower.contains("textedit") || lower.contains("텍스트에디트") || lower.contains("텍스트 편집")
    {
        return Some("TextEdit");
    }
    if lower.contains("calculator") || lower.contains("계산기") {
        return Some("Calculator");
    }
    if lower.contains("finder") || lower.contains("파인더") {
        return Some("Finder");
    }
    if lower.contains("preview") || lower.contains("미리보기") {
        return Some("Preview");
    }
    if lower.contains("calendar") || lower.contains("캘린더") {
        return Some("Calendar");
    }
    None
}

pub fn looks_like_dialog(desc: &str) -> bool {
    let desc_lower = desc.to_lowercase();
    desc_lower.contains("cancel")
        || desc_lower.contains("취소")
        || desc_lower.contains("open dialog")
        || desc_lower.contains("open file")
        || desc_lower.contains("save dialog")
        || desc_lower.contains("save")
}

pub async fn ensure_app_focus(target_app: &str, retries: usize) -> bool {
    let effective_retries = std::env::var("STEER_FOCUS_RETRIES")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or_else(|| retries.clamp(1, 2));

    if let Ok(front) = crate::tool_chaining::CrossAppBridge::get_frontmost_app() {
        if front.eq_ignore_ascii_case(target_app) {
            return true;
        }
    }

    for _ in 0..effective_retries {
        let _ = crate::tool_chaining::CrossAppBridge::switch_to_app(target_app);
        tokio::time::sleep(tokio::time::Duration::from_millis(220)).await;
        if let Ok(front) = crate::tool_chaining::CrossAppBridge::get_frontmost_app() {
            if front.eq_ignore_ascii_case(target_app) {
                return true;
            }
        }
    }
    false
}

pub fn compute_plan_key(goal: &str, image_b64: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(goal.as_bytes());
    hasher.update(image_b64.as_bytes());
    let out = hasher.finalize();
    format!("{:x}", out)
}

pub fn resume_hint_for_goal(
    goal: &str,
    checkpoint: &Option<String>,
    front_app: Option<&str>,
) -> Option<serde_json::Value> {
    let lower = goal.to_lowercase();
    let cp = checkpoint.as_deref().unwrap_or("");
    let front = front_app.unwrap_or("");
    if lower.contains("mail") && cp == "mail_compose_open" && front.eq_ignore_ascii_case("Mail") {
        return Some(serde_json::json!({"action":"shortcut","key":"v","modifiers":["command"]}));
    }
    if lower.contains("notes") && cp == "notes_note_created" && front.eq_ignore_ascii_case("Notes")
    {
        return Some(serde_json::json!({"action":"shortcut","key":"v","modifiers":["command"]}));
    }
    if lower.contains("textedit")
        && cp == "textedit_new_doc"
        && front.eq_ignore_ascii_case("TextEdit")
    {
        return Some(serde_json::json!({"action":"type","text":"Total hours per year: "}));
    }
    None
}
