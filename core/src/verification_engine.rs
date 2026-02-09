use crate::nl_automation::{Plan, StepType, VerificationResult};

pub fn verify_plan(plan: &Plan) -> VerificationResult {
    let mut issues = Vec::new();
    if plan.steps.is_empty() {
        issues.push("Plan has no steps".to_string());
    }

    let has_extract = plan
        .steps
        .iter()
        .any(|s| matches!(s.step_type, StepType::Extract));
    if !has_extract {
        issues.push("No extract step found for result verification".to_string());
    }

    if matches!(plan.intent, crate::nl_automation::IntentType::FlightSearch) {
        if plan.slots.get("from").map(|v| v.is_empty()).unwrap_or(true) {
            issues.push("Missing flight origin (from)".to_string());
        }
        if plan.slots.get("to").map(|v| v.is_empty()).unwrap_or(true) {
            issues.push("Missing flight destination (to)".to_string());
        }
        if plan
            .slots
            .get("date_start")
            .map(|v| v.is_empty())
            .unwrap_or(true)
        {
            issues.push("Missing flight start date".to_string());
        }
    }

    if matches!(
        plan.intent,
        crate::nl_automation::IntentType::ShoppingCompare
    ) {
        if plan
            .slots
            .get("product_name")
            .map(|v| v.is_empty())
            .unwrap_or(true)
        {
            issues.push("Missing product name".to_string());
        }
    }

    if matches!(plan.intent, crate::nl_automation::IntentType::FormFill) {
        if plan
            .slots
            .get("form_purpose")
            .map(|v| v.is_empty())
            .unwrap_or(true)
        {
            issues.push("Missing form purpose".to_string());
        }
    }

    VerificationResult {
        ok: issues.is_empty(),
        issues,
    }
}

pub fn verify_execution(plan: &Plan, logs: &[String]) -> VerificationResult {
    let base = verify_plan(plan);
    let mut issues = base.issues;
    let has_summary = logs.iter().any(|line| line.starts_with("Summary: "));
    let has_manual = logs
        .iter()
        .any(|line| line.to_lowercase().contains("manual input"));
    let has_blocked = logs
        .iter()
        .any(|line| line.to_lowercase().contains("blocked"));

    if matches!(
        plan.intent,
        crate::nl_automation::IntentType::FlightSearch
            | crate::nl_automation::IntentType::ShoppingCompare
    ) && !has_summary
    {
        issues.push("No summary extracted".to_string());
    }
    if has_manual {
        issues.push("Manual input required during execution".to_string());
    }
    if has_blocked {
        issues.push("Execution blocked by policy".to_string());
    }

    VerificationResult {
        ok: issues.is_empty(),
        issues,
    }
}
