use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum IntentType {
    FlightSearch,
    ShoppingCompare,
    FormFill,
    GenericTask,
}

impl IntentType {
    pub fn as_str(&self) -> &'static str {
        match self {
            IntentType::FlightSearch => "flight_search",
            IntentType::ShoppingCompare => "shopping_compare",
            IntentType::FormFill => "form_fill",
            IntentType::GenericTask => "generic_task",
        }
    }
}

pub type SlotMap = HashMap<String, String>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntentResult {
    pub intent: IntentType,
    pub confidence: f32,
    pub slots: SlotMap,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlotFillResult {
    pub slots: SlotMap,
    pub missing: Vec<String>,
    pub follow_up: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepType {
    Navigate,
    Click,
    Fill,
    Select,
    Wait,
    Extract,
    Screenshot,
    Approve,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanStep {
    pub step_id: String,
    pub step_type: StepType,
    pub description: String,
    pub data: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    pub plan_id: String,
    pub intent: IntentType,
    pub slots: SlotMap,
    pub steps: Vec<PlanStep>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalContext {
    #[serde(default)]
    pub approval_id: Option<String>,
    pub action: String,
    pub message: String,
    pub risk_level: String,
    pub policy: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionResult {
    pub status: String,
    pub logs: Vec<String>,
    pub approval: Option<ApprovalContext>,
    #[serde(default)]
    pub manual_steps: Vec<String>,
    pub resume_from: Option<usize>,
    #[serde(default)]
    pub resume_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationResult {
    pub ok: bool,
    pub issues: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalDecision {
    pub status: String,
    pub requires_approval: bool,
    pub message: String,
    pub risk_level: String,
    pub policy: String,
}
