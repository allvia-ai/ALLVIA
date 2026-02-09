//! CLI-based LLM client for Steer Agent
//!
//! Supports Codex, Gemini, and Claude CLI tools for LLM execution.
//! Uses login-based authentication (no API keys required).

use anyhow::{anyhow, Result};
use log::{debug, warn};
use std::process::Command; // Added missing imports

// Removed unused imports: Write, Stdio, Duration, FromStr

/// Supported CLI LLM providers
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LLMProvider {
    Codex,
    Gemini,
    Claude,
}

impl LLMProvider {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "codex" => Some(Self::Codex),
            "gemini" => Some(Self::Gemini),
            "claude" => Some(Self::Claude),
            _ => None,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Gemini => "gemini",
            Self::Claude => "claude",
        }
    }
}

/// CLI LLM Client - executes prompts via CLI tools
pub struct CLILLMClient {
    provider: LLMProvider,
    #[allow(dead_code)]
    timeout_sec: u64,
    cwd: Option<String>,
}

impl CLILLMClient {
    pub fn new(provider: LLMProvider) -> Self {
        let timeout = std::env::var("STEER_CLI_TIMEOUT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(120);

        Self {
            provider,
            timeout_sec: timeout,
            cwd: None,
        }
    }

    pub fn with_cwd(mut self, cwd: &str) -> Self {
        self.cwd = Some(cwd.to_string());
        self
    }

    /// Get provider from environment variable
    pub fn from_env() -> Option<Self> {
        println!("[CLI DEBUG] Checking STEER_CLI_LLM env var...");
        match std::env::var("STEER_CLI_LLM") {
            Ok(val) => {
                println!("[CLI DEBUG] Found STEER_CLI_LLM={}", val);
                let provider = LLMProvider::from_str(&val)?;
                // Use /tmp to prevent Gemini CLI from analyzing project code
                let mut client = Self::new(provider);
                client.cwd = Some("/tmp".to_string());
                Some(client)
            }
            Err(_) => {
                println!("[CLI DEBUG] STEER_CLI_LLM NOT set.");
                None
            }
        }
    }

    pub fn uses_stdin(&self) -> bool {
        matches!(self.provider, LLMProvider::Codex | LLMProvider::Claude)
    }

    /// Check if CLI is available
    pub fn check_version(&self) -> Result<String> {
        let cmd = match self.provider {
            LLMProvider::Codex => "codex",
            LLMProvider::Gemini => "gemini",
            LLMProvider::Claude => "claude",
        };

        let output = Command::new(cmd)
            .arg("--version")
            .output()
            .map_err(|e| anyhow!("{} CLI not found: {}", cmd, e))?;

        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
        } else {
            Err(anyhow!("{} CLI not configured or not logged in", cmd))
        }
    }

    /// Execute prompt via CLI
    /// Execute prompt and return raw output
    /// Execute prompt and return raw output
    pub fn execute(&self, prompt: &str) -> Result<String> {
        debug!("Preparing to execute CLI LLM...");
        let mut cmd = match self.provider {
            LLMProvider::Codex => std::process::Command::new("codex"),
            LLMProvider::Gemini => std::process::Command::new("gemini"),
            LLMProvider::Claude => std::process::Command::new("claude"),
        };

        // Configure arguments based on provider
        let use_stdin = match self.provider {
            LLMProvider::Codex => {
                cmd.args(&[
                    "exec",
                    "--sandbox",
                    "danger-full-access",
                    "--skip-git-repo-check",
                    "--color",
                    "never",
                    "-",
                ]);
                true
            }
            LLMProvider::Gemini => {
                // Gemini CLI hangs on stdin pipe (triggers Agent mode).
                // We must use positional argument with --sandbox.
                // ARG_MAX is ~1MB on macOS, usually sufficient for resized screenshots (100-400KB).
                cmd.arg("--sandbox");
                cmd.arg(prompt);
                false
            }
            LLMProvider::Claude => {
                cmd.args(&["--dangerously-skip-permissions", "-p", "-"]);
                true
            }
        };

        // Set Cwd if specified (Only if NOT Gemini, as Gemini enforces workspace match)
        if let Some(cwd) = &self.cwd {
            if !matches!(self.provider, LLMProvider::Gemini) {
                debug!("Setting CWD to: {}", cwd);
                cmd.current_dir(cwd);
            }
        }

        // Spawn process
        if use_stdin {
            cmd.stdin(std::process::Stdio::piped());
        } else {
            cmd.stdin(std::process::Stdio::null());
        }

        cmd.stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        debug!("Spawning command (Args: {} bytes)...", prompt.len());
        let mut child = cmd.spawn()?;

        // Write prompt to stdin ONLY if using stdin
        if use_stdin {
            if let Some(mut stdin) = child.stdin.take() {
                debug!("Writing {} bytes to stdin...", prompt.len());
                use std::io::Write;
                stdin.write_all(prompt.as_bytes())?;
            }
        }

        // Wait for output
        debug!("Waiting for output...");
        let output = child.wait_with_output()?;
        debug!("Exit Status: {}", output.status);

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            warn!("CLI STDERR: {}", stderr);
            return Err(anyhow::anyhow!("CLI Error: {}", stderr));
        }

        let raw_output = String::from_utf8_lossy(&output.stdout).to_string();
        debug!("Raw Output: {:.200}...", raw_output); // truncated log

        // Attempt to extract JSON using Regex (Robustness)
        match Self::extract_json(&raw_output) {
            Some(json) => Ok(json),
            None => {
                warn!("Failed to extract JSON from output: {:.100}...", raw_output);
                Err(anyhow::anyhow!(
                    "No valid JSON found in CLI output: {}",
                    raw_output
                ))
            }
        }
    }

    /// Extract the first valid JSON object or array from a string
    fn extract_json(text: &str) -> Option<String> {
        // Try to find a JSON block enclosed in ```json ... ```
        if let Some(start) = text.find("```json") {
            // Search for closing backticks AFTER the opening tag
            let content_start = start + 7;
            if let Some(end_offset) = text[content_start..].find("```") {
                let end = content_start + end_offset;
                let json_str = &text[content_start..end].trim();
                return Some(json_str.to_string());
            }
        }

        // Fallback: simple brace matching (naive but often works for single objects)
        if let Some(start) = text.find('{') {
            if let Some(end) = text.rfind('}') {
                if end > start {
                    return Some(text[start..=end].to_string());
                }
            }
        }

        // If simple extraction fails, return raw and let caller try to parse (or fail)
        // But for this function, we return Option
        Some(text.to_string())
    }

    /// Execute with base64 image for vision tasks
    pub fn execute_with_vision(&self, base64_image: &str, prompt: &str) -> Result<String> {
        // For vision tasks, we embed the image in the prompt
        // Most CLI tools support markdown-style image embedding
        let full_prompt = format!(
            "I'm showing you a screenshot (base64 encoded below). {}\n\n[Screenshot data: {} bytes]\n\nBase64 Image:\n{}",
            prompt,
            base64_image.len(),
            base64_image
        );

        self.execute(&full_prompt)
    }

    #[allow(dead_code)]
    fn build_command(&self) -> (String, Vec<String>) {
        match self.provider {
            LLMProvider::Codex => (
                "codex".to_string(),
                vec![
                    "exec".to_string(),
                    "--sandbox".to_string(),
                    "danger-full-access".to_string(),
                    "--skip-git-repo-check".to_string(),
                    "--color".to_string(),
                    "never".to_string(),
                    "-".to_string(),
                ],
            ),
            LLMProvider::Gemini => ("gemini".to_string(), vec!["-s".to_string()]),
            LLMProvider::Claude => (
                "claude".to_string(),
                vec![
                    "--dangerously-skip-permissions".to_string(),
                    "-p".to_string(),
                    "-".to_string(),
                ],
            ),
        }
    }
}

/// Convenience function for quick execution
pub fn execute_cli_llm(prompt: &str) -> Result<String> {
    let client = CLILLMClient::from_env().ok_or_else(|| {
        anyhow!("STEER_CLI_LLM not set. Use: export STEER_CLI_LLM=gemini|codex|claude")
    })?;
    client.execute(prompt)
}

/// Check if CLI LLM is configured and available
pub fn is_cli_llm_available() -> bool {
    CLILLMClient::from_env()
        .map(|c| c.check_version().is_ok())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_from_str() {
        assert_eq!(LLMProvider::from_str("gemini"), Some(LLMProvider::Gemini));
        assert_eq!(LLMProvider::from_str("CODEX"), Some(LLMProvider::Codex));
        assert_eq!(LLMProvider::from_str("Claude"), Some(LLMProvider::Claude));
        assert_eq!(LLMProvider::from_str("unknown"), None);
    }

    #[test]
    fn test_build_command() {
        let client = CLILLMClient::new(LLMProvider::Gemini);
        let (cmd, args) = client.build_command();
        assert_eq!(cmd, "gemini");
        assert!(args.contains(&"-s".to_string()));
    }
}
