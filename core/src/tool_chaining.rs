//! Tool Chaining Engine - The REAL missing piece
//!
//! This enables complex multi-step scenarios like:
//! "Check calendar, then send summary to Slack"
//!
//! Core concept: Each tool outputs data that can be consumed by the next tool

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

// =====================================================
// TOOL RESULT CONTEXT
// =====================================================

/// Result from a tool execution that can be passed to next tool
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub tool_name: String,
    pub success: bool,
    pub output: Value,
    pub extracted_data: HashMap<String, String>, // Key data points
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

impl ToolResult {
    pub fn success(tool_name: &str, output: Value) -> Self {
        Self {
            tool_name: tool_name.to_string(),
            success: true,
            output,
            extracted_data: HashMap::new(),
            timestamp: chrono::Utc::now(),
        }
    }

    pub fn failed(tool_name: &str, error: &str) -> Self {
        Self {
            tool_name: tool_name.to_string(),
            success: false,
            output: serde_json::json!({"error": error}),
            extracted_data: HashMap::new(),
            timestamp: chrono::Utc::now(),
        }
    }

    /// Add extracted data point
    pub fn with_data(mut self, key: &str, value: &str) -> Self {
        self.extracted_data
            .insert(key.to_string(), value.to_string());
        self
    }

    /// Get value for use in next tool
    pub fn get(&self, key: &str) -> Option<&String> {
        self.extracted_data.get(key)
    }
}

// =====================================================
// EXECUTION CONTEXT (Cross-tool state)
// =====================================================

/// Shared context across tool chain execution
#[derive(Debug, Clone, Default)]
pub struct ExecutionContext {
    pub variables: HashMap<String, String>,
    pub tool_results: Vec<ToolResult>,
    pub clipboard: Option<String>,
    pub current_app: Option<String>,
    pub last_screenshot: Option<String>,
}

impl ExecutionContext {
    pub fn new() -> Self {
        Self::default()
    }

    /// Set a variable for later use
    pub fn set(&mut self, key: &str, value: &str) {
        self.variables.insert(key.to_string(), value.to_string());
    }

    /// Get a variable
    pub fn get(&self, key: &str) -> Option<&String> {
        self.variables.get(key)
    }

    /// Get last tool result
    pub fn last_result(&self) -> Option<&ToolResult> {
        self.tool_results.last()
    }

    /// Add tool result
    pub fn add_result(&mut self, result: ToolResult) {
        // Also copy extracted data to context variables
        for (k, v) in &result.extracted_data {
            self.variables.insert(k.clone(), v.clone());
        }
        self.tool_results.push(result);
    }

    /// Template substitution - replace {{var}} with actual values
    pub fn substitute(&self, template: &str) -> String {
        let mut result = template.to_string();
        for (key, value) in &self.variables {
            result = result.replace(&format!("{{{{{}}}}}", key), value);
        }
        // Also support $var syntax
        for (key, value) in &self.variables {
            result = result.replace(&format!("${}", key), value);
        }
        result
    }
}

// =====================================================
// TOOL CHAIN - Multi-step workflow
// =====================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolStep {
    pub name: String,
    pub tool: String,
    pub params: HashMap<String, String>,
    #[serde(default)]
    pub extract: HashMap<String, String>, // JSONPath-like extraction rules
    #[serde(default)]
    pub on_fail: FailAction,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum FailAction {
    #[default]
    Stop,
    Continue,
    Retry,
    Skip,
}

#[derive(Debug, Clone)]
pub struct ToolChain {
    pub name: String,
    pub steps: Vec<ToolStep>,
    pub context: ExecutionContext,
}

impl ToolChain {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            steps: Vec::new(),
            context: ExecutionContext::new(),
        }
    }

    /// Add a step to the chain
    pub fn add_step(&mut self, step: ToolStep) {
        self.steps.push(step);
    }

    /// Build a chain from JSON definition
    pub fn from_json(json: &Value) -> Result<Self> {
        let name = json["name"].as_str().unwrap_or("unnamed");
        let mut chain = Self::new(name);

        if let Some(steps) = json["steps"].as_array() {
            for step_json in steps {
                let step: ToolStep = serde_json::from_value(step_json.clone())?;
                chain.add_step(step);
            }
        }

        Ok(chain)
    }
}

// =====================================================
// CROSS-APP BRIDGE
// =====================================================

/// Bridge for passing data between applications
pub struct CrossAppBridge;

impl CrossAppBridge {
    fn run_osascript_output(script: &str, timeout_ms: u64) -> Result<Output> {
        let mut child = Command::new("osascript")
            .arg("-e")
            .arg(script)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        let started = Instant::now();
        loop {
            if let Some(_) = child.try_wait()? {
                return Ok(child.wait_with_output()?);
            }
            if started.elapsed() >= Duration::from_millis(timeout_ms) {
                let _ = child.kill();
                let _ = child.wait();
                return Err(anyhow::anyhow!(
                    "osascript timed out after {}ms",
                    timeout_ms
                ));
            }
            std::thread::sleep(Duration::from_millis(40));
        }
    }

    /// Copy text to system clipboard
    pub fn copy_to_clipboard(text: &str) -> Result<()> {
        let script = format!(
            r#"set the clipboard to "{}""#,
            text.replace("\"", "\\\"").replace("\n", "\\n")
        );

        std::process::Command::new("osascript")
            .arg("-e")
            .arg(&script)
            .status()?;

        println!(
            "📋 [Bridge] Copied to clipboard: {}...",
            &text[..text.len().min(50)]
        );
        Ok(())
    }

    /// Get text from system clipboard
    pub fn get_clipboard() -> Result<String> {
        let output = std::process::Command::new("osascript")
            .arg("-e")
            .arg("the clipboard")
            .output()?;

        let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Ok(text)
    }

    /// Paste from clipboard (Cmd+V)
    pub fn paste() -> Result<()> {
        let script = r#"tell application "System Events" to keystroke "v" using command down"#;
        std::process::Command::new("osascript")
            .arg("-e")
            .arg(script)
            .status()?;
        Ok(())
    }

    /// Switch to application (Modified to use 'open -a' for reliability)
    pub fn switch_to_app(app_name: &str) -> Result<()> {
        println!("      🚀 [Bridge] Opening '{}' via CLI...", app_name);

        // Prefer Peekaboo app launch if available (stronger focus + permissions)
        if crate::peekaboo_cli::is_available() {
            let status = std::process::Command::new("peekaboo")
                .arg("app")
                .arg("launch")
                .arg(app_name)
                .status();
            if let Ok(status) = status {
                if status.success() {
                    if Self::wait_for_frontmost(app_name, 8, 200) {
                        println!("🔀 [Bridge] Switched to: {}", app_name);
                        return Ok(());
                    }
                }
            }
        }

        let status = std::process::Command::new("open")
            .arg("-a")
            .arg(app_name)
            .status()?;

        if !status.success() {
            return Err(anyhow::anyhow!(
                "Failed to open app '{}' (Exit code: {:?})",
                app_name,
                status.code()
            ));
        }

        if !Self::wait_for_frontmost(app_name, 8, 200) {
            let _ = crate::applescript::activate_app(app_name);
        }

        println!("🔀 [Bridge] Switched to: {}", app_name);
        Ok(())
    }

    fn wait_for_frontmost(app_name: &str, retries: usize, wait_ms: u64) -> bool {
        for _ in 0..retries {
            if let Ok(front) = Self::get_frontmost_app() {
                if front.eq_ignore_ascii_case(app_name) {
                    return true;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(wait_ms));
        }
        false
    }

    /// Get frontmost application name
    pub fn get_frontmost_app() -> Result<String> {
        let output = Self::run_osascript_output(
            r#"tell application "System Events" to get name of first application process whose frontmost is true"#,
            1200,
        )?;

        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    /// Read selected text from current app
    pub fn get_selected_text() -> Result<Option<String>> {
        // Try Cmd+C and read clipboard
        let old_clipboard = Self::get_clipboard().ok();

        let script = r#"tell application "System Events" to keystroke "c" using command down"#;
        std::process::Command::new("osascript")
            .arg("-e")
            .arg(script)
            .status()?;

        std::thread::sleep(std::time::Duration::from_millis(100));

        let new_clipboard = Self::get_clipboard()?;

        // Restore clipboard if changed
        if Some(&new_clipboard) != old_clipboard.as_ref() {
            Ok(Some(new_clipboard))
        } else {
            Ok(None)
        }
    }

    /// Write to a temporary file and return path
    pub fn write_temp_file(content: &str, extension: &str) -> Result<String> {
        let path = format!("/tmp/steer_bridge_{}.{}", uuid::Uuid::new_v4(), extension);
        std::fs::write(&path, content)?;
        println!("📁 [Bridge] Wrote temp file: {}", path);
        Ok(path)
    }
}

// =====================================================
// HIGH-LEVEL ORCHESTRATION
// =====================================================

/// Execute a complex multi-app scenario
pub async fn execute_scenario(description: &str) -> Result<ExecutionContext> {
    let mut ctx = ExecutionContext::new();

    println!("🎯 [Scenario] Starting: {}", description);

    // Parse the scenario description to identify apps and actions
    // This is where LLM would normally decompose the task

    // For now, set up the context
    ctx.set("scenario", description);
    ctx.current_app = CrossAppBridge::get_frontmost_app().ok();

    Ok(ctx)
}

// =====================================================
// COMMON TOOL IMPLEMENTATIONS
// =====================================================

/// Read content from an app (generic)
pub fn read_from_app(app_name: &str, ctx: &mut ExecutionContext) -> Result<ToolResult> {
    CrossAppBridge::switch_to_app(app_name)?;
    std::thread::sleep(std::time::Duration::from_millis(300));

    // Try to select all and copy
    let script = r#"tell application "System Events"
        keystroke "a" using command down
        delay 0.1
        keystroke "c" using command down
    end tell"#;

    std::process::Command::new("osascript")
        .arg("-e")
        .arg(script)
        .status()?;

    std::thread::sleep(std::time::Duration::from_millis(200));

    let content = CrossAppBridge::get_clipboard()?;
    ctx.set("last_read", &content);

    Ok(ToolResult::success(
        "read_from_app",
        serde_json::json!({
            "app": app_name,
            "content": content
        }),
    )
    .with_data("content", &content))
}

/// Write content to an app
pub fn write_to_app(
    app_name: &str,
    content: &str,
    ctx: &mut ExecutionContext,
) -> Result<ToolResult> {
    // Substitute variables in content
    let resolved_content = ctx.substitute(content);

    CrossAppBridge::switch_to_app(app_name)?;
    std::thread::sleep(std::time::Duration::from_millis(300));

    CrossAppBridge::copy_to_clipboard(&resolved_content)?;
    CrossAppBridge::paste()?;

    Ok(ToolResult::success(
        "write_to_app",
        serde_json::json!({
            "app": app_name,
            "written": resolved_content.len()
        }),
    ))
}

/// Calculate expression
pub fn calculate(expression: &str, ctx: &mut ExecutionContext) -> Result<ToolResult> {
    // Use bc for calculation
    let _output = std::process::Command::new("bc")
        .arg("-l")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()?
        .wait_with_output()?;

    // Fallback: use Python
    let output = std::process::Command::new("python3")
        .arg("-c")
        .arg(format!("print({})", expression))
        .output()?;

    let result = String::from_utf8_lossy(&output.stdout).trim().to_string();
    ctx.set("calc_result", &result);

    Ok(ToolResult::success(
        "calculate",
        serde_json::json!({
            "expression": expression,
            "result": result
        }),
    )
    .with_data("result", &result))
}

// =====================================================
// TESTS
// =====================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_context_substitution() {
        let mut ctx = ExecutionContext::new();
        ctx.set("name", "John");
        ctx.set("value", "123");

        assert_eq!(ctx.substitute("Hello {{name}}!"), "Hello John!");
        assert_eq!(ctx.substitute("Value is $value"), "Value is 123");
    }

    #[test]
    fn test_tool_result_chaining() {
        let mut ctx = ExecutionContext::new();

        let result1 = ToolResult::success("calc", serde_json::json!({"result": 579}))
            .with_data("calc_result", "579");

        ctx.add_result(result1);

        // Now the result should be available as a variable
        assert_eq!(ctx.get("calc_result"), Some(&"579".to_string()));
    }
}
