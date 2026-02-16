use anyhow::Result;
use reqwest::Client;
use std::time::Duration;
use tokio::time::sleep;

pub const TEXT_CHUNK_LIMIT: usize = 3900;
pub const DEFAULT_MAX_SEND_ATTEMPTS: u32 = 4;

fn bool_env_with_default(key: &str, default: bool) -> bool {
    std::env::var(key)
        .ok()
        .map(|v| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(default)
}

pub fn split_message_chunks(message: &str, max_len: usize) -> Vec<String> {
    if message.trim().is_empty() {
        return Vec::new();
    }
    let mut chunks: Vec<String> = Vec::new();
    let mut current = String::new();

    for line in message.lines() {
        let line = line.trim_end();
        let candidate = if current.is_empty() {
            line.to_string()
        } else {
            format!("{}\n{}", current, line)
        };
        if candidate.chars().count() <= max_len {
            current = candidate;
            continue;
        }

        if !current.is_empty() {
            chunks.push(current);
            current = String::new();
        }

        if line.chars().count() <= max_len {
            current = line.to_string();
            continue;
        }

        let mut segment = String::new();
        for ch in line.chars() {
            segment.push(ch);
            if segment.chars().count() >= max_len {
                chunks.push(segment);
                segment = String::new();
            }
        }
        if !segment.is_empty() {
            current = segment;
        }
    }

    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

fn parse_u32_env(name: &str, default: u32, min: u32, max: u32) -> u32 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.trim().parse::<u32>().ok())
        .map(|v| v.clamp(min, max))
        .unwrap_or(default)
}

fn parse_u64_env(name: &str, default: u64, min: u64, max: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .map(|v| v.clamp(min, max))
        .unwrap_or(default)
}

fn parse_f64_env(name: &str, default: f64, min: f64, max: f64) -> f64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.trim().parse::<f64>().ok())
        .map(|v| v.clamp(min, max))
        .unwrap_or(default)
}

fn telegram_retry_policy(max_send_attempts: u32) -> crate::retry_policy::RetryPolicy {
    let attempts = parse_u32_env(
        "STEER_TELEGRAM_RETRY_ATTEMPTS",
        max_send_attempts.max(1),
        1,
        12,
    );
    let min_delay_ms = parse_u64_env("STEER_TELEGRAM_RETRY_MIN_DELAY_MS", 400, 100, 120_000);
    let max_delay_ms = parse_u64_env("STEER_TELEGRAM_RETRY_MAX_DELAY_MS", 30_000, 500, 180_000);
    let jitter = parse_f64_env("STEER_TELEGRAM_RETRY_JITTER", 0.1, 0.0, 0.5);
    crate::retry_policy::RetryPolicy::new(attempts, min_delay_ms, max_delay_ms, jitter)
}

async fn send_chunk_with_retry(
    client: &Client,
    token: &str,
    chat_id: &str,
    chunk: &str,
    parse_mode: Option<&str>,
    max_send_attempts: u32,
) -> Result<()> {
    let url = format!("https://api.telegram.org/bot{}/sendMessage", token);
    let policy = telegram_retry_policy(max_send_attempts);

    for attempt in 1..=policy.attempts {
        let mut params = vec![("chat_id", chat_id), ("text", chunk)];
        if let Some(mode) = parse_mode {
            params.push(("parse_mode", mode));
        }

        match client.post(&url).form(&params).send().await {
            Ok(resp) => {
                if resp.status().is_success() {
                    crate::diagnostic_events::emit(
                        "telegram.send.ok",
                        serde_json::json!({
                            "attempt": attempt,
                            "chat_id": chat_id,
                            "size": chunk.chars().count()
                        }),
                    );
                    return Ok(());
                }
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                let retryable = crate::retry_policy::retryable_http_status(status);
                if retryable && attempt < policy.attempts {
                    let retry_after_ms = crate::retry_policy::parse_retry_after_ms(&body);
                    let delay_ms = crate::retry_policy::compute_backoff_delay_ms(
                        policy,
                        attempt - 1,
                        retry_after_ms,
                        "telegram.send",
                    );
                    crate::diagnostic_events::emit(
                        "telegram.send.retry",
                        serde_json::json!({
                            "attempt": attempt,
                            "status": status.as_u16(),
                            "delay_ms": delay_ms,
                            "retry_after_ms": retry_after_ms
                        }),
                    );
                    sleep(Duration::from_millis(delay_ms)).await;
                    continue;
                }
                crate::diagnostic_events::emit(
                    "telegram.send.error",
                    serde_json::json!({
                        "attempt": attempt,
                        "status": status.as_u16(),
                        "body": body
                    }),
                );
                return Err(anyhow::anyhow!(
                    "Telegram API Error (status={}): {}",
                    status,
                    body
                ));
            }
            Err(e) => {
                if attempt < policy.attempts {
                    let delay_ms = crate::retry_policy::compute_backoff_delay_ms(
                        policy,
                        attempt - 1,
                        None,
                        "telegram.network",
                    );
                    crate::diagnostic_events::emit(
                        "telegram.send.retry",
                        serde_json::json!({
                            "attempt": attempt,
                            "error": e.to_string(),
                            "delay_ms": delay_ms
                        }),
                    );
                    sleep(Duration::from_millis(delay_ms)).await;
                    continue;
                }
                crate::diagnostic_events::emit(
                    "telegram.send.error",
                    serde_json::json!({
                        "attempt": attempt,
                        "error": e.to_string()
                    }),
                );
                return Err(anyhow::anyhow!("Telegram request failed: {}", e));
            }
        }
    }

    Err(anyhow::anyhow!("Telegram send failed after retries"))
}

pub async fn send_message_chunked(
    client: &Client,
    token: &str,
    chat_id: &str,
    message: &str,
    parse_mode: Option<&str>,
    max_send_attempts: u32,
) -> Result<()> {
    if let Err(policy_err) = crate::outbound_policy::enforce_telegram_send_policy(chat_id, message) {
        crate::diagnostic_events::emit(
            "telegram.send.blocked",
            serde_json::json!({
                "reason": "outbound_policy",
                "chat_id": chat_id,
                "error": policy_err
            }),
        );
        if bool_env_with_default("STEER_TELEGRAM_REQUIRE_SEND", false) {
            return Err(anyhow::anyhow!(
                "telegram send blocked by outbound policy: {}",
                policy_err
            ));
        }
        return Ok(());
    }

    let ctx = crate::send_policy::SendPolicyContext {
        session_key: Some(format!("telegram_chat_{}", chat_id)),
        channel: Some("telegram".to_string()),
        chat_type: None,
        target_id: Some(chat_id.to_string()),
    };
    if matches!(
        crate::send_policy::should_send_with_context("telegram", message, Some(&ctx)),
        crate::send_policy::SendDecision::Deny
    ) {
        crate::diagnostic_events::emit(
            "telegram.send.blocked",
            serde_json::json!({
                "reason": "send_policy",
                "chat_id": chat_id
            }),
        );
        if bool_env_with_default("STEER_TELEGRAM_REQUIRE_SEND", false) {
            return Err(anyhow::anyhow!(
                "telegram send blocked by send policy (chat_id={})",
                chat_id
            ));
        }
        return Ok(());
    }

    let chunks = split_message_chunks(message, TEXT_CHUNK_LIMIT);
    if chunks.is_empty() {
        return Ok(());
    }
    for chunk in chunks {
        send_chunk_with_retry(
            client,
            token,
            chat_id,
            &chunk,
            parse_mode,
            max_send_attempts.max(1),
        )
        .await?;
    }
    Ok(())
}
