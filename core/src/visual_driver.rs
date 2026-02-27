use crate::applescript;
use anyhow::{Context, Result};
use base64::{engine::general_purpose, Engine as _};
use std::fs;
use std::io::Cursor;
use std::process::Command;

use crate::error::AppError;
use serde::{Deserialize, Serialize};
use tracing::{debug, error, info, warn};

// =====================================================
// Clawdbot-inspired helper functions
// =====================================================

/// Convert error to AI-friendly format (clawdbot pattern: toAIFriendlyError)
/// Provides actionable context for the LLM to retry with a different approach
pub fn to_ai_friendly_error(err: anyhow::Error, context: &str) -> anyhow::Error {
    anyhow::anyhow!(
        "Failed to {}: {} (Try a different approach - the element may have moved or be unavailable)",
        context,
        err
    )
}

/// Normalize timeout values to safe bounds (clawdbot pattern: normalizeTimeoutMs)
pub fn normalize_timeout_ms(value: Option<u64>, default: u64, max: u64) -> u64 {
    value.map(|v| v.max(500).min(max)).unwrap_or(default)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum UiAction {
    OpenUrl(String),
    Wait(u64),           // Seconds
    Click(String),       // Element description or AppleScript target
    ClickVisual(String), // Vision-based click: "Click the blue submit button"
    Type(String),
    Scroll(String),      // "down" | "up"
    ActivateApp(String), // "frontmost" or app name
    KeyboardShortcut(String, Vec<String>), // key, modifiers (e.g. "n", ["command"])
                         // Verify(String), // Removed: Legacy standalone verify unused
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SmartStep {
    pub action: UiAction,
    pub description: String,
    pub pre_verify: Option<String>, // Prompt for checking BEFORE action
    pub post_verify: Option<String>, // Prompt for checking AFTER action
    pub critical: bool,             // Stop on failure?
}

impl SmartStep {
    pub fn new(action: UiAction, desc: &str) -> Self {
        Self {
            action,
            description: desc.to_string(),
            pre_verify: None,
            post_verify: None,
            critical: true,
        }
    }

    pub fn with_pre_check(mut self, prompt: &str) -> Self {
        self.pre_verify = Some(prompt.to_string());
        self
    }

    pub fn with_post_check(mut self, prompt: &str) -> Self {
        self.post_verify = Some(prompt.to_string());
        self
    }
}

pub struct VisualDriver {
    pub steps: Vec<SmartStep>,
}

fn should_fallback_to_native_type(err_text: &str) -> bool {
    let lower = err_text.to_lowercase();
    lower.contains("1002")
        || lower.contains("keystroke")
        || lower.contains("허용되지 않습니다")
        || lower.contains("not allowed to send keystrokes")
}

impl Default for VisualDriver {
    fn default() -> Self {
        Self::new()
    }
}

impl VisualDriver {
    pub fn new() -> Self {
        Self { steps: Vec::new() }
    }

    pub fn capture_screen() -> Result<(String, f32)> {
        // ... (existing code, ensure it returns result)
        // I will just reuse the existing implementation
        Self::capture_screen_internal(true)
    }

    // Internal helper that can skip optimization if needed, but for now we keep it same.
    // Actually, to implement diffing efficiently, we might want the raw DynamicImage, but keeping B64 interface is easier for now to avoid refactoring everything.
    // Or better, let's just make a new helper that returns the DynamicImage for internal use.

    fn capture_image_internal() -> Result<image::DynamicImage> {
        let uuid = uuid::Uuid::new_v4();
        let output_path = format!("/tmp/steer_vision_{}.jpg", uuid);

        let status = Command::new("screencapture")
            .arg("-x")
            .arg("-t")
            .arg("jpg")
            .arg("-C")
            .arg(&output_path)
            .status()
            .context("Failed to run screencapture")?;

        if !status.success() {
            return Err(AppError::Vision("screencapture failed".to_string()).into());
        }

        let image_data = fs::read(&output_path).context("Failed to read image")?;
        let _ = fs::remove_file(&output_path);

        image::load_from_memory(&image_data).context("Failed to load image")
    }

    pub fn capture_screen_internal(optimize: bool) -> Result<(String, f32)> {
        let img = Self::capture_image_internal()?;

        let (orig_w, _orig_h) = (img.width(), img.height());
        // ... (resizing logic from before)
        let max_dim = 1920u32;
        let scale_factor = if optimize && orig_w > max_dim {
            orig_w as f32 / max_dim as f32
        } else {
            1.0
        };

        let resized = if scale_factor > 1.0 {
            img.resize(max_dim, max_dim, image::imageops::FilterType::Triangle)
        } else {
            img
        };

        let mut buffer = Cursor::new(Vec::new());
        resized.write_to(&mut buffer, image::ImageOutputFormat::Jpeg(80))?;
        let b64 = general_purpose::STANDARD.encode(buffer.get_ref());
        Ok((b64, scale_factor))
    }

    /// Calculate percentage difference between two images (0.0 to 1.0)
    fn calculate_diff(img1: &image::DynamicImage, img2: &image::DynamicImage) -> f64 {
        use image::GenericImageView;
        let (w1, h1) = img1.dimensions();
        let (w2, h2) = img2.dimensions();

        if w1 != w2 || h1 != h2 {
            return 1.0;
        } // Changed resolution is a big diff

        // Simple pixel diff (could be optimized with checking random samples for speed)
        // For 1 second wait, full scan might be slow if 4K.
        // Let's resize both to small thumbnails for comparison (e.g., 256x ?)
        let thumb1 = img1.resize_exact(256, 144, image::imageops::FilterType::Nearest);
        let thumb2 = img2.resize_exact(256, 144, image::imageops::FilterType::Nearest);

        let mut diff_pixels = 0;
        let total_pixels = 256 * 144;

        for y in 0..144 {
            for x in 0..256 {
                let p1 = thumb1.get_pixel(x, y);
                let p2 = thumb2.get_pixel(x, y);

                // RGB Euclidean distance
                let r_diff = (p1[0] as i32 - p2[0] as i32).abs();
                let g_diff = (p1[1] as i32 - p2[1] as i32).abs();
                let b_diff = (p1[2] as i32 - p2[2] as i32).abs();

                if r_diff + g_diff + b_diff > 30 {
                    // Sensitivity threshold
                    diff_pixels += 1;
                }
            }
        }

        diff_pixels as f64 / total_pixels as f64
    }

    /// Adaptive Wait: Wait until the screen is STABLE (Diff < threshold)
    /// This ensures we don't act during animations, but proceed immediately when static.
    /// Returns Ok(true) if settled, Ok(false) if timed out.
    pub async fn wait_for_ui_settle(timeout_ms: u64) -> Result<bool> {
        let start = std::time::Instant::now();
        let threshold = 0.005; // 0.5% diff = stricter stability
        let mut consecutive_stable_frames = 0;
        let required_stable_frames = 3; // ~600ms of stability (200ms interval * 3)

        let mut prev_img = match Self::capture_image_internal() {
            Ok(img) => img,
            Err(_) => return Ok(true), // Fail safe: assume stable if capture fails (to avoid blocking)
        };

        loop {
            if start.elapsed().as_millis() as u64 > timeout_ms {
                warn!(
                    "      ⏰ Timeout waiting for UI settle (waited {}ms). Unstable.",
                    timeout_ms
                );
                return Ok(false);
            }

            tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

            if let Ok(new_img) = Self::capture_image_internal() {
                let diff = Self::calculate_diff(&prev_img, &new_img);
                if diff < threshold {
                    consecutive_stable_frames += 1;
                    if consecutive_stable_frames >= required_stable_frames {
                        info!("      ⚡️ UI Settled (Ready).");
                        return Ok(true);
                    }
                } else {
                    consecutive_stable_frames = 0;
                    info!("      🌊 UI Moving ({:.1}% diff)...", diff * 100.0);
                }
                prev_img = new_img;
            }
        }
    }

    pub fn add_step(&mut self, step: SmartStep) -> &mut Self {
        self.steps.push(step);
        self
    }

    // Helper for legacy support
    pub fn add_legacy_step(&mut self, action: UiAction) -> &mut Self {
        self.steps.push(SmartStep::new(action, "Legacy Step"));
        self
    }

    async fn verify_condition(
        llm: &dyn crate::llm_gateway::LLMClient,
        prompt: &str,
    ) -> Result<bool> {
        info!("      👁️ Vision Check: '{}'", prompt);
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await; // Brief pause before capture

        match Self::capture_screen() {
            Ok((b64, _scale)) => {
                // Ignore scale for verification
                let full_prompt = format!(
                    "Screen Verification Task.\nCondition to verify: '{}'.\nReply ONLY with 'YES' or 'NO'.",
                    prompt
                );
                match llm.analyze_screen(&full_prompt, &b64).await {
                    Ok(resp) => {
                        let success = resp.trim().to_uppercase().starts_with("YES");
                        info!("      🤖 Result: {}", if success { "PASS" } else { "FAIL" });
                        Ok(success)
                    }
                    Err(e) => {
                        warn!("      ⚠️ Vision API Error: {}", e);
                        Ok(false) // Conservative failure
                    }
                }
            }
            Err(e) => {
                warn!("      ⚠️ Capture Failed: {}", e);
                Ok(false)
            }
        }
    }

    pub async fn execute(&self, llm: Option<&dyn crate::llm_gateway::LLMClient>) -> Result<()> {
        info!("👻 [Smart Visual Driver] Starting Verified Automation...");

        for (i, step) in self.steps.iter().enumerate() {
            info!("   Step {}: {}", i + 1, step.description);

            // 1. Pre-Verification
            if let Some(pre_prompt) = &step.pre_verify {
                if let Some(brain) = llm {
                    if !Self::verify_condition(brain, pre_prompt).await? {
                        if step.critical {
                            return Err(AppError::Execution(format!(
                                "❌ Pre-check failed: {}",
                                pre_prompt
                            ))
                            .into());
                        } else {
                            warn!("      ⚠️ Pre-check failed, but proceeding (non-critical).");
                        }
                    }
                }
            }

            // 2. Action Execution
            match &step.action {
                UiAction::OpenUrl(url) => {
                    crate::applescript::open_url(url).map(|_| ())?;
                }
                UiAction::Wait(secs) => {
                    tokio::time::sleep(tokio::time::Duration::from_secs(*secs)).await;
                }
                UiAction::Click(target) => {
                    // Use frontmost application instead of hardcoded Safari
                    let target_clone = target.clone();
                    let script = format!(
                        "tell application \"System Events\" to click button {:?} of window 1 of (first application process whose frontmost is true)",
                        target_clone
                    );

                    if crate::env_flag("STEER_ADAPTIVE_POLLING") {
                        info!("      ⏳ Adaptive Polling: Waiting for UI settle...");
                        let _ = Self::wait_for_ui_settle(2000).await;
                    }

                    // [Survival] Run blocking script with timeout
                    let task = tokio::task::spawn_blocking(move || applescript::run(&script));

                    match tokio::time::timeout(std::time::Duration::from_secs(5), task).await {
                        Ok(Ok(Ok(_))) => {} // Success
                        Ok(Ok(Err(e))) => {
                            warn!("      (Click failed: {})", e);
                            if step.critical {
                                return Err(AppError::Execution(format!(
                                    "Critical Click Failed: {}",
                                    e
                                ))
                                .into());
                            }
                        }
                        Ok(Err(_)) => {
                            // JoinError
                            return Err(anyhow::anyhow!("Task Panic"));
                        }
                        Err(_) => {
                            // Timeout
                            warn!("      (Click timed out)");
                            if step.critical {
                                return Err(anyhow::anyhow!("Critical Click Timed Out"));
                            }
                        }
                    }
                }
                UiAction::Type(text) => {
                    let text_clone = text.clone();
                    let text_for_native_fallback = text.clone();
                    let compact: String =
                        text_clone.chars().filter(|c| !c.is_whitespace()).collect();
                    let calc_like = !compact.is_empty()
                        && compact.chars().all(|c| {
                            c.is_ascii_digit()
                                || matches!(c, '+' | '-' | '*' | '/' | '=' | '.' | ',' | '(' | ')')
                        });

                    // For non-calculator text, prefer clipboard-paste typing for reliability (especially multiline).
                    let task = tokio::task::spawn_blocking(move || {
                        if calc_like {
                            let script = format!(
                                "tell application \"System Events\" to keystroke {:?}",
                                text_clone
                            );
                            applescript::run(&script)
                        } else {
                            let lines = [
                                "on run argv",
                                "set targetText to item 1 of argv",
                                "set oldClipboard to the clipboard",
                                "set the clipboard to targetText",
                                "tell application \"System Events\" to keystroke \"v\" using {command down}",
                                // Give target app enough time to consume clipboard before restoring.
                                "delay 0.35",
                                "try",
                                "set the clipboard to oldClipboard",
                                "end try",
                                "return \"ok\"",
                                "end run",
                            ];
                            applescript::run_with_args(&lines, &[text_clone])
                        }
                    });

                    match tokio::time::timeout(std::time::Duration::from_secs(5), task).await {
                        Ok(Ok(Ok(_))) => {}
                        Ok(Ok(Err(e))) => {
                            #[cfg(target_os = "macos")]
                            {
                                let err_text = e.to_string();
                                if should_fallback_to_native_type(&err_text) {
                                    warn!(
                                        "      (Type permission issue detected, trying native fallback)"
                                    );
                                    let fallback_text = text_for_native_fallback.clone();
                                    let fallback = tokio::task::spawn_blocking(move || {
                                        crate::macos::actions::type_text(&fallback_text)
                                    });
                                    match tokio::time::timeout(
                                        std::time::Duration::from_secs(5),
                                        fallback,
                                    )
                                    .await
                                    {
                                        Ok(Ok(Ok(_))) => {}
                                        Ok(Ok(Err(e2))) => {
                                            return Err(anyhow::anyhow!(
                                                "Type Failed: {} | Native fallback failed: {}",
                                                err_text,
                                                e2
                                            ));
                                        }
                                        Ok(Err(_)) => {
                                            return Err(anyhow::anyhow!(
                                                "Type Failed: {} | Native fallback task panic",
                                                err_text
                                            ));
                                        }
                                        Err(_) => {
                                            return Err(anyhow::anyhow!(
                                                "Type Failed: {} | Native fallback timed out",
                                                err_text
                                            ));
                                        }
                                    }
                                } else {
                                    return Err(anyhow::anyhow!("Type Failed: {}", err_text));
                                }
                            }
                            #[cfg(not(target_os = "macos"))]
                            {
                                return Err(anyhow::anyhow!("Type Failed: {}", e));
                            }
                        }
                        Ok(Err(_)) => return Err(anyhow::anyhow!("Task Panic")),
                        Err(_) => return Err(anyhow::anyhow!("Type Timed Out")),
                    }
                }
                UiAction::Scroll(direction) => {
                    let dir = direction.to_lowercase();
                    let key_code = if dir == "up" { 116 } else { 121 }; // page up/down
                    let script = format!(
                        "tell application \"System Events\" to key code {}",
                        key_code
                    );
                    let task = tokio::task::spawn_blocking(move || applescript::run(&script));
                    match tokio::time::timeout(std::time::Duration::from_secs(5), task).await {
                        Ok(Ok(Ok(_))) => {}
                        Ok(Ok(Err(e))) => return Err(anyhow::anyhow!("Scroll Failed: {}", e)),
                        Ok(Err(_)) => return Err(anyhow::anyhow!("Task Panic")),
                        Err(_) => return Err(anyhow::anyhow!("Scroll Timed Out")),
                    }
                }
                UiAction::ActivateApp(app) => {
                    let app_name = app.clone();
                    let task = tokio::task::spawn_blocking(move || {
                        if app_name.to_lowercase() == "frontmost" {
                            applescript::activate_frontmost_app()
                        } else {
                            applescript::activate_app(&app_name)
                        }
                    });
                    match tokio::time::timeout(std::time::Duration::from_secs(5), task).await {
                        Ok(Ok(Ok(_))) => {}
                        Ok(Ok(Err(e))) => return Err(anyhow::anyhow!("Activate Failed: {}", e)),
                        Ok(Err(_)) => return Err(anyhow::anyhow!("Task Panic")),
                        Err(_) => return Err(anyhow::anyhow!("Activate Timed Out")),
                    }
                }
                UiAction::KeyboardShortcut(key, modifiers) => {
                    // [FIX] Wait a moment to ensure the app is fully activated
                    tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;

                    // Helper function to get current app name
                    fn get_frontmost_app() -> Result<String, anyhow::Error> {
                        let script = r#"
                            tell application "System Events"
                                name of first application process whose frontmost is true
                            end tell
                        "#;
                        applescript::run(script)
                    }

                    // Helper function to execute menu click
                    async fn execute_menu_click(
                        app: &str,
                        menu_items: &str,
                        description: &str,
                    ) -> Result<(), anyhow::Error> {
                        info!("      🔧 [WORKAROUND] Using menu click for {}", description);
                        let script = format!(
                            r#"
                            tell application "System Events"
                                tell process "{}"
                                    {}
                                end tell
                            end tell
                        "#,
                            app, menu_items
                        );

                        let task = tokio::task::spawn_blocking(move || applescript::run(&script));

                        match tokio::time::timeout(std::time::Duration::from_secs(5), task).await {
                            Ok(Ok(Ok(_))) => Ok(()),
                            Ok(Ok(Err(e))) => Err(anyhow::anyhow!("Menu click failed: {}", e)),
                            Ok(Err(_)) => Err(anyhow::anyhow!("Task Panic")),
                            Err(_) => Err(anyhow::anyhow!("Menu click timed out")),
                        }
                    }

                    // Check if we need menu click workaround
                    if let Ok(app_name) = get_frontmost_app() {
                        let app = app_name.trim();
                        let is_notes = app.eq_ignore_ascii_case("Notes") || app == "메모";
                        let is_textedit =
                            app.eq_ignore_ascii_case("TextEdit") || app == "텍스트 편집기";
                        let is_calculator =
                            app.eq_ignore_ascii_case("Calculator") || app == "계산기";

                        // WORKAROUND: Cmd+A (Select All)
                        if key == "a" && modifiers.contains(&"command".to_string()) {
                            if is_notes {
                                let result = execute_menu_click(
                                    app,
                                    r#"click menu item "모두 선택" of menu "편집" of menu bar 1"#,
                                    "Notes Cmd+A",
                                )
                                .await;
                                if result.is_ok() {
                                    return Ok(());
                                }
                            } else if is_textedit {
                                let result = execute_menu_click(
                                    app,
                                    r#"click menu item "모두 선택" of menu "편집" of menu bar 1"#,
                                    "TextEdit Cmd+A",
                                )
                                .await;
                                if result.is_ok() {
                                    return Ok(());
                                }
                            }
                        }

                        // WORKAROUND: Cmd+C (Copy)
                        if key == "c" && modifiers.contains(&"command".to_string()) {
                            if is_notes {
                                let result = execute_menu_click(
                                    app,
                                    r#"click menu item "복사" of menu "편집" of menu bar 1"#,
                                    "Notes Cmd+C",
                                )
                                .await;
                                if result.is_ok() {
                                    return Ok(());
                                }
                            } else if is_textedit {
                                let result = execute_menu_click(
                                    app,
                                    r#"click menu item "복사" of menu "편집" of menu bar 1"#,
                                    "TextEdit Cmd+C",
                                )
                                .await;
                                if result.is_ok() {
                                    return Ok(());
                                }
                            }
                        } else if is_calculator {
                            // Try English first, then Korean
                            let english_result = execute_menu_click(
                                app,
                                r#"click menu item "Copy" of menu "Edit" of menu bar 1"#,
                                "Calculator Cmd+C (En)",
                            )
                            .await;

                            if english_result.is_ok() {
                                return Ok(());
                            } else {
                                // Fallback to Korean
                                let korean_result = execute_menu_click(
                                    app,
                                    r#"click menu item "복사" of menu "편집" of menu bar 1"#,
                                    "Calculator Cmd+C (Ko)",
                                )
                                .await;
                                if korean_result.is_ok() {
                                    return Ok(());
                                }
                            }
                        }

                        // WORKAROUND: Cmd+N (New Note/Document)
                        if key == "n" && modifiers.contains(&"command".to_string()) && is_notes {
                            let result = execute_menu_click(
                                app,
                                r#"click menu item "새로운 메모" of menu "파일" of menu bar 1"#,
                                "Notes Cmd+N",
                            )
                            .await;
                            if result.is_ok() {
                                return Ok(());
                            }
                        }

                        // WORKAROUND: Cmd+V (Paste) - Notes only
                        if key == "v" && modifiers.contains(&"command".to_string()) && is_notes {
                            let result = execute_menu_click(
                                app,
                                r#"click menu item "붙여넣기" of menu "편집" of menu bar 1"#,
                                "Notes Cmd+V",
                            )
                            .await;
                            if result.is_ok() {
                                return Ok(());
                            }
                        }
                    }

                    let key_str = key.clone();
                    let mods_str = modifiers
                        .iter()
                        .map(|m| format!("{} down", m))
                        .collect::<Vec<_>>()
                        .join(", ");

                    // Construct AppleScript: keystroke "n" using {command down}
                    let script = if key_str.eq_ignore_ascii_case("escape")
                        || key_str.eq_ignore_ascii_case("esc")
                    {
                        "tell application \"System Events\" to key code 53".to_string()
                    } else if modifiers.is_empty() {
                        format!(
                            "tell application \"System Events\" to keystroke \"{}\"",
                            key_str
                        )
                    } else {
                        // CORRECT SYNTAX: using {command down} WITH BRACES!
                        format!(
                            "tell application \"System Events\" to keystroke \"{}\" using {{{}}}",
                            key_str, mods_str
                        )
                    };

                    info!("      ⌨️ Shortcut: {} + {:?}", key_str, modifiers);
                    debug!("      🔍 [DEBUG] AppleScript Command: {}", script);
                    let task = tokio::task::spawn_blocking(move || applescript::run(&script));
                    match tokio::time::timeout(std::time::Duration::from_secs(5), task).await {
                        Ok(Ok(Ok(_))) => {}
                        Ok(Ok(Err(e))) => return Err(anyhow::anyhow!("Shortcut Failed: {}", e)),
                        Ok(Err(_)) => return Err(anyhow::anyhow!("Task Panic")),
                        Err(_) => return Err(anyhow::anyhow!("Shortcut Timed Out")),
                    }
                }
                UiAction::ClickVisual(desc) => {
                    info!("      👁️ Vision Click: Finding '{}'...", desc);
                    if let Some(brain) = llm {
                        let max_retries = 2;
                        let find_timeout_ms = normalize_timeout_ms(
                            std::env::var("STEER_CLICK_VISUAL_TIMEOUT_MS")
                                .ok()
                                .and_then(|v| v.parse::<u64>().ok()),
                            8000,
                            30000,
                        );
                        let click_timeout_ms = normalize_timeout_ms(
                            std::env::var("STEER_CLICK_EXEC_TIMEOUT_MS")
                                .ok()
                                .and_then(|v| v.parse::<u64>().ok()),
                            5000,
                            15000,
                        );
                        for attempt in 0..=max_retries {
                            if attempt > 0 {
                                info!(
                                    "      ⏳ Retry {}/{}: Re-observing screen...",
                                    attempt, max_retries
                                );
                                tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
                            } else if crate::env_flag("STEER_ADAPTIVE_POLLING") {
                                info!("      ⏳ Adaptive Polling: Waiting for UI settle before Vision...");
                                let _ = Self::wait_for_ui_settle(2000).await;
                            }

                            debug!("      📸 Capturing screen for Vision Click...");
                            match Self::capture_screen() {
                                Ok((b64, scale)) => {
                                    debug!("      🔍 Calling find_element_coordinates (image size: {} bytes)...", b64.len());
                                    let coord_result = tokio::time::timeout(
                                        tokio::time::Duration::from_millis(find_timeout_ms),
                                        brain.find_element_coordinates(desc, &b64),
                                    )
                                    .await;

                                    match coord_result {
                                        Err(_) => {
                                            warn!(
                                                "      ⚠️ Vision coordinate lookup timed out after {}ms.",
                                                find_timeout_ms
                                            );
                                            if attempt == max_retries && step.critical {
                                                return Err(anyhow::anyhow!(
                                                    "Visual Click LLM timeout"
                                                ));
                                            }
                                        }
                                        Ok(Err(e)) => {
                                            error!("      ⚠️ LLM Vision Error: {}", e);
                                            if attempt == max_retries && step.critical {
                                                return Err(anyhow::anyhow!(
                                                    "Visual Click LLM error"
                                                ));
                                            }
                                        }
                                        Ok(Ok(Some((x_raw, y_raw)))) => {
                                            // Apply scaling back to original screen size
                                            let x = (x_raw as f32 * scale) as i32;
                                            let y = (y_raw as f32 * scale) as i32;
                                            info!(
                                                "      🎯 LLM Target: ({}, {}) [Scaled x{:.2}]",
                                                x, y, scale
                                            );

                                            // [Phase 4] Hybrid Grounding (macOS only)
                                            #[cfg(target_os = "macos")]
                                            {
                                                use crate::macos::accessibility;
                                                if let Some((_sx, _sy)) =
                                                    accessibility::get_element_center_at(x, y)
                                                {
                                                    info!("      🧲 Grounded: Valid UI Element confirmed at ({}, {})", x, y);
                                                } else {
                                                    warn!("      ⚠️  Warning: No UI Element found at coordinates via Accessibility API.");
                                                    // Optional: If we are strict, we could continue (retry) here.
                                                    // For now, we trust vision but warn.
                                                }
                                            }

                                            let script = format!("tell application \"System Events\" to click at {{{}, {}}}", x, y);
                                            debug!("      🖱️ Executing AppleScript: {}", script);
                                            let click_script = script.clone();
                                            let click_task =
                                                tokio::task::spawn_blocking(move || {
                                                    applescript::run(&click_script)
                                                });
                                            let click_result = tokio::time::timeout(
                                                tokio::time::Duration::from_millis(
                                                    click_timeout_ms,
                                                ),
                                                click_task,
                                            )
                                            .await;

                                            match click_result {
                                                Ok(Ok(Ok(_))) => {
                                                    info!("      ✅ Click executed successfully!");
                                                    break; // Success!
                                                }
                                                Ok(Ok(Err(e))) => {
                                                    error!(
                                                        "      ❌ Click visual script failed: {}",
                                                        e
                                                    );
                                                    if attempt == max_retries && step.critical {
                                                        return Err(anyhow::anyhow!(
                                                            "Visual Click execution failed"
                                                        ));
                                                    }
                                                }
                                                Ok(Err(_)) => {
                                                    error!("      ❌ Click visual task panicked.");
                                                    if attempt == max_retries && step.critical {
                                                        return Err(anyhow::anyhow!(
                                                            "Visual Click task panic"
                                                        ));
                                                    }
                                                }
                                                Err(_) => {
                                                    warn!(
                                                        "      ⚠️ Click visual execution timed out after {}ms.",
                                                        click_timeout_ms
                                                    );
                                                    if attempt == max_retries && step.critical {
                                                        return Err(anyhow::anyhow!(
                                                            "Visual Click execution timeout"
                                                        ));
                                                    }
                                                }
                                            }
                                        }
                                        Ok(Ok(None)) => {
                                            warn!(
                                                "      ⚠️ Element '{}' not found on screen.",
                                                desc
                                            );
                                            if attempt == max_retries && step.critical {
                                                return Err(anyhow::anyhow!(
                                                    "Visual Element '{}' not found after retries",
                                                    desc
                                                ));
                                            }
                                        }
                                    }
                                }
                                Err(e) => {
                                    error!("      ❌ Screen capture failed: {}", e);
                                    if attempt == max_retries && step.critical {
                                        return Err(anyhow::anyhow!("Screen capture failed"));
                                    }
                                }
                            }
                        }
                    } else {
                        warn!("      ⚠️ No LLM client provided for Visual Click.");
                    }
                }
            }

            // 3. Post-Verification
            if let Some(post_prompt) = &step.post_verify {
                if let Some(brain) = llm {
                    // Wait a bit for UI to settle (Strict)
                    if !Self::wait_for_ui_settle(4000).await? {
                        if step.critical {
                            return Err(anyhow::anyhow!("❌ Post-action UI failed to settle."));
                        } else {
                            warn!(
                                "      ⚠️ UI unstable after action, but proceeding (non-critical)."
                            );
                        }
                    }

                    if !Self::verify_condition(brain, post_prompt).await? && step.critical {
                        return Err(anyhow::anyhow!("❌ Post-check failed: {}", post_prompt));
                    }
                }
            }
        }

        info!("👻 [Smart Visual Driver] Automation Complete.");
        Ok(())
    }
}

// Pre-built sequences (Updated)
pub fn n8n_fallback_create_workflow() -> VisualDriver {
    let mut driver = VisualDriver::new();
    // Legacy support wrapper
    driver
        .add_legacy_step(UiAction::OpenUrl("https://app.n8n.cloud".to_string()))
        .add_legacy_step(UiAction::Wait(5))
        .add_legacy_step(UiAction::Click("Create Workflow".to_string()));
    driver
}
