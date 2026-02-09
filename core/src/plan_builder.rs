use crate::nl_automation::{IntentType, Plan, PlanStep, SlotMap, StepType};
use serde_json::json;
use uuid::Uuid;

pub fn build_plan(intent: &IntentType, slots: &SlotMap) -> Plan {
    let plan_id = Uuid::new_v4().to_string();
    let mut steps = Vec::new();

    match intent {
        IntentType::FlightSearch => {
            let url = build_flight_url(slots)
                .unwrap_or_else(|| "https://www.google.com/travel/flights".to_string());
            steps.push(step(
                StepType::Navigate,
                "Open Google Flights",
                json!({"url": url}),
            ));
            steps.push(step(
                StepType::Wait,
                "Wait for page to load",
                json!({"seconds": 2}),
            ));
            steps.push(step(
                StepType::Fill,
                "Enter departure",
                json!({"field": "from", "value": slots.get("from"), "auto": true}),
            ));
            steps.push(step(
                StepType::Fill,
                "Enter destination",
                json!({"field": "to", "value": slots.get("to"), "auto": true}),
            ));
            steps.push(step(
                StepType::Fill,
                "Enter dates",
                json!({"field": "date_start", "value": slots.get("date_start"), "date_end": slots.get("date_end"), "auto": true}),
            ));
            steps.push(step(
                StepType::Select,
                "Apply filters",
                json!({"budget": slots.get("budget_max"), "time_window": slots.get("time_window"), "direct_only": slots.get("direct_only"), "auto": true}),
            ));
            steps.push(step(
                StepType::Extract,
                "Extract top 3 results",
                json!({"fields": ["price", "time", "stops"]}),
            ));
            steps.push(step(
                StepType::Approve,
                "Ask user to open booking link",
                json!({"action": "open_booking_link"}),
            ));
        }
        IntentType::ShoppingCompare => {
            let url = build_shopping_url(slots)
                .unwrap_or_else(|| "https://shopping.naver.com".to_string());
            steps.push(step(
                StepType::Navigate,
                "Open price comparison site",
                json!({"url": url}),
            ));
            steps.push(step(
                StepType::Wait,
                "Wait for results",
                json!({"seconds": 2}),
            ));
            steps.push(step(
                StepType::Fill,
                "Search product",
                json!({"field": "query", "value": slots.get("product_name"), "auto": true}),
            ));
            steps.push(step(
                StepType::Click,
                "Submit search",
                json!({"action": "submit_search", "auto": true}),
            ));
            steps.push(step(
                StepType::Select,
                "Apply filters",
                json!({"brand": slots.get("brand"), "price_min": slots.get("price_min"), "price_max": slots.get("price_max"), "auto": true}),
            ));
            steps.push(step(
                StepType::Extract,
                "Extract top 3 products",
                json!({"fields": ["price", "seller", "shipping"]}),
            ));
            steps.push(step(
                StepType::Approve,
                "Ask user to open product link",
                json!({"action": "open_product_link"}),
            ));
        }
        IntentType::FormFill => {
            let url = slots
                .get("target_url")
                .cloned()
                .unwrap_or_else(|| "".to_string());
            steps.push(step(
                StepType::Navigate,
                "Open target form",
                json!({"url": url}),
            ));
            steps.push(step(
                StepType::Extract,
                "Detect form fields",
                json!({"fields": ["name", "email", "phone", "address"]}),
            ));
            steps.push(step(
                StepType::Fill,
                "Fill form fields from profile",
                json!({"profile_id": slots.get("profile_id"), "field": "form_profile", "auto": true}),
            ));
            steps.push(step(
                StepType::Approve,
                "Ask user to submit form",
                json!({"action": "submit_form"}),
            ));
        }
        IntentType::GenericTask => {
            steps.push(step(
                StepType::Wait,
                "Collect more details from user",
                json!({"note": "Need clarification"}),
            ));
        }
    }

    Plan {
        plan_id,
        intent: intent.clone(),
        slots: slots.clone(),
        steps,
    }
}

fn step(step_type: StepType, description: &str, data: serde_json::Value) -> PlanStep {
    PlanStep {
        step_id: Uuid::new_v4().to_string(),
        step_type,
        description: description.to_string(),
        data,
    }
}

fn build_flight_url(slots: &SlotMap) -> Option<String> {
    let from = slots.get("from")?.trim();
    let to = slots.get("to")?.trim();
    let date = slots.get("date_start")?.trim();
    if from.is_empty() || to.is_empty() || date.is_empty() {
        return None;
    }
    let query = format!("Flights from {} to {} on {}", from, to, date);
    Some(format!(
        "https://www.google.com/travel/flights?q={}",
        urlencoding::encode(&query)
    ))
}

fn build_shopping_url(slots: &SlotMap) -> Option<String> {
    let product = slots.get("product_name")?.trim();
    if product.is_empty() {
        return None;
    }
    Some(format!(
        "https://search.shopping.naver.com/search/all?query={}",
        urlencoding::encode(product)
    ))
}
