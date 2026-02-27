#![allow(dead_code)]
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Workflow recommendation status
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum RecommendationStatus {
    Pending,
    Approved,
    Rejected,
}

impl RecommendationStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Approved => "approved",
            Self::Rejected => "rejected",
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Self {
        match s {
            "approved" => Self::Approved,
            "rejected" => Self::Rejected,
            _ => Self::Pending,
        }
    }
}

/// Trigger specification for a workflow
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggerSpec {
    pub trigger_type: String,   // gmail, schedule, webhook, file_watch
    pub filter: Option<String>, // e.g., "subject:미팅"
    pub params: serde_json::Value,
}

/// Action specification for a workflow
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionSpec {
    pub action_type: String, // calendar_add, telegram_notify, notion_create, etc.
    pub params: serde_json::Value,
    pub on_error: Option<String>, // continue, stop, retry
}

/// Feedback data for learning
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedbackData {
    pub success: bool,
    pub error_message: Option<String>,
    pub execution_time_ms: u64,
    pub user_rating: Option<i32>, // 1-5
}

/// Main workflow recommendation structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowRecommendation {
    pub id: String,
    pub title: String,
    pub summary: String,
    pub trigger: TriggerSpec,
    pub actions: Vec<ActionSpec>,
    pub n8n_prompt: String,
    pub confidence: f64,
    pub status: RecommendationStatus,
    pub created_at: DateTime<Utc>,
    pub approved_at: Option<DateTime<Utc>>,
    pub n8n_workflow_id: Option<String>,
    pub feedback: Option<FeedbackData>,
}

impl WorkflowRecommendation {
    pub fn new(
        title: String,
        summary: String,
        trigger: TriggerSpec,
        actions: Vec<ActionSpec>,
        n8n_prompt: String,
        confidence: f64,
    ) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            title,
            summary,
            trigger,
            actions,
            n8n_prompt,
            confidence,
            status: RecommendationStatus::Pending,
            created_at: Utc::now(),
            approved_at: None,
            n8n_workflow_id: None,
            feedback: None,
        }
    }

    pub fn approve(&mut self) {
        self.status = RecommendationStatus::Approved;
        self.approved_at = Some(Utc::now());
    }

    pub fn reject(&mut self) {
        self.status = RecommendationStatus::Rejected;
    }

    pub fn mark_executed(&mut self, workflow_id: String) {
        // Execution success does not change review state.
        self.n8n_workflow_id = Some(workflow_id);
    }

    pub fn mark_failed(&mut self, error: String) {
        // Execution failure is captured in feedback, not as a review-state transition.
        self.feedback = Some(FeedbackData {
            success: false,
            error_message: Some(error),
            execution_time_ms: 0,
            user_rating: None,
        });
    }
}

/// Detected pattern from logs
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectedPattern {
    pub pattern_id: String,
    pub pattern_type: String, // app_sequence, keyword, file_pattern
    pub occurrences: u32,
    pub similarity_score: f64,
    pub sample_events: Vec<String>,
    pub detected_at: DateTime<Utc>,
}

impl DetectedPattern {
    pub fn should_recommend(&self) -> bool {
        // 7일 내 3회 이상 반복, 유사도 0.8 이상
        self.occurrences >= 3 && self.similarity_score >= 0.8
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_recommendation_status_transitions() {
        let trigger = TriggerSpec {
            trigger_type: "gmail".to_string(),
            filter: None,
            params: json!({}),
        };
        let action = ActionSpec {
            action_type: "telegram".to_string(),
            params: json!({}),
            on_error: None,
        };

        let mut rec = WorkflowRecommendation::new(
            "Test Workflow".to_string(),
            "Summary".to_string(),
            trigger,
            vec![action],
            "Prompt".to_string(),
            0.9,
        );

        assert_eq!(rec.status, RecommendationStatus::Pending);
        assert!(rec.approved_at.is_none());

        rec.approve();
        assert_eq!(rec.status, RecommendationStatus::Approved);
        assert!(rec.approved_at.is_some());

        rec.mark_executed("workflow-123".to_string());
        assert_eq!(rec.status, RecommendationStatus::Approved);
        assert_eq!(rec.n8n_workflow_id, Some("workflow-123".to_string()));

        rec.mark_failed("API Error".to_string());
        assert_eq!(rec.status, RecommendationStatus::Approved);
        assert!(rec.feedback.is_some());
        assert_eq!(
            rec.feedback.unwrap().error_message,
            Some("API Error".to_string())
        );
    }

    #[test]
    fn test_serialization() {
        let status = RecommendationStatus::Approved;
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(json, "\"Approved\"");

        let deserialized: RecommendationStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, status);
    }
}
