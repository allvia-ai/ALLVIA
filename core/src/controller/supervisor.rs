use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use anyhow::Result;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SupervisorDecision {
    pub action: String, // "accept", "review", "escalate"
    pub reason: String,
    pub focus_keywords: Vec<String>,
    pub notes: String,
}

pub struct Supervisor;

impl Supervisor {
    fn fallback_review(reason: String, notes: String) -> SupervisorDecision {
        SupervisorDecision {
            action: "review".to_string(),
            reason,
            focus_keywords: Vec::new(),
            notes,
        }
    }

    fn parse_decision(content: &str) -> Result<SupervisorDecision> {
        let trimmed = content.trim();
        if trimmed.is_empty() {
            return Ok(Self::fallback_review(
                "Supervisor returned empty response".to_string(),
                "Empty response from LLM".to_string(),
            ));
        }

        if let Ok(decision) = serde_json::from_str::<SupervisorDecision>(trimmed) {
            return Ok(decision);
        }

        if let Some(recovered) = crate::llm_gateway::recover_json(trimmed) {
            if let Ok(decision) = serde_json::from_value::<SupervisorDecision>(recovered.clone()) {
                return Ok(decision);
            }

            if let Some(inner) = recovered.get("action") {
                if inner.is_object() {
                    if let Ok(decision) = serde_json::from_value::<SupervisorDecision>(inner.clone()) {
                        return Ok(decision);
                    }
                }
            }
        }

        let preview = trimmed.chars().take(160).collect::<String>();
        Ok(Self::fallback_review(
            "Supervisor response was not valid JSON".to_string(),
            format!("Unparseable supervisor output: {}", preview),
        ))
    }

    pub async fn consult(
        llm: &dyn crate::llm_gateway::LLMClient,
        goal: &str,
        plan: &Value,
        history: &[String]
    ) -> Result<SupervisorDecision> {
        let system_prompt = crate::prompts::SUPERVISOR_SYSTEM_PROMPT;

        // Plan might be complex, simplify for prompt if needed
        let plan_str = serde_json::to_string_pretty(plan).unwrap_or_default();
        let history_str = history.join("\n");

        let user_msg = format!(
            "GOAL: {}\n\nHISTORY:\n{}\n\nPROPOSED ACTION:\n{}",
            goal,
            history_str,
            plan_str
        );

        let messages = vec![
            json!({ "role": "system", "content": system_prompt }),
            json!({ "role": "user", "content": user_msg })
        ];

        let content = llm.chat_completion(messages).await?;
        Self::parse_decision(&content)
    }
}
