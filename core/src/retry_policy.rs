use reqwest::StatusCode;
use serde_json::Value;

#[derive(Debug, Clone, Copy)]
pub struct RetryPolicy {
    pub attempts: u32,
    pub min_delay_ms: u64,
    pub max_delay_ms: u64,
    /// 0.0 ~ 0.5 권장
    pub jitter_ratio: f64,
}

impl RetryPolicy {
    pub fn new(attempts: u32, min_delay_ms: u64, max_delay_ms: u64, jitter_ratio: f64) -> Self {
        let attempts = attempts.clamp(1, 16);
        let min_delay_ms = min_delay_ms.clamp(50, 120_000);
        let max_delay_ms = max_delay_ms.max(min_delay_ms).clamp(min_delay_ms, 180_000);
        let jitter_ratio = if jitter_ratio.is_finite() {
            jitter_ratio.clamp(0.0, 0.5)
        } else {
            0.0
        };
        Self {
            attempts,
            min_delay_ms,
            max_delay_ms,
            jitter_ratio,
        }
    }
}

pub fn retryable_http_status(status: StatusCode) -> bool {
    status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error()
}

pub fn parse_retry_after_ms(body: &str) -> Option<u64> {
    let parsed: Value = serde_json::from_str(body).ok()?;

    let seconds = parsed
        .get("parameters")
        .and_then(|p| p.get("retry_after"))
        .and_then(|v| v.as_u64())
        .or_else(|| parsed.get("retry_after").and_then(|v| v.as_u64()))
        .or_else(|| {
            parsed
                .get("error")
                .and_then(|e| e.get("parameters"))
                .and_then(|p| p.get("retry_after"))
                .and_then(|v| v.as_u64())
        })?;

    Some(seconds.max(1).saturating_mul(1_000))
}

fn stable_jitter_factor(seed: &str, attempt: u32, jitter_ratio: f64) -> f64 {
    if jitter_ratio <= 0.0 {
        return 1.0;
    }
    let mut hash: u64 = 1469598103934665603;
    for b in seed.as_bytes() {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(1099511628211);
    }
    hash ^= attempt as u64;
    hash = hash.wrapping_mul(1099511628211);

    let normalized = (hash as f64) / (u64::MAX as f64); // 0~1
    let span = 2.0 * jitter_ratio;
    (1.0 - jitter_ratio) + (normalized * span)
}

pub fn compute_backoff_delay_ms(
    policy: RetryPolicy,
    attempt_index_zero_based: u32,
    retry_after_ms: Option<u64>,
    seed: &str,
) -> u64 {
    let exp = 2_u64.saturating_pow(attempt_index_zero_based.min(16));
    let base = policy
        .min_delay_ms
        .saturating_mul(exp)
        .min(policy.max_delay_ms);
    let hinted = retry_after_ms.unwrap_or(0).min(policy.max_delay_ms);
    let picked = base.max(hinted);
    let jitter_factor = stable_jitter_factor(seed, attempt_index_zero_based, policy.jitter_ratio);
    ((picked as f64 * jitter_factor).round() as u64).clamp(policy.min_delay_ms, policy.max_delay_ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_retry_after_reads_common_shapes() {
        let body = r#"{"parameters":{"retry_after":2}}"#;
        assert_eq!(parse_retry_after_ms(body), Some(2000));
        let body2 = r#"{"retry_after":5}"#;
        assert_eq!(parse_retry_after_ms(body2), Some(5000));
    }

    #[test]
    fn compute_backoff_is_bounded() {
        let p = RetryPolicy::new(4, 400, 10_000, 0.1);
        let d = compute_backoff_delay_ms(p, 3, Some(90_000), "telegram");
        assert!((400..=10_000).contains(&d));
    }
}
