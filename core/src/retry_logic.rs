//! Retry Logic - Clawdbot-style robust execution
//!
//! Ported from: clawdbot-main/src/agents/pi-embedded-runner/run.ts
//!
//! Features:
//! - Multi-attempt with exponential backoff
//! - Context overflow detection
//! - Auth profile rotation (stub)

use anyhow::Result;
use std::time::Duration;
use tokio::time::sleep;

// =====================================================
// RETRY CONFIGURATION
// =====================================================

#[derive(Debug, Clone)]
pub struct RetryConfig {
    pub max_attempts: usize,
    pub base_delay_ms: u64,
    pub max_delay_ms: u64,
    pub backoff_multiplier: f64,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            base_delay_ms: 1000,
            max_delay_ms: 30000,
            backoff_multiplier: 2.0,
        }
    }
}

// =====================================================
// ERROR CLASSIFICATION
// =====================================================

#[derive(Debug, Clone, PartialEq)]
pub enum FailoverReason {
    RateLimit,
    Auth,
    Timeout,
    ContextOverflow,
    NetworkError,
    Unknown,
}

/// Classify an error message to determine retry strategy
pub fn classify_error(message: &str) -> FailoverReason {
    let msg_lower = message.to_lowercase();

    // Rate limit detection (from clawdbot)
    if msg_lower.contains("rate limit")
        || msg_lower.contains("429")
        || msg_lower.contains("too many requests")
        || msg_lower.contains("quota exceeded")
    {
        return FailoverReason::RateLimit;
    }

    // Auth errors
    if msg_lower.contains("unauthorized")
        || msg_lower.contains("401")
        || msg_lower.contains("invalid api key")
        || msg_lower.contains("authentication")
    {
        return FailoverReason::Auth;
    }

    // Timeout
    if msg_lower.contains("timeout")
        || msg_lower.contains("timed out")
        || msg_lower.contains("deadline exceeded")
    {
        return FailoverReason::Timeout;
    }

    // Context overflow
    if msg_lower.contains("context length")
        || msg_lower.contains("context window")
        || msg_lower.contains("maximum context")
        || msg_lower.contains("token limit")
        || msg_lower.contains("too long")
    {
        return FailoverReason::ContextOverflow;
    }

    // Network errors
    if msg_lower.contains("connection")
        || msg_lower.contains("network")
        || msg_lower.contains("dns")
        || msg_lower.contains("could not resolve")
    {
        return FailoverReason::NetworkError;
    }

    FailoverReason::Unknown
}

/// Determine if an error is retryable
pub fn is_retryable(reason: &FailoverReason) -> bool {
    match reason {
        FailoverReason::RateLimit => true,        // Retry after backoff
        FailoverReason::Timeout => true,          // Retry immediately
        FailoverReason::NetworkError => true,     // Retry with delay
        FailoverReason::Auth => false,            // Don't retry (need new key)
        FailoverReason::ContextOverflow => false, // Need compaction
        FailoverReason::Unknown => true,          // Try once more
    }
}

/// Calculate delay based on attempt number
pub fn calculate_delay(config: &RetryConfig, attempt: usize) -> Duration {
    let delay = config.base_delay_ms as f64 * config.backoff_multiplier.powi(attempt as i32);
    let clamped = delay.min(config.max_delay_ms as f64) as u64;
    Duration::from_millis(clamped)
}

// =====================================================
// RETRY EXECUTOR
// =====================================================

/// Execute a future with retry logic
pub async fn with_retry<F, Fut, T>(
    config: &RetryConfig,
    operation_name: &str,
    mut operation: F,
) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    let mut last_error = None;

    for attempt in 0..config.max_attempts {
        match operation().await {
            Ok(result) => {
                if attempt > 0 {
                    println!(
                        "✅ [Retry] {} succeeded on attempt {}",
                        operation_name,
                        attempt + 1
                    );
                }
                return Ok(result);
            }
            Err(e) => {
                let error_msg = e.to_string();
                let reason = classify_error(&error_msg);

                println!(
                    "⚠️ [Retry] {} failed (attempt {}/{}): {:?} - {}",
                    operation_name,
                    attempt + 1,
                    config.max_attempts,
                    reason,
                    &error_msg[..error_msg.len().min(100)]
                );

                if !is_retryable(&reason) || attempt + 1 >= config.max_attempts {
                    last_error = Some(e);
                    break;
                }

                // Apply backoff
                let delay = calculate_delay(config, attempt);
                println!("   ⏳ Waiting {}ms before retry...", delay.as_millis());
                sleep(delay).await;

                last_error = Some(e);
            }
        }
    }

    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("Retry failed without error")))
}

// =====================================================
// CONTEXT WINDOW GUARD
// =====================================================

#[derive(Debug, Clone)]
pub struct ContextWindowInfo {
    pub max_tokens: usize,
    pub current_tokens: usize,
    pub warn_threshold: usize,
    pub hard_limit: usize,
}

impl ContextWindowInfo {
    pub fn new(max_tokens: usize) -> Self {
        Self {
            max_tokens,
            current_tokens: 0,
            warn_threshold: (max_tokens as f64 * 0.8) as usize,
            hard_limit: (max_tokens as f64 * 0.95) as usize,
        }
    }

    /// Estimate tokens from text (rough: 1 token ≈ 4 chars)
    pub fn estimate_tokens(text: &str) -> usize {
        text.len() / 4
    }

    /// Update current token count
    pub fn update(&mut self, messages: &[String]) {
        self.current_tokens = messages.iter().map(|m| Self::estimate_tokens(m)).sum();
    }

    /// Check if we're approaching the limit
    pub fn should_warn(&self) -> bool {
        self.current_tokens >= self.warn_threshold
    }

    /// Check if we've exceeded the limit
    pub fn should_compact(&self) -> bool {
        self.current_tokens >= self.hard_limit
    }

    /// Get remaining capacity
    pub fn remaining(&self) -> usize {
        self.max_tokens.saturating_sub(self.current_tokens)
    }
}

// =====================================================
// HISTORY COMPACTION
// =====================================================

/// Compact conversation history by summarizing old messages
pub fn compact_history(messages: &[String], keep_recent: usize) -> Vec<String> {
    if messages.len() <= keep_recent + 1 {
        return messages.to_vec();
    }

    let split_point = messages.len() - keep_recent;
    let old_messages = &messages[..split_point];
    let recent_messages = &messages[split_point..];

    // Create a summary of old messages
    let summary = format!(
        "[Compacted History: {} previous actions including: {}]",
        old_messages.len(),
        old_messages
            .iter()
            .take(3)
            .cloned()
            .collect::<Vec<_>>()
            .join(", ")
    );

    let mut result = vec![summary];
    result.extend(recent_messages.iter().cloned());
    result
}

// =====================================================
// TESTS
// =====================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classify_error() {
        assert_eq!(
            classify_error("Rate limit exceeded"),
            FailoverReason::RateLimit
        );
        assert_eq!(
            classify_error("HTTP 429 Too Many Requests"),
            FailoverReason::RateLimit
        );
        assert_eq!(classify_error("Invalid API key"), FailoverReason::Auth);
        assert_eq!(
            classify_error("Connection timeout"),
            FailoverReason::Timeout
        );
        assert_eq!(
            classify_error("Context length exceeded"),
            FailoverReason::ContextOverflow
        );
        assert_eq!(classify_error("Random error"), FailoverReason::Unknown);
    }

    #[test]
    fn test_calculate_delay() {
        let config = RetryConfig::default();
        assert_eq!(calculate_delay(&config, 0), Duration::from_millis(1000));
        assert_eq!(calculate_delay(&config, 1), Duration::from_millis(2000));
        assert_eq!(calculate_delay(&config, 2), Duration::from_millis(4000));
    }

    #[test]
    fn test_context_window() {
        let mut ctx = ContextWindowInfo::new(4096);
        assert!(!ctx.should_warn());

        ctx.current_tokens = 3500;
        assert!(ctx.should_warn());

        ctx.current_tokens = 4000;
        assert!(ctx.should_compact());
    }

    #[test]
    fn test_compact_history() {
        let history = vec![
            "Step 1".to_string(),
            "Step 2".to_string(),
            "Step 3".to_string(),
            "Step 4".to_string(),
            "Step 5".to_string(),
        ];

        let compacted = compact_history(&history, 2);
        assert_eq!(compacted.len(), 3); // 1 summary + 2 recent
        assert!(compacted[0].contains("Compacted History"));
        assert_eq!(compacted[1], "Step 4");
        assert_eq!(compacted[2], "Step 5");
    }
}
