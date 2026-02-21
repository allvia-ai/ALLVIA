//! Local llama-server management for Steer Agent
//!
//! Auto-starts llama-server with Gemma3 GGUF model and provides
//! OpenAI-compatible chat completion via localhost.

use anyhow::Result;
use reqwest::Client;
use serde_json::{json, Value};
use std::env;
use std::process::Command;
use std::time::Duration;
use tokio::time::sleep;

/// Default port for local llama-server
const DEFAULT_PORT: u16 = 8090;

/// Check if llama-server is already running on the configured port
pub fn is_running() -> bool {
    let port = get_port();
    std::net::TcpStream::connect(format!("127.0.0.1:{}", port)).is_ok()
}

/// Get the configured port
fn get_port() -> u16 {
    env::var("STEER_LLAMA_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_PORT)
}

/// Get the llama-server binary path
fn get_server_path() -> String {
    env::var("STEER_LLAMA_SERVER").unwrap_or_else(|_| {
        let home = env::var("HOME").unwrap_or_else(|_| "/Users/david".to_string());
        format!(
            "{}/Desktop/python/github/AI-summary/llama_build/llama.cpp/build/bin/llama-server",
            home
        )
    })
}

/// Get the GGUF model path
fn get_model_path() -> String {
    env::var("STEER_LLAMA_MODEL").unwrap_or_else(|_| {
        let home = env::var("HOME").unwrap_or_else(|_| "/Users/david".to_string());
        format!(
            "{}/Desktop/python/github/AI-summary/models/gguf/gemma-3-4b-it-Q4_K_M.gguf",
            home
        )
    })
}

/// Start llama-server in the background if not already running.
/// Returns true if server is ready, false otherwise.
pub async fn ensure_running() -> bool {
    if is_running() {
        return true;
    }

    let server_path = get_server_path();
    let model_path = get_model_path();
    let port = get_port();

    // Verify binary and model exist
    if !std::path::Path::new(&server_path).exists() {
        eprintln!(
            "⚠️ [llama_local] llama-server not found at: {}",
            server_path
        );
        return false;
    }
    if !std::path::Path::new(&model_path).exists() {
        eprintln!("⚠️ [llama_local] GGUF model not found at: {}", model_path);
        return false;
    }

    eprintln!(
        "🦙 [llama_local] Starting llama-server on port {} with {}...",
        port,
        model_path.split('/').last().unwrap_or(&model_path)
    );

    // GPU layers: -1 = offload all layers to Metal
    let gpu_layers = env::var("STEER_LLAMA_GPU_LAYERS")
        .ok()
        .and_then(|s| s.parse::<i32>().ok())
        .unwrap_or(-1);

    let result = Command::new(&server_path)
        .args([
            "-m",
            &model_path,
            "--port",
            &port.to_string(),
            "-ngl",
            &gpu_layers.to_string(),
            "-c",
            "4096",
            "--host",
            "127.0.0.1",
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();

    match result {
        Ok(_child) => {
            // Wait for server to be ready (max 15 seconds)
            for i in 0..30 {
                sleep(Duration::from_millis(500)).await;
                if is_running() {
                    eprintln!(
                        "✅ [llama_local] llama-server ready after {:.1}s",
                        (i + 1) as f64 * 0.5
                    );
                    return true;
                }
            }
            eprintln!("❌ [llama_local] llama-server failed to start within 15s");
            false
        }
        Err(e) => {
            eprintln!("❌ [llama_local] Failed to spawn llama-server: {}", e);
            false
        }
    }
}

/// Call the local llama-server using OpenAI-compatible chat completion API.
/// Returns the assistant message content.
pub async fn chat_completion(messages: &[Value]) -> Result<String> {
    let port = get_port();
    let url = format!("http://127.0.0.1:{}/v1/chat/completions", port);

    let body = json!({
        "model": "local",
        "messages": messages,
        "temperature": 0.3,
        "max_tokens": 2048
    });

    let client = Client::builder()
        .timeout(Duration::from_secs(120))
        .build()?;

    let res = client
        .post(&url)
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await?;

    if !res.status().is_success() {
        let err_text = res.text().await.unwrap_or_default();
        return Err(anyhow::anyhow!(
            "llama-server API error ({}): {}",
            port,
            err_text
        ));
    }

    let res_json: Value = res.json().await?;
    let content = res_json["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("")
        .to_string();

    if content.is_empty() {
        return Err(anyhow::anyhow!("llama-server returned empty response"));
    }

    Ok(content)
}
