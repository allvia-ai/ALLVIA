use crate::db::Recommendation;
use crate::llm_gateway::LLMClient;
use serde_json::json;
use std::sync::Arc;

pub struct ArchitectSession {
    llm: Arc<dyn LLMClient>,
    pub recommendation: Recommendation,
    history: Vec<serde_json::Value>,
}

impl ArchitectSession {
    pub fn new(llm: Arc<dyn LLMClient>, recommendation: Recommendation) -> Self {
        // Initial System Prompt
        let system_prompt = format!(
            "You are 'The Architect', an intelligent automation expert.
You are helping the user refine a workflow recommendation.

**Recommendation**: {}
**Summary**: {}
**Trigger**: {}
**Draft Prompt**: {}

**Goal**:
Ask the user clarifying questions to customize this automation. 
Example: 'Who should I email?', 'What keywords are you looking for?', 'What time should this run?'
Do NOT ask technical questions about JSON or API keys unless necessary. Stick to user preferences.

After gathering 2-3 key details, ask for confirmation to 'Build' it.
If the user says 'Build' or 'Yes', output the token '[BUILD_COMPLETED]' and a summary of the final plan.",
            recommendation.title,
            recommendation.summary,
            recommendation.trigger,
            recommendation.n8n_prompt
        );

        ArchitectSession {
            llm,
            recommendation,
            history: vec![
                json!({ "role": "system", "content": system_prompt })
            ],
        }
    }

    pub async fn start(&mut self) -> Result<String, String> {
        // Generate opening line
        self.history.push(json!({
            "role": "user", 
            "content": "Start the interview. Briefly explain what this automation does and ask the first question." 
        }));

        let response = self.call_llm().await?;
        Ok(response)
    }

    pub async fn chat(&mut self, user_input: &str) -> Result<String, String> {
        self.history.push(json!({ "role": "user", "content": user_input }));
        
        // Save to DB (Chat Memory) for UI to show? 
        // Or we rely on the frontend to display what it just sent.
        // But we should verify if `run_agent_task` expects us to return the AI response.

        let response = self.call_llm().await?;
        
        // Check for [BUILD_COMPLETED]
        if response.contains("[BUILD_COMPLETED]") {
             // Do not auto-approve on marker text alone.
             // Explicit user approval endpoint must perform the state transition.
             return Ok(response.replace("[BUILD_COMPLETED]", "✅ **Automation Plan Ready** (승인 대기)"));
        }

        Ok(response)
    }

    async fn call_llm(&mut self) -> Result<String, String> {
        // Convert history to format needed by LLMClient
        // LLMClient::chat accepts strictly typed history? 
        // Let's check `llm_gateway.rs`. It usually takes `Vec<Value>` or specific struct.
        // Assuming `chat_completion` takes json array.
        
        // We clone to avoid borrow issues if needed, or pass ref.
        let messages = self.history.clone(); 
        
        let response = self.llm.chat_completion(messages).await
            .map_err(|e| e.to_string())?;

        self.history.push(json!({ "role": "assistant", "content": response }));
        Ok(response)
    }
}
