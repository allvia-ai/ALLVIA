use anyhow::Result;
use reqwest::Client;
use serde_json::Value;
use std::time::Duration;
use tokio::time::sleep;

pub const TEXT_CHUNK_LIMIT: usize = 3900;
pub const DEFAULT_MAX_SEND_ATTEMPTS: u32 = 4;

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

fn parse_retry_after_seconds(body: &str) -> Option<u64> {
    let parsed: Value = serde_json::from_str(body).ok()?;
    if let Some(v) = parsed
        .get("parameters")
        .and_then(|p| p.get("retry_after"))
        .and_then(|v| v.as_u64())
    {
        return Some(v.max(1));
    }
    parsed
        .get("retry_after")
        .and_then(|v| v.as_u64())
        .map(|v| v.max(1))
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
    let mut backoff_secs: u64 = 1;

    for attempt in 1..=max_send_attempts {
        let mut params = vec![("chat_id", chat_id), ("text", chunk)];
        if let Some(mode) = parse_mode {
            params.push(("parse_mode", mode));
        }

        match client.post(&url).form(&params).send().await {
            Ok(resp) => {
                if resp.status().is_success() {
                    return Ok(());
                }
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                let retryable = status.as_u16() == 429 || status.is_server_error();
                if retryable && attempt < max_send_attempts {
                    let retry_after = parse_retry_after_seconds(&body)
                        .unwrap_or(backoff_secs)
                        .min(30);
                    sleep(Duration::from_secs(retry_after)).await;
                    backoff_secs = (backoff_secs * 2).min(30);
                    continue;
                }
                return Err(anyhow::anyhow!(
                    "Telegram API Error (status={}): {}",
                    status,
                    body
                ));
            }
            Err(e) => {
                if attempt < max_send_attempts {
                    sleep(Duration::from_secs(backoff_secs.min(30))).await;
                    backoff_secs = (backoff_secs * 2).min(30);
                    continue;
                }
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
