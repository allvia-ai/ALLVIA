#![allow(dead_code)]
use crate::db;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Feedback data for workflow execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionFeedback {
    pub recommendation_id: i64,
    pub workflow_id: String,
    pub success: bool,
    pub error_category: Option<ErrorCategory>,
    pub error_message: Option<String>,
    pub execution_time_ms: u64,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ErrorCategory {
    ApiError,     // n8n API 오류
    AuthError,    // 인증 문제
    SchemaError,  // 잘못된 워크플로우 스키마
    TriggerError, // 트리거 설정 오류
    ActionError,  // 액션 실행 오류
    Unknown,
}

impl ErrorCategory {
    pub fn from_error(error: &str) -> Self {
        let lower = error.to_lowercase();
        if lower.contains("auth") || lower.contains("permission") || lower.contains("unauthorized")
        {
            Self::AuthError
        } else if lower.contains("schema") || lower.contains("invalid") || lower.contains("json") {
            Self::SchemaError
        } else if lower.contains("trigger") {
            Self::TriggerError
        } else if lower.contains("api") || lower.contains("network") || lower.contains("connection")
        {
            Self::ApiError
        } else {
            Self::Unknown
        }
    }
}

/// Feedback collector for improving recommendation quality
pub struct FeedbackCollector {
    suppressed_patterns: Vec<String>, // 억제된 패턴 ID 목록
}

impl FeedbackCollector {
    pub fn new() -> Self {
        Self {
            suppressed_patterns: Vec::new(),
        }
    }

    /// Record successful execution
    pub fn record_success(
        &mut self,
        recommendation_id: i64,
        workflow_id: &str,
        execution_time_ms: u64,
    ) {
        let feedback = ExecutionFeedback {
            recommendation_id,
            workflow_id: workflow_id.to_string(),
            success: true,
            error_category: None,
            error_message: None,
            execution_time_ms,
            created_at: Utc::now(),
        };
        self.save_feedback(&feedback);
        println!("📈 Feedback recorded: Success for workflow {}", workflow_id);
    }

    /// Record failed execution
    pub fn record_failure(&mut self, recommendation_id: i64, workflow_id: &str, error: &str) {
        let category = ErrorCategory::from_error(error);
        let feedback = ExecutionFeedback {
            recommendation_id,
            workflow_id: workflow_id.to_string(),
            success: false,
            error_category: Some(category),
            error_message: Some(error.to_string()),
            execution_time_ms: 0,
            created_at: Utc::now(),
        };
        self.save_feedback(&feedback);

        // Check if pattern should be suppressed
        self.check_suppression(recommendation_id);

        println!(
            "📉 Feedback recorded: Failure for workflow {} - {}",
            workflow_id, error
        );
    }

    /// Save feedback to database
    fn save_feedback(&self, feedback: &ExecutionFeedback) {
        let json = serde_json::to_string(feedback).unwrap_or_default();
        let event = format!(
            r#"{{"type":"workflow_feedback","source":"feedback_collector","data":{}}}"#,
            json
        );
        let _ = db::insert_event(&event);
    }

    /// Check if a pattern should be suppressed due to repeated failures
    fn check_suppression(&mut self, recommendation_id: i64) {
        // Count recent failures for this recommendation
        // If 3+ consecutive failures, suppress similar patterns
        let events = db::get_recent_events(24 * 7).unwrap_or_default();
        let mut fail_count = 0;

        for event in events.iter().rev() {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(event) {
                if val["type"].as_str() == Some("workflow_feedback") {
                    if let Some(data) = val["data"].as_object() {
                        if data.get("recommendation_id").and_then(|v| v.as_i64())
                            == Some(recommendation_id)
                        {
                            if data.get("success").and_then(|v| v.as_bool()) == Some(false) {
                                fail_count += 1;
                            } else {
                                break; // Reset on success
                            }
                        }
                    }
                }
            }
        }

        if fail_count >= 3 {
            // Mark recommendation as failed in DB
            let _ = db::mark_recommendation_failed(
                recommendation_id,
                "suppressed due to repeated execution feedback failures",
            );
            println!(
                "⚠️  Recommendation {} suppressed due to repeated failures",
                recommendation_id
            );
        }
    }

    /// Get quality metrics for recommendations
    pub fn get_quality_metrics(&self) -> QualityMetrics {
        let events = db::get_recent_events(24 * 30).unwrap_or_default(); // Last 30 days

        let mut total = 0;
        let mut successes = 0;

        for event in &events {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(event) {
                if val["type"].as_str() == Some("workflow_feedback") {
                    total += 1;
                    if val["data"]["success"].as_bool() == Some(true) {
                        successes += 1;
                    }
                }
            }
        }

        QualityMetrics {
            total_executions: total,
            successful_executions: successes,
            success_rate: if total > 0 {
                (successes as f64 / total as f64) * 100.0
            } else {
                0.0
            },
        }
    }
}

#[derive(Debug, Clone)]
pub struct QualityMetrics {
    pub total_executions: u32,
    pub successful_executions: u32,
    pub success_rate: f64,
}

impl std::fmt::Display for QualityMetrics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Quality: {}/{} executions ({:.1}% success rate)",
            self.successful_executions, self.total_executions, self.success_rate
        )
    }
}
