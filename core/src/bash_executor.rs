//! Advanced Bash Executor - Clawdbot-style shell execution
//!
//! Ported from: clawdbot-main/src/agents/bash-tools.exec.ts
//!
//! Features:
//! - PTY-like interactive shell support
//! - Background process tracking
//! - Approval workflow integration
//! - Process registry for long-running commands

use anyhow::{Context, Result};
use std::collections::HashMap;
use std::io::Write;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

// =====================================================
// PROCESS REGISTRY (from bash-process-registry.ts)
// =====================================================

#[derive(Debug, Clone)]
pub struct ProcessInfo {
    pub id: String,
    pub pid: u32,
    pub command: String,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub status: ProcessStatus,
    pub output: Vec<String>,
    pub exit_code: Option<i32>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ProcessStatus {
    Running,
    Completed,
    Failed,
    Killed,
}

lazy_static::lazy_static! {
    static ref PROCESS_REGISTRY: Mutex<HashMap<String, ProcessInfo>> = Mutex::new(HashMap::new());
}

/// Register a new background process
pub fn register_process(id: &str, pid: u32, command: &str) {
    let info = ProcessInfo {
        id: id.to_string(),
        pid,
        command: command.to_string(),
        started_at: chrono::Utc::now(),
        status: ProcessStatus::Running,
        output: Vec::new(),
        exit_code: None,
    };

    if let Ok(mut registry) = PROCESS_REGISTRY.lock() {
        registry.insert(id.to_string(), info);
        println!("📋 [Process] Registered: {} (PID {})", id, pid);
    }
}

/// Update process status
pub fn update_process(
    id: &str,
    status: ProcessStatus,
    exit_code: Option<i32>,
    output: Option<Vec<String>>,
) {
    if let Ok(mut registry) = PROCESS_REGISTRY.lock() {
        if let Some(info) = registry.get_mut(id) {
            info.status = status;
            info.exit_code = exit_code;
            if let Some(lines) = output {
                info.output = lines;
            }
        }
    }
}

/// Get process info
pub fn get_process(id: &str) -> Option<ProcessInfo> {
    PROCESS_REGISTRY.lock().ok()?.get(id).cloned()
}

/// List all active processes
pub fn list_active_processes() -> Vec<ProcessInfo> {
    PROCESS_REGISTRY
        .lock()
        .ok()
        .map(|r| {
            r.values()
                .filter(|p| p.status == ProcessStatus::Running)
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

/// Kill a process by ID
pub fn kill_process(id: &str) -> Result<bool> {
    let info = get_process(id).ok_or_else(|| anyhow::anyhow!("Process not found"))?;

    // Send SIGTERM
    let kill_result = Command::new("kill")
        .args(["-15", &info.pid.to_string()])
        .status()?;

    if kill_result.success() {
        update_process(id, ProcessStatus::Killed, None, None);
        println!("🔪 [Process] Killed: {} (PID {})", id, info.pid);
        Ok(true)
    } else {
        Ok(false)
    }
}

// =====================================================
// ADVANCED BASH EXECUTION
// =====================================================

#[derive(Debug, Clone)]
pub struct BashExecConfig {
    pub timeout_ms: u64,
    pub working_dir: Option<String>,
    pub env_vars: HashMap<String, String>,
    pub background: bool,
    pub approval_required: bool,
}

impl Default for BashExecConfig {
    fn default() -> Self {
        Self {
            timeout_ms: 30000,
            working_dir: None,
            env_vars: HashMap::new(),
            background: false,
            approval_required: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct BashExecResult {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    pub duration_ms: u64,
    pub process_id: Option<String>,
}

/// Execute a bash command with advanced options
pub fn execute_bash(cmd: &str, config: &BashExecConfig) -> Result<BashExecResult> {
    use crate::approval_gate::{ApprovalGate, ApprovalLevel};

    let start = Instant::now();

    // Check approval
    if config.approval_required {
        match ApprovalGate::check_command(cmd) {
            ApprovalLevel::Blocked => {
                return Ok(BashExecResult {
                    success: false,
                    stdout: String::new(),
                    stderr: "Command blocked by security policy".to_string(),
                    exit_code: -1,
                    duration_ms: 0,
                    process_id: None,
                });
            }
            ApprovalLevel::RequireApproval => {
                if crate::env_flag("STEER_BASH_ALLOW_AUTO_APPROVAL") {
                    println!(
                        "⚠️ [Bash] Command requires approval but auto-approved by STEER_BASH_ALLOW_AUTO_APPROVAL=1: {}",
                        cmd
                    );
                } else {
                    return Ok(BashExecResult {
                        success: false,
                        stdout: String::new(),
                        stderr: "Command requires explicit approval".to_string(),
                        exit_code: -2,
                        duration_ms: 0,
                        process_id: None,
                    });
                }
            }
            ApprovalLevel::AutoApprove => {}
        }
    }

    // Background execution
    if config.background {
        return execute_background(cmd, config);
    }

    // Foreground execution with timeout
    let mut command = Command::new("/bin/bash");
    command.arg("-c").arg(cmd);

    if let Some(dir) = &config.working_dir {
        command.current_dir(dir);
    }

    for (key, value) in &config.env_vars {
        command.env(key, value);
    }

    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());

    let child = command
        .spawn()
        .context(format!("Failed to spawn command: {}", cmd))?;

    // Wait with timeout
    let output = wait_with_timeout(child, Duration::from_millis(config.timeout_ms))
        .context("Command execution failed")?;

    let duration = start.elapsed().as_millis() as u64;

    Ok(BashExecResult {
        success: output.0,
        stdout: output.1,
        stderr: output.2,
        exit_code: output.3,
        duration_ms: duration,
        process_id: None,
    })
}

/// Execute command in background
fn execute_background(cmd: &str, config: &BashExecConfig) -> Result<BashExecResult> {
    let mut command = Command::new("/bin/bash");
    command.arg("-c").arg(cmd);

    if let Some(dir) = &config.working_dir {
        command.current_dir(dir);
    }

    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());

    let child = command
        .spawn()
        .context(format!("Failed to spawn background command: {}", cmd))?;

    let pid = child.id();
    let process_id = format!("bg_{}", uuid::Uuid::new_v4().to_string()[..8].to_string());

    register_process(&process_id, pid, cmd);

    // Spawn a thread to wait for completion
    let proc_id = process_id.clone();
    std::thread::spawn(move || {
        let output = child.wait_with_output();
        match output {
            Ok(out) => {
                let status = if out.status.success() {
                    ProcessStatus::Completed
                } else {
                    ProcessStatus::Failed
                };
                let stdout_lines: Vec<String> = String::from_utf8_lossy(&out.stdout)
                    .lines()
                    .map(|s| s.to_string())
                    .collect();
                update_process(&proc_id, status, out.status.code(), Some(stdout_lines));
            }
            Err(_) => {
                update_process(&proc_id, ProcessStatus::Failed, Some(-1), None);
            }
        }
    });

    Ok(BashExecResult {
        success: true,
        stdout: format!("Background process started: {}", process_id),
        stderr: String::new(),
        exit_code: 0,
        duration_ms: 0,
        process_id: Some(process_id),
    })
}

/// Wait for child process with timeout
fn wait_with_timeout(child: Child, timeout: Duration) -> Result<(bool, String, String, i32)> {
    use std::sync::mpsc;
    use std::thread;

    let (tx, rx) = mpsc::channel();

    thread::spawn(move || {
        let result = child.wait_with_output();
        let _ = tx.send(result);
    });

    match rx.recv_timeout(timeout) {
        Ok(result) => {
            let output = result?;
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            let exit_code = output.status.code().unwrap_or(-1);
            Ok((output.status.success(), stdout, stderr, exit_code))
        }
        Err(_) => {
            // Timeout - try to kill the process (but child is moved, so we can't)
            Err(anyhow::anyhow!(
                "Command timed out after {}ms",
                timeout.as_millis()
            ))
        }
    }
}

// =====================================================
// PTY-LIKE INTERACTIVE SESSION
// =====================================================

/// Interactive shell session for commands that need input
pub struct InteractiveSession {
    child: Option<Child>,
    pub session_id: String,
}

impl InteractiveSession {
    /// Start a new interactive shell session
    pub fn new() -> Result<Self> {
        let child = Command::new("/bin/bash")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("Failed to start interactive bash session")?;

        let session_id = format!("pty_{}", uuid::Uuid::new_v4().to_string()[..8].to_string());

        println!("🖥️ [PTY] Started interactive session: {}", session_id);

        Ok(Self {
            child: Some(child),
            session_id,
        })
    }

    /// Send a command to the interactive session
    pub fn send(&mut self, input: &str) -> Result<String> {
        let child = self
            .child
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("Session not running"))?;

        let stdin = child
            .stdin
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("No stdin available"))?;

        writeln!(stdin, "{}", input)?;
        stdin.flush()?;

        // Read some output (non-blocking would be better but this is a simple version)
        std::thread::sleep(Duration::from_millis(100));

        Ok(format!("Sent: {}", input))
    }

    /// Close the interactive session
    pub fn close(&mut self) -> Result<()> {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            println!("🖥️ [PTY] Closed session: {}", self.session_id);
        }
        Ok(())
    }
}

impl Drop for InteractiveSession {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

// =====================================================
// CONVENIENCE FUNCTIONS
// =====================================================

/// Simple synchronous command execution
pub fn exec(cmd: &str) -> Result<String> {
    let config = BashExecConfig {
        approval_required: true,
        ..Default::default()
    };

    let result = execute_bash(cmd, &config)?;

    if result.success {
        Ok(result.stdout)
    } else {
        Err(anyhow::anyhow!("Command failed: {}", result.stderr))
    }
}

/// Execute with custom timeout
pub fn exec_timeout(cmd: &str, timeout_ms: u64) -> Result<String> {
    let config = BashExecConfig {
        timeout_ms,
        approval_required: true,
        ..Default::default()
    };

    let result = execute_bash(cmd, &config)?;

    if result.success {
        Ok(result.stdout)
    } else {
        Err(anyhow::anyhow!("Command failed: {}", result.stderr))
    }
}

/// Execute in background
pub fn exec_background(cmd: &str) -> Result<String> {
    let config = BashExecConfig {
        background: true,
        approval_required: true,
        ..Default::default()
    };

    let result = execute_bash(cmd, &config)?;

    if let Some(id) = result.process_id {
        Ok(format!("Background process started: {}", id))
    } else {
        Ok(result.stdout)
    }
}

// =====================================================
// TESTS
// =====================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_exec() {
        let result = execute_bash("echo hello", &BashExecConfig::default());
        assert!(result.is_ok());
        let res = result.unwrap();
        assert!(res.success);
        assert!(res.stdout.contains("hello"));
    }

    #[test]
    fn test_process_registry() {
        register_process("test1", 12345, "sleep 100");
        let info = get_process("test1");
        assert!(info.is_some());
        assert_eq!(info.unwrap().pid, 12345);
    }
}
