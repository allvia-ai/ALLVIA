use crate::context_pruning;
use crate::mcp_client;
use crate::recommendation::AutomationProposal;
use anyhow::Result;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::env;

use async_trait::async_trait;
use std::time::Duration;

// =====================================================
// Phase 30: Intelligence Upgrade (Supervisor + Thinking)
// =====================================================

// =====================================================
// Phase 29: Robust JSON Recovery (Advanced CLI)
// =====================================================

/// Attempt to recover valid JSON from malformed LLM responses
/// Handles: markdown blocks, partial JSON, common syntax errors
pub fn recover_json(raw: &str) -> Option<Value> {
    // Step 1: Clean markdown code blocks
    let clean = raw
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim()
        .to_string();

    // Step 2: Try direct parse
    if let Ok(v) = serde_json::from_str::<Value>(&clean) {
        return Some(v);
    }

    // Step 3: Find first { and last } and extract
    if let (Some(start), Some(end)) = (clean.find('{'), clean.rfind('}')) {
        if start < end {
            let json_candidate = &clean[start..=end];
            if let Ok(v) = serde_json::from_str::<Value>(json_candidate) {
                return Some(v);
            }

            // Step 4: Try fixing common errors
            // 4a: Trailing comma before }
            let fixed = json_candidate.replace(",}", "}").replace(",]", "]");
            if let Ok(v) = serde_json::from_str::<Value>(&fixed) {
                return Some(v);
            }

            // 4b: Unquoted keys (simple cases)
            let re_unquoted = regex::Regex::new(r#"(\{|,)\s*(\w+)\s*:"#).ok()?;
            let with_quotes = re_unquoted.replace_all(&fixed, r#"$1"$2":"#);
            if let Ok(v) = serde_json::from_str::<Value>(&with_quotes) {
                return Some(v);
            }
        }
    }

    // Step 5: Look for action pattern in text
    // e.g., "I will click" -> {"action": "click_visual", "description": "..."}
    let lower = clean.to_lowercase();
    if lower.contains("done") || lower.contains("goal achieved") || lower.contains("completed") {
        return Some(json!({"action": "done"}));
    }

    None
}

#[async_trait]
pub trait LLMClient: Send + Sync {
    async fn plan_next_step(
        &self,
        goal: &str,
        ui_tree: &Value,
        action_history: &[String],
    ) -> Result<Value>;
    async fn chat_completion(&self, messages: Vec<Value>) -> Result<String>;
    async fn plan_vision_step(
        &self,
        goal: &str,
        image_b64: &str,
        history: &[String],
    ) -> Result<Value>;
    async fn analyze_routine(&self, logs: &[String]) -> Result<String>;
    async fn recommend_automation(&self, logs: &[String]) -> Result<String>;
    async fn build_n8n_workflow(&self, user_prompt: &str) -> Result<String>;
    async fn fix_n8n_workflow(
        &self,
        user_prompt: &str,
        bad_json: &str,
        error_msg: &str,
    ) -> Result<String, Box<dyn std::error::Error>>;
    async fn get_embedding(&self, text: &str) -> Result<Vec<f32>>;
    async fn propose_workflow(
        &self,
        logs: &[String],
    ) -> Result<AutomationProposal, Box<dyn std::error::Error>>;
    async fn analyze_tendency(&self, logs: &[String]) -> Result<String>;
    async fn parse_intent(&self, user_input: &str) -> Result<Value>;
    async fn parse_intent_with_history(
        &self,
        user_input: &str,
        history: &[crate::db::ChatMessage],
    ) -> Result<Value>;
    async fn generate_recommendation_from_pattern(
        &self,
        pattern_description: &str,
        sample_events: &[String],
    ) -> Result<AutomationProposal>;
    async fn analyze_screen(
        &self,
        prompt: &str,
        image_b64: &str,
    ) -> Result<String, Box<dyn std::error::Error>>;
    async fn find_element_coordinates(
        &self,
        element_description: &str,
        image_b64: &str,
    ) -> Result<Option<(i32, i32)>>;
    async fn score_quality(
        &self,
        system_prompt: &str,
        payload: &serde_json::Value,
    ) -> Result<String>;
    async fn propose_solution_stack(&self, goal: &str) -> Result<Value>;
    async fn inference_local(&self, prompt: &str, model: Option<&str>) -> Result<String>;
    fn route_task(&self, task_description: &str, pii_detected: bool) -> (bool, String);
    async fn analyze_user_feedback(
        &self,
        feedback: &str,
        history_summary: &str,
    ) -> Result<FeedbackAnalysis>;
}

#[derive(Clone)]
pub struct OpenAILLMClient {
    pub client: reqwest::Client,
    pub api_key: String,
    pub model: String,
}

impl OpenAILLMClient {
    pub fn new() -> Result<Self> {
        crate::load_env_with_fallback();
        let api_key = env::var("OPENAI_API_KEY")
            .map_err(|_| anyhow::anyhow!("OPENAI_API_KEY not set in .env"))?;
        let client = Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(120)) // [Fix] Total request timeout
            .connect_timeout(Duration::from_secs(10)) // [Fix] Connection timeout
            .build()?;

        let default_model = env::var("STEER_OPENAI_MODEL")
            .or_else(|_| env::var("OPENAI_MODEL"))
            .ok()
            .map(|m| m.trim().to_string())
            .filter(|m| !m.is_empty())
            .unwrap_or_else(|| "gpt-4o-mini".to_string());

        Ok(Self {
            client,
            api_key,
            model: default_model,
        })
    }

    fn llm_primary_mode() -> Option<String> {
        env::var("STEER_LLM_PRIMARY")
            .ok()
            .map(|v| v.trim().to_lowercase())
            .filter(|v| !v.is_empty())
    }

    fn cli_first_enabled() -> bool {
        matches!(
            Self::llm_primary_mode().as_deref(),
            Some("cli")
                | Some("codex")
                | Some("gemini")
                | Some("claude")
                | Some("local")
                | Some("llama")
        )
    }

    fn vision_model(&self) -> String {
        env::var("STEER_VISION_MODEL")
            .ok()
            .map(|m| m.trim().to_string())
            .filter(|m| !m.is_empty())
            .unwrap_or_else(|| self.model.clone())
    }

    fn history_contains_case_insensitive(history: &[String], needle: &str) -> bool {
        let needle_lower = needle.to_lowercase();
        history
            .iter()
            .any(|h| h.to_lowercase().contains(&needle_lower))
    }

    fn ordered_apps_in_goal(goal: &str) -> Vec<&'static str> {
        let goal_lower = goal.to_lowercase();
        let app_catalog: [&'static str; 7] = [
            "Calendar",
            "Safari",
            "Finder",
            "TextEdit",
            "Notes",
            "Calculator",
            "Mail",
        ];

        let mut found: Vec<(usize, &'static str)> = app_catalog
            .iter()
            .filter_map(|app| goal_lower.find(&app.to_lowercase()).map(|idx| (idx, *app)))
            .collect();
        found.sort_by_key(|(idx, _)| *idx);
        found.into_iter().map(|(_, app)| app).collect()
    }

    fn cmd_sequence_from_goal(goal: &str) -> Vec<char> {
        let text = goal.to_lowercase();
        let mut seq = Vec::new();
        let mut start = 0usize;

        while let Some(rel) = text[start..].find("cmd+") {
            let pos = start + rel + 4;
            let rest = &text[pos..];
            let mut pushed = false;
            for ch in rest.chars() {
                if ch.is_ascii_whitespace() {
                    continue;
                }
                if matches!(ch, 'a' | 'c' | 'v' | 'n') {
                    seq.push(ch);
                }
                pushed = true;
                break;
            }
            if pushed {
                start = pos.saturating_add(1);
            } else {
                break;
            }
        }

        seq
    }

    fn cmd_sequence_from_history(history: &[String]) -> Vec<char> {
        let mut seq = Vec::new();
        for entry in history {
            let lower = entry.to_lowercase();
            if lower.contains("selected all contents") {
                seq.push('a');
                continue;
            }
            if lower.contains("copied selection") {
                seq.push('c');
                continue;
            }
            if lower.contains("pasted clipboard contents") {
                seq.push('v');
                continue;
            }
            if lower.contains("shortcut 'n'") && lower.contains("command") {
                seq.push('n');
                continue;
            }
            if lower.contains("shortcut 'a'") && lower.contains("command") {
                seq.push('a');
                continue;
            }
            if lower.contains("shortcut 'c'") && lower.contains("command") {
                seq.push('c');
                continue;
            }
            if lower.contains("shortcut 'v'") && lower.contains("command") {
                seq.push('v');
                continue;
            }
        }
        seq
    }

    fn next_missing_cmd(goal: &str, history: &[String]) -> Option<char> {
        let expected = Self::cmd_sequence_from_goal(goal);
        if expected.is_empty() {
            return None;
        }
        let actual = Self::cmd_sequence_from_history(history);
        let mut matched_prefix = 0usize;
        for c in actual {
            if matched_prefix < expected.len() && c == expected[matched_prefix] {
                matched_prefix += 1;
            }
        }
        expected.get(matched_prefix).copied()
    }

    fn history_count_case_insensitive(history: &[String], needle: &str) -> usize {
        let needle_lower = needle.to_lowercase();
        history
            .iter()
            .filter(|h| h.to_lowercase().contains(&needle_lower))
            .count()
    }

    fn extract_single_quoted_fragments(goal: &str) -> Vec<String> {
        let mut out = Vec::new();
        let parts: Vec<&str> = goal.split('\'').collect();
        for (idx, part) in parts.iter().enumerate() {
            if idx % 2 == 1 {
                let trimmed = part.trim();
                if !trimmed.is_empty() {
                    out.push(trimmed.to_string());
                }
            }
        }
        out
    }

    fn first_missing_fragment_by_keywords(
        goal: &str,
        history: &[String],
        keywords: &[&str],
    ) -> Option<String> {
        let history_blob = history.join("\n").to_lowercase();
        Self::extract_single_quoted_fragments(goal)
            .into_iter()
            .find(|frag| {
                let frag_lower = frag.to_lowercase();
                let keyword_match = keywords
                    .iter()
                    .any(|k| frag_lower.contains(&k.to_lowercase()));
                keyword_match && !history_blob.contains(&frag_lower)
            })
    }

    fn fallback_vision_action(&self, goal: &str, history: &[String]) -> Value {
        let goal_lower = goal.to_lowercase();

        if let Some(next_cmd) = Self::next_missing_cmd(goal, history) {
            let action = match next_cmd {
                'n' => json!({ "action": "shortcut", "key": "n", "modifiers": ["command"] }),
                'a' => json!({ "action": "select_all" }),
                'c' => {
                    let copy_count =
                        Self::history_count_case_insensitive(history, "Copied selection");
                    let calculator_opened =
                        Self::history_contains_case_insensitive(history, "Opened app: Calculator");
                    if copy_count >= 1 && !calculator_opened {
                        if let Some(status_text) = Self::first_missing_fragment_by_keywords(
                            goal,
                            history,
                            &["상태", "status"],
                        ) {
                            json!({ "action": "type", "text": status_text })
                        } else {
                            json!({ "action": "open_app", "name": "Calculator" })
                        }
                    } else {
                        json!({ "action": "copy" })
                    }
                }
                'v' => {
                    let calculator_opened =
                        Self::history_contains_case_insensitive(history, "Opened app: Calculator");
                    if calculator_opened {
                        if let Some(cost_text) = Self::first_missing_fragment_by_keywords(
                            goal,
                            history,
                            &["예상비용", "cost", "budget"],
                        ) {
                            json!({ "action": "type", "text": cost_text })
                        } else {
                            json!({ "action": "paste" })
                        }
                    } else {
                        json!({ "action": "paste" })
                    }
                }
                _ => json!({ "action": "wait", "seconds": 1 }),
            };
            return action;
        }

        for app in Self::ordered_apps_in_goal(goal) {
            let opened_marker = format!("Opened app: {}", app);
            if !Self::history_contains_case_insensitive(history, &opened_marker) {
                return json!({ "action": "open_app", "name": app });
            }
        }

        if goal_lower.contains("google")
            || goal_lower.contains("검색")
            || goal_lower.contains("search")
        {
            return json!({ "action": "open_url", "url": "https://www.google.com" });
        }

        json!({ "action": "wait", "seconds": 1 })
    }

    /// Internal helper for robust API calls (Retry Logic)
    /// Internal helper for robust API calls (Retry Logic)
    /// Returns parsed JSON Value on success.
    /// Returns specific errors for Rate Limit (Quota vs Burst) or HTTP errors.
    pub async fn post_with_retry(
        &self,
        url: &str,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value, anyhow::Error> {
        let max_retries = std::env::var("STEER_OPENAI_MAX_RETRIES")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .filter(|v| *v >= 1 && *v <= 10)
            .unwrap_or(1); // Default reduced to 1 for faster fallback
        let retry_429_sec = std::env::var("STEER_OPENAI_429_RETRY_SEC")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|v| *v >= 1 && *v <= 120)
            .unwrap_or(2); // Default reduced to 2s
        let mut attempt = 0;
        let mut backoff = tokio::time::Duration::from_secs(1);

        loop {
            attempt += 1;
            let mut wait_override: Option<tokio::time::Duration> = None;

            let req = self
                .client
                .post(url)
                .header("Authorization", format!("Bearer {}", self.api_key))
                .json(body);

            match req.send().await {
                Ok(resp) => {
                    let status = resp.status();
                    if status.is_success() {
                        return Ok(resp.json().await?);
                    }

                    // Clone headers before consuming body
                    let headers = resp.headers().clone();
                    // Read error body
                    let error_text = resp.text().await.unwrap_or_default();

                    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                        // Check for hard quota vs soft rate limit
                        if error_text.to_lowercase().contains("quota")
                            || error_text.to_lowercase().contains("billing")
                            || error_text.contains("access_terminated")
                        {
                            return Err(anyhow::anyhow!("RATE_LIMITED_QUOTA: {}", error_text));
                        }

                        if attempt > max_retries {
                            return Err(anyhow::anyhow!(
                                "RATE_LIMITED_EXHAUSTED: OpenAI 429 after {} retries. Error: {}",
                                max_retries,
                                error_text
                            ));
                        }

                        let retry_after_header = headers
                            .get(reqwest::header::RETRY_AFTER)
                            .and_then(|v| v.to_str().ok())
                            .and_then(|s| s.trim().parse::<u64>().ok())
                            .filter(|v| *v >= 1 && *v <= 300);
                        let retry_after = retry_after_header.unwrap_or(retry_429_sec);
                        wait_override = Some(tokio::time::Duration::from_secs(retry_after));
                        eprintln!(
                            "⚠️ LLM rate limited (429). Body: '{}'. Retrying in {}s (attempt {}/{})...",
                            error_text.replace('\n', " "), retry_after, attempt, max_retries
                        );
                    } else if status.is_server_error() {
                        if attempt > max_retries {
                            return Err(anyhow::anyhow!("HTTP {}: {}", status, error_text));
                        }
                        // retry 5xx
                    } else {
                        // 4xx (Client Error) - Fail immediately
                        return Err(anyhow::anyhow!("HTTP {}: {}", status, error_text));
                    }
                }
                Err(e) => {
                    if attempt > max_retries {
                        return Err(anyhow::anyhow!("Max retries exceeded: {}", e));
                    }
                    eprintln!(
                        "⚠️ LLM Network Error (Attempt {}/{}): {}. Retrying in {:?}...",
                        attempt, max_retries, e, backoff
                    );
                }
            }

            let sleep_for = wait_override.unwrap_or(backoff);
            tokio::time::sleep(sleep_for).await;
            backoff = std::cmp::max(backoff * 2, sleep_for);
        }
    }

    /// Dynamically build context for workflow generation based on available integrations & tools
    fn get_workflow_context(&self) -> String {
        let mut context = String::from("## AVAILABLE NODES\n");

        // Always available core nodes
        context.push_str("### Core Nodes (Always Available)\n");
        context.push_str("- Triggers: n8n-nodes-base.cron, n8n-nodes-base.webhook, n8n-nodes-base.manualTrigger\n");
        context.push_str("- HTTP: n8n-nodes-base.httpRequest (v4)\n");
        context
            .push_str("- Logic: n8n-nodes-base.if, n8n-nodes-base.switch, n8n-nodes-base.merge\n");
        context
            .push_str("- Data: n8n-nodes-base.set, n8n-nodes-base.code, n8n-nodes-base.function\n");
        context
            .push_str("- Files: n8n-nodes-base.readBinaryFiles, n8n-nodes-base.writeBinaryFile\n");
        context.push_str("- OS Control: n8n-nodes-base.executeCommand\n\n");

        context.push_str("### OS AUTOMATION CAPABILITIES\n");

        // Check for 'cliclick' (better mouse control)
        let has_cliclick = std::process::Command::new("which")
            .arg("cliclick")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);

        if has_cliclick {
            context.push_str("- ✅ EXACT MOUSE CONTROL: 'cliclick' IS INSTALLED.\n");
            context
                .push_str("  - Use: `cliclick c:x,y` (click), `cliclick dc:x,y` (double click)\n");
        } else {
            context.push_str("- ⚠️ MOUSE CONTROL: 'cliclick' is NOT installed.\n");
            context.push_str("  - PREFERRED: Use AppleScript via `osascript` for basic clicks if absolutely necessary, OR suggest installing cliclick.\n");
            context.push_str("  - Command: `osascript -e 'tell application \"System Events\" to click at {x,y}'` (Note: requires Accessibility permission)\n");
        }

        context.push_str("- ✅ KEYBOARD: Use AppleScript via `osascript`.\n");
        context.push_str("  - Command: `osascript -e 'tell application \"System Events\" to keystroke \"text\"'`\n\n");

        context.push_str("### OS AUTOMATION RULES (CRITICAL)\n");
        context.push_str("1. DO NOT invent nodes like 'n8n-nodes-base.click'. Use 'n8n-nodes-base.executeCommand'.\n");
        context.push_str(
            "2. ALWAYS wrap OS commands in a way that handles potential permissions errors.\n",
        );

        // Check for configured integrations
        context.push_str("### Configured Integrations (Prefer These)\n");

        // Check env vars for configured services
        if std::env::var("GOOGLE_CLIENT_ID").is_ok() || std::env::var("GMAIL_CREDENTIALS").is_ok() {
            context.push_str(
                "- ✅ Gmail: n8n-nodes-base.gmail, n8n-nodes-base.gmailTrigger (CONFIGURED)\n",
            );
            context.push_str("- ✅ Google Calendar: n8n-nodes-base.googleCalendar (CONFIGURED)\n");
            context.push_str("- ✅ Google Sheets: n8n-nodes-base.googleSheets (CONFIGURED)\n");
        }

        if std::env::var("SLACK_TOKEN").is_ok() || std::env::var("SLACK_WEBHOOK").is_ok() {
            context.push_str("- ✅ Slack: n8n-nodes-base.slack (CONFIGURED)\n");
        }

        if std::env::var("TELEGRAM_BOT_TOKEN").is_ok() {
            context.push_str("- ✅ Telegram: n8n-nodes-base.telegram (CONFIGURED)\n");
        }

        if std::env::var("NOTION_API_KEY").is_ok() {
            context.push_str("- ✅ Notion: n8n-nodes-base.notion (CONFIGURED)\n");
        }

        if std::env::var("OPENAI_API_KEY").is_ok() {
            context.push_str("- ✅ OpenAI: @n8n/n8n-nodes-langchain.openAi (CONFIGURED)\n");
        }

        // Other common nodes that can be added without credentials
        context.push_str("\n### Other Popular Nodes\n");
        context.push_str("- Discord: n8n-nodes-base.discord\n");
        context.push_str("- GitHub: n8n-nodes-base.github\n");
        context.push_str("- Airtable: n8n-nodes-base.airtable\n");
        context.push_str("- RSS: n8n-nodes-base.rssFeedRead\n");
        context.push_str("- Wait: n8n-nodes-base.wait\n");
        context.push_str("- DateTime: n8n-nodes-base.dateTime\n");

        context
    }
}

#[async_trait]
impl LLMClient for OpenAILLMClient {
    #[allow(dead_code)]
    async fn plan_next_step(
        &self,
        goal: &str,
        ui_tree: &Value,
        action_history: &[String],
    ) -> Result<Value> {
        let system_prompt = r#"
You are a MacOS Automation Agent. Your job is to FULLY achieve the user's goal.
You CAN control the ENTIRE computer - you can open anything, navigate anywhere.
You MUST think step-by-step using <think> tags before deciding an action.

Format:
<think>
1. Analyze current UI state vs Goal.
2. Identify missing information or next logical step.
3. Validate if the last action succeeded.
4. Formulate the specific JSON action.
</think>
{ "action": ... }

Available Actions:

### OPENING APPS/WEBSITES:
1. Open URL: { "action": "open_url", "url": "https://..." }
2. Shell: { "action": "shell.run", "command": "..." }
3. Search Files: { "action": "system.search", "query": "..." }
4. Read File: { "action": "shell.run", "command": "cat /path/to/file.txt" }

### READING CONTENT:
5. Read Web Page: { "action": "read_page" }
6. Read UI: { "action": "ui.read" }

### UI INTERACTION:
7. Click Element: { "action": "ui.click", "element_id": "UUID" }
8. Click Text (POWERFUL): { "action": "ui.click_text", "text": "Button Label" }
9. Type: { "action": "ui.type", "text": "Hello" }

### COMPLETION:
10. Report: { "action": "report", "message": "Here's what I found: ..." }
11. Done: { "action": "done" }
12. Fail: { "action": "fail", "reason": "..." }

Output internal monologue in <think>...</think> followed by ONLY valid JSON.
"#;

        let history_str = if action_history.is_empty() {
            "None yet".to_string()
        } else {
            action_history.join("\n- ")
        };

        let user_msg = format!(
            "GOAL: {}\n\nPREVIOUS ACTIONS I'VE TAKEN:\n- {}\n\nCURRENT UI STATE:\n{}",
            goal,
            history_str,
            serde_json::to_string_pretty(ui_tree).unwrap_or_default()
        );

        let request_body = json!({
            "model": &self.model,
            "messages": [
                { "role": "system", "content": system_prompt },
                { "role": "user", "content": user_msg }
            ],
            "response_format": { "type": "json_object" },
            "temperature": 0.0
        });

        let body = match self
            .post_with_retry("https://api.openai.com/v1/chat/completions", &request_body)
            .await
        {
            Ok(r) => r,
            Err(e) if e.to_string().contains("RATE_LIMITED") => {
                eprintln!("⚠️ [plan_next_step] OpenAI rate limited (quota/exhausted), invoking fallback chain...");
                let messages = vec![
                    json!({"role": "system", "content": system_prompt}),
                    json!({"role": "user", "content": user_msg}),
                ];
                let fallback_result = crate::cli_llm::fallback_chat_completion(&messages).await?;
                let action_json = match recover_json(&fallback_result) {
                    Some(v) => v,
                    None => {
                        let fallback_action = self.fallback_vision_action(goal, action_history);
                        eprintln!(
                            "⚠️ [plan_next_step] Failed to parse fallback JSON; using deterministic fallback: {}",
                            fallback_action
                        );
                        fallback_action
                    }
                };
                return Ok(action_json);
            }
            Err(e) => {
                eprintln!(
                    "⚠️ [plan_next_step] OpenAI error: {}. Invoking fallback chain...",
                    e
                );
                let messages = vec![
                    json!({"role": "system", "content": system_prompt}),
                    json!({"role": "user", "content": user_msg}),
                ];
                let fallback_result = crate::cli_llm::fallback_chat_completion(&messages).await?;
                let action_json = match recover_json(&fallback_result) {
                    Some(v) => v,
                    None => {
                        let fallback_action = self.fallback_vision_action(goal, action_history);
                        eprintln!(
                            "⚠️ [plan_next_step] Failed to parse fallback JSON; using deterministic fallback: {}",
                            fallback_action
                        );
                        fallback_action
                    }
                };
                return Ok(action_json);
            }
        };

        let content_opt = body["choices"][0]["message"]["content"].as_str();

        let content_str = match content_opt {
            Some(c) => c,
            None => {
                // Log the full body to debug "No content" error
                let body_str = serde_json::to_string_pretty(&body).unwrap_or_default();
                return Err(anyhow::anyhow!(
                    "No content in LLM response. Raw Body: {}",
                    body_str
                ));
            }
        };

        // Check for <think> block
        if let Some(start_think) = content_str.find("<think>") {
            if let Some(end_think) = content_str.find("</think>") {
                let thinking = &content_str[start_think + 7..end_think];
                log::info!("🧠 [Thinking]: {}", thinking.trim());
            }
        }

        let action_json = recover_json(content_str)
            .ok_or_else(|| anyhow::anyhow!("Failed to parse JSON action"))?;
        Ok(action_json)
    }

    /// Generic Chat Completion (for Architect/Chat features)
    /// Falls back to CLI LLM chain (Gemini→Codex→llama-server) on 429.
    async fn chat_completion(&self, messages: Vec<Value>) -> Result<String> {
        if Self::cli_first_enabled() {
            eprintln!(
                "ℹ️ [chat_completion] CLI-first mode enabled (STEER_LLM_PRIMARY={}).",
                Self::llm_primary_mode().unwrap_or_else(|| "cli".to_string())
            );
            return crate::cli_llm::fallback_chat_completion(&messages).await;
        }

        let body = json!({
            "model": self.model,
            "messages": messages
        });

        let res_json = match self
            .post_with_retry("https://api.openai.com/v1/chat/completions", &body)
            .await
        {
            Ok(r) => r,
            Err(e) => {
                eprintln!(
                    "⚠️ [chat_completion] OpenAI error/rate-limit: {}. Invoking fallback chain...",
                    e
                );
                return crate::cli_llm::fallback_chat_completion(&messages).await;
            }
        };

        let content = res_json["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("")
            .to_string();

        Ok(content)
    }

    /// Plan the next step using Vision (Screenshots) instead of DOM tree
    async fn plan_vision_step(
        &self,
        goal: &str,
        image_b64: &str,
        history: &[String],
    ) -> Result<Value> {
        // [MCP] Fetch available tools dynamically (Shared for both CLI and Cloud LLM)
        let mut mcp_tools_doc = "No MCP tools available.".to_string();
        if let Ok(guard) = mcp_client::get_mcp_registry() {
            if let Some(registry) = guard.as_ref() {
                let tools = registry.list_all_tools();
                if !tools.is_empty() {
                    let list: Vec<String> = tools
                        .iter()
                        .map(|(server, tool)| {
                            format!("- {}/{}: {}", server, tool.name, tool.description)
                        })
                        .collect();
                    mcp_tools_doc = list.join("\n");
                }
            }
        }

        // Try CLI LLM first if STEER_CLI_LLM is set (skip only when CLI can't use stdin and payload is large)
        if let Some(cli_client) = crate::cli_llm::CLILLMClient::from_env() {
            let mut final_image_b64 = image_b64.to_string();
            let cli_max_b64 = std::env::var("STEER_CLI_VISION_MAX_B64")
                .ok()
                .and_then(|s| s.parse::<usize>().ok())
                .filter(|v| *v >= 4_000)
                .unwrap_or(24_000);

            // Keep vision payload small enough for CLI model context.
            // Applies to all providers (including stdin-based Codex/Claude).
            if final_image_b64.len() > cli_max_b64 {
                println!(
                    "⚠️ [Vision] Image payload large for CLI context ({} bytes). Attempting to downscale...",
                    final_image_b64.len()
                );
                use base64::{engine::general_purpose, Engine as _};
                use std::io::Cursor;

                if let Ok(data) = general_purpose::STANDARD.decode(&final_image_b64) {
                    if let Ok(img) = image::load_from_memory(&data) {
                        let profiles: &[(u32, u32, u8)] = &[
                            (1024, 768, 60),
                            (800, 600, 50),
                            (640, 480, 42),
                            (512, 384, 36),
                            (384, 288, 30),
                            (320, 240, 28),
                        ];
                        for (w, h, q) in profiles {
                            let resized = img.resize(*w, *h, image::imageops::FilterType::Triangle);
                            let mut buffer = Cursor::new(Vec::new());
                            if resized
                                .write_to(&mut buffer, image::ImageOutputFormat::Jpeg(*q))
                                .is_ok()
                            {
                                let candidate_b64 =
                                    general_purpose::STANDARD.encode(buffer.get_ref());
                                if candidate_b64.len() < final_image_b64.len() {
                                    println!(
                                        "      📉 Downscaled: {} -> {} bytes ({}x{}, q={})",
                                        final_image_b64.len(),
                                        candidate_b64.len(),
                                        w,
                                        h,
                                        q
                                    );
                                    final_image_b64 = candidate_b64;
                                }
                                if final_image_b64.len() <= cli_max_b64 {
                                    break;
                                }
                            }
                        }
                    }
                }
            }

            if final_image_b64.len() > cli_max_b64 {
                println!(
                    "⚠️ [Vision] Image still too large for CLI context ({} > {} bytes). Routing to Cloud LLM.",
                    final_image_b64.len(),
                    cli_max_b64
                );
                // Fall through to OpenAI logic below
            } else {
                println!(
                    "ℹ️ [Vision] Using CLI LLM ({:?}) as primary...",
                    std::env::var("STEER_CLI_LLM")
                );

                let history_str = if history.is_empty() {
                    "None".to_string()
                } else {
                    history.join("\n- ")
                };

                // [MCP tools doc available from outer scope]

                let cli_prompt = format!(
                    r#"
You are a desktop automation agent. Look at the current screen and decide the next action.

GOAL: {}
HISTORY: {}

Respond with a JSON object containing your "thought" and the "action".
The "thought" should explain your visual observation and reasoning for the next step.
The "action" must be one of the allowed actions.

Available actions:
- Open an app: {{\"action\": \"open_app\", \"name\": \"Calculator\"}}
- Open URL: {{\"action\": \"open_url\", \"url\": \"https://google.com\"}}
- Type text: {{\"action\": \"type\", \"text\": \"hello\"}}
- Press key: {{\"action\": \"key\", \"key\": \"return\"}} (single key only)
- Shortcut: {{\"action\": \"shortcut\", \"key\": \"n\", \"modifiers\": [\"command\"]}}
- Click: {{\"action\": \"click_visual\", \"description\": \"search button\"}}
- Read screen text: {{\"action\": \"read\", \"query\": \"What is the number shown?\"}}
- Select text by search: {{\"action\": \"select_text\", \"text\": \"Rust programming\"}}
- Run shell command: {{\"action\": \"shell\", \"command\": \"ls -la\"}}
- Take UI snapshot: {{\"action\": \"snapshot\"}}
- Click by ref (after snapshot): {{\"action\": \"click_ref\", \"ref\": \"E5\"}}
- Spawn subagent: {{\"action\": \"spawn_agent\", \"name\": \"worker\", \"task\": \"do something\"}}
- Switch to app: {{\"action\": \"switch_app\", \"app\": \"Notes\"}}
- Copy to clipboard: {{\"action\": \"copy\", \"text\": \"data to copy\"}}
- Paste from clipboard: {{\"action\": \"paste\"}}
- Read clipboard: {{\"action\": \"read_clipboard\"}}
- Transfer between apps: {{\"action\": \"transfer\", \"from\": \"Calculator\", \"to\": \"Notes\"}}
- Call external service (MCP): {{\"action\": \"mcp\", \"server\": \"filesystem\", \"tool\": \"read_file\", \"arguments\": {{\"path\": \"/path/to/file\"}}}}
- List MCP tools: {{\"action\": \"mcp_list\"}}
- Done: {{\"action\": \"done\"}}

SNAPSHOT -> REF FLOW (IMPORTANT):
- If you need to click a specific UI element, prefer:
  1) {{\"action\": \"snapshot\"}} to get refs.
  2) Use an id from SNAPSHOT_REFS in HISTORY with {{\"action\": \"click_ref\", \"ref\": \"E5\"}}.
- If HISTORY contains SNAPSHOT_REFS, use click_ref and avoid click_visual unless no match exists.

AVAILABLE MCP TOOLS (Use 'mcp' action):
{}

Example Response:
{{
  \"thought\": \"I see the Calculator app is open but showing 0. The goal is 123+456. I need to type 1 first.\",
  \"action\": {{\"action\": \"type\", \"text\": \"1\"}}
}}

BROWSER NAVIGATION (IMPORTANT!):
Use `open_url` for navigation/search (reliable).
Use `shortcut` with `command+l` ONLY when you need to copy the current URL.
- GOOD (search): {{ "action": "open_url", "url": "https://google.com/search?q=query" }}
- GOOD (copy URL): {{ "action": "shortcut", "key": "l", "modifiers": ["command"] }} then {{ "action": "shortcut", "key": "c", "modifiers": ["command"] }}

CALCULATOR RULES (IMPORTANT!):
- ALWAYS perform the calculation explicitly; never read an existing value and assume it's correct.
- Use "*" for multiply and press "=" at the end (e.g., "365*24=").
- If you read a decimal like "259.48", type it EXACTLY (keep the decimal point).

TEXT SELECTION RULES:
- If the goal says select a specific substring (e.g., "Rust programming"), use {{ "action": "select_text", "text": "Rust programming" }} before copying.
- Do NOT press Cmd+C on a blank document or without a selection.

DIALOGS / POPUPS:
- If an "Open/Save" dialog appears, DO NOT try to click buttons.
- Use Escape ({{ "action": "key", "key": "escape" }}) or Cmd+W ({{ "action": "shortcut", "key": "w", "modifiers": ["command"] }}) to close it.


CROSS-APP WORKFLOW PATTERNS (IMPORTANT!):
When moving data between apps (e.g. "copy from Calculator to Notes"), the "transfer" action is STRONGLY RECOMMENDED.
It automatically handles: Switch App -> Select All -> Copy -> Switch Back -> Paste.

BEST WAY (Reliable):
1. Open source app and prepare data (e.g. calculate 100+200)
2. Call {{ "action": "transfer", "from": "SourceApp", "to": "TargetApp" }}
3. Done.

MCP RULE: ONLY use 'mcp' for Filesystem operations (finding/reading files).
NEVER use 'mcp' inside apps like Mail, Safari, or Discord. Use Visual Actions (click/read) instead.
IF HISTORY SHOWS AN MCP RESULT, do not repeat the call! Read the result.
NEVER output actions like "filesystem/..." directly; always use the 'mcp' action.

WEB-TO-APP PATTERN (E.g. Safari Price -> Excel):
Select All (Cmd+A) on a webpage is often bad.
Instead, use **VISUAL TRANSCRIPTION**:
1. Use `read` to extract the value from the screen (e.g., "1,250,000").
2. switch_app to Target (Excel).
3. Type the value manually: {{ "action": "type", "text": "1,250,000" }}

MANUAL WAY (If transfer fails):
1. open_app "Source" -> shortcut cmd+a -> shortcut cmd+c
2. switch_app "Target"
3. shortcut cmd+v

Example Using Transfer (Preferred):
{{
  "thought": "Calculation complete. Using transfer to move result to TextEdit safely.",
  "action": {{"action": "transfer", "from": "Calculator", "to": "TextEdit"}}
}}

Respond with ONE JSON object only.
"#,
                    goal, history_str, mcp_tools_doc
                );

                let parse_cli_action = |response: &str| -> Option<Value> {
                    let clean = response
                        .trim()
                        .trim_start_matches("```json")
                        .trim_start_matches("```")
                        .trim_end_matches("```")
                        .trim();

                    if let Some(start) = clean.find('{') {
                        if let Some(end) = clean.rfind('}') {
                            let json_str = &clean[start..=end];
                            if let Ok(json_val) = serde_json::from_str::<Value>(json_str) {
                                if let Some(thought) =
                                    json_val.get("thought").and_then(|t| t.as_str())
                                {
                                    log::info!("[Vision] 🤔 Thought: {}", thought);
                                }

                                if let Some(inner_action) = json_val.get("action") {
                                    if inner_action.is_object() {
                                        return Some(inner_action.clone());
                                    }
                                }

                                return Some(json_val);
                            }
                        }
                    }
                    None
                };

                // Use execute_with_vision to actually see the screen
                let primary_failed =
                    match cli_client.execute_with_vision(&final_image_b64, &cli_prompt) {
                        Ok(response) => {
                            let preview = response.chars().take(200).collect::<String>();
                            log::info!("[CLI LLM] Response: {}", preview);
                            if let Some(action) = parse_cli_action(&response) {
                                return Ok(action);
                            }
                            log::warn!("[CLI LLM] Could not parse JSON from primary provider.");
                            true
                        }
                        Err(e) => {
                            log::warn!("[CLI LLM] Failed: {}", e);
                            true
                        }
                    };

                // If Gemini fails, try Codex CLI before OpenAI fallback to avoid API hard-fail cascades.
                if primary_failed
                    && std::env::var("STEER_CLI_LLM_FAILOVER")
                        .map(|v| !matches!(v.as_str(), "0" | "false" | "FALSE" | "no" | "NO"))
                        .unwrap_or(true)
                    && std::env::var("STEER_CLI_LLM")
                        .map(|v| v.eq_ignore_ascii_case("gemini"))
                        .unwrap_or(false)
                {
                    println!("⚠️ [Vision] Gemini CLI failed; trying Codex CLI failover...");
                    let codex_client =
                        crate::cli_llm::CLILLMClient::new(crate::cli_llm::LLMProvider::Codex)
                            .with_cwd("/tmp");

                    match codex_client.execute_with_vision(&final_image_b64, &cli_prompt) {
                        Ok(response) => {
                            let preview = response.chars().take(200).collect::<String>();
                            log::info!("[CLI LLM:codex failover] Response: {}", preview);
                            if let Some(action) = parse_cli_action(&response) {
                                println!("✅ [Vision] Codex CLI failover succeeded.");
                                return Ok(action);
                            }
                            log::warn!(
                                "[CLI LLM:codex failover] Could not parse JSON, falling back to OpenAI"
                            );
                        }
                        Err(e) => {
                            log::warn!(
                                "[CLI LLM:codex failover] Failed: {}, falling back to OpenAI",
                                e
                            );
                        }
                    }
                }
            } // End else
        } // End if let Some

        let mut openai_image_b64 = image_b64.to_string();
        let openai_max_b64 = std::env::var("STEER_OPENAI_VISION_MAX_B64")
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .filter(|v| *v >= 4_000)
            .unwrap_or(8_000);

        if openai_image_b64.len() > openai_max_b64 {
            println!(
                "⚠️ [Vision] OpenAI payload large ({} bytes). Attempting to downscale...",
                openai_image_b64.len()
            );
            use base64::{engine::general_purpose, Engine as _};
            use std::io::Cursor;

            if let Ok(data) = general_purpose::STANDARD.decode(&openai_image_b64) {
                if let Ok(img) = image::load_from_memory(&data) {
                    let profiles: &[(u32, u32, u8)] = &[
                        (1024, 768, 60),
                        (800, 600, 50),
                        (640, 480, 42),
                        (512, 384, 36),
                        (384, 288, 30),
                        (320, 240, 28),
                        (256, 192, 24),
                        (224, 168, 22),
                        (192, 144, 20),
                        (160, 120, 18),
                        (128, 96, 16),
                    ];
                    for (w, h, q) in profiles {
                        let resized = img.resize(*w, *h, image::imageops::FilterType::Triangle);
                        let mut buffer = Cursor::new(Vec::new());
                        if resized
                            .write_to(&mut buffer, image::ImageOutputFormat::Jpeg(*q))
                            .is_ok()
                        {
                            let candidate_b64 = general_purpose::STANDARD.encode(buffer.get_ref());
                            if candidate_b64.len() < openai_image_b64.len() {
                                println!(
                                    "      📉 OpenAI downscaled: {} -> {} bytes ({}x{}, q={})",
                                    openai_image_b64.len(),
                                    candidate_b64.len(),
                                    w,
                                    h,
                                    q
                                );
                                openai_image_b64 = candidate_b64;
                            }
                            if openai_image_b64.len() <= openai_max_b64 {
                                break;
                            }
                        }
                    }
                }
            }
        }

        let minimal_prompt = std::env::var("STEER_VISION_PROMPT_MINIMAL")
            .ok()
            .map(|v| {
                matches!(
                    v.trim().to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes" | "on"
                )
            })
            .unwrap_or(false);
        let system_prompt = if minimal_prompt {
            format!(
                "You are a desktop automation agent.\n\
                 Goal: \"{}\"\n\
                 Decide ONLY the next single action from this set: \
                 click_visual, click_ref, type, shortcut, read, scroll, open_app, open_url, \
                 select_all, copy, paste, read_clipboard, done.\n\
                 IMPORTANT: open_app must include a non-empty name field.\n\
                 Example: {{\"action\":\"open_app\",\"name\":\"Calendar\"}}\n\
                 Return JSON object only with keys: thought, action.\n\
                 If goal is fully satisfied, return done.",
                goal
            )
        } else {
            crate::prompts::VISION_PLANNING_PROMPT
                .replace("{goal}", goal)
                .replace("{mcp_tools}", &mcp_tools_doc)
        };

        let history_str = if history.is_empty() {
            "None".to_string()
        } else if minimal_prompt {
            history
                .iter()
                .rev()
                .take(8)
                .cloned()
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect::<Vec<_>>()
                .join("\n- ")
        } else {
            history.join("\n- ")
        };
        let user_msg = format!("GOAL: {}\n\nHISTORY:\n- {}", goal, history_str);

        let vision_detail = std::env::var("STEER_OPENAI_VISION_DETAIL")
            .ok()
            .map(|v| v.trim().to_lowercase())
            .filter(|v| matches!(v.as_str(), "low" | "high" | "auto"))
            .unwrap_or_else(|| "low".to_string());
        let vision_max_tokens = std::env::var("STEER_VISION_MAX_TOKENS")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .filter(|v| *v >= 32 && *v <= 800)
            .unwrap_or(64);

        let vision_model = self.vision_model();
        let body = json!({
            "model": vision_model,
            "messages": [
                { "role": "system", "content": system_prompt },
                {
                    "role": "user",
                    "content": [
                        { "type": "text", "text": user_msg },
                        {
                            "type": "image_url",
                            "image_url": {
                                "url": format!("data:image/jpeg;base64,{}", openai_image_b64),
                                "detail": vision_detail
                            }
                        }
                    ]
                }
            ],
            "max_tokens": vision_max_tokens,
            "response_format": { "type": "json_object" }
        });

        let res_json = match self
            .post_with_retry("https://api.openai.com/v1/chat/completions", &body)
            .await
        {
            Ok(r) => r,
            Err(e) => {
                eprintln!("⚠️ [plan_vision_step] OpenAI error/rate-limited: {}. Invoking fallback chain (text-only)...", e);
                let text_messages = vec![
                    json!({"role": "system", "content": system_prompt}),
                    json!({"role": "user", "content": user_msg}),
                ];
                let fallback_result =
                    crate::cli_llm::fallback_chat_completion(&text_messages).await?;
                let action_json = match recover_json(&fallback_result) {
                    Some(v) => v,
                    None => {
                        let fallback_action = self.fallback_vision_action(goal, history);
                        eprintln!(
                            "⚠️ [plan_vision_step] Failed to parse fallback JSON; using deterministic fallback: {}",
                            fallback_action
                        );
                        fallback_action
                    }
                };
                return Ok(action_json);
            }
        };

        // Handle Refusal (Safety Filter) - Try CLI LLM Fallback
        if let Some(refusal) = res_json["choices"][0]["message"]["refusal"].as_str() {
            log::warn!(
                "OpenAI refused (Safety): {}. Trying CLI LLM fallback...",
                refusal
            );

            // Try CLI LLM if configured
            if let Some(cli_client) = crate::cli_llm::CLILLMClient::from_env() {
                let cli_prompt = format!(
                    "{}\n\nGOAL: {}\nHISTORY: {:?}\n\nRespond with JSON only: {{\"action\": \"...\", ...}}",
                    "You are a UI automation agent. Plan the next action to achieve the goal.",
                    goal,
                    history
                );

                match cli_client.execute(&cli_prompt) {
                    Ok(cli_response) => {
                        log::info!("CLI LLM fallback succeeded ({} chars)", cli_response.len());
                        // Try to parse as JSON
                        let clean = cli_response
                            .trim()
                            .trim_start_matches("```json")
                            .trim_start_matches("```")
                            .trim_end_matches("```");
                        if let Ok(action_json) = serde_json::from_str::<Value>(clean) {
                            return Ok(action_json);
                        }
                        // If not valid JSON, return a fail action
                        return Ok(
                            json!({"action": "fail", "reason": "CLI LLM response not valid JSON"}),
                        );
                    }
                    Err(e) => {
                        log::error!("CLI LLM fallback also failed: {}", e);
                    }
                }
            }

            if !crate::env_flag("STEER_ALLOW_REFUSAL_FALLBACK") {
                return Err(anyhow::anyhow!(
                    "OpenAI refusal with fallback disabled by policy: {}",
                    refusal
                ));
            }

            let fallback_action = self.fallback_vision_action(goal, history);
            log::warn!(
                "Using deterministic fallback action after refusal: {}",
                fallback_action
            );
            return Ok(fallback_action);
        }

        let content_opt = res_json["choices"][0]["message"]["content"].as_str();
        let content = match content_opt {
            Some(c) => c,
            None => {
                let body_str = serde_json::to_string_pretty(&res_json).unwrap_or_default();
                return Err(anyhow::anyhow!(
                    "No content in Vision LLM response. Raw Body: {}",
                    body_str
                ));
            }
        };

        // Sanitize content (sometimes it adds markdown code blocks)
        let clean_content = content
            .trim()
            .trim_start_matches("```json")
            .trim_start_matches("```")
            .trim_end_matches("```");

        let action_json: Value = serde_json::from_str(clean_content)?;
        Ok(action_json)
    }

    async fn analyze_routine(&self, logs: &[String]) -> Result<String> {
        if logs.is_empty() {
            return Ok("No data to analyze.".to_string());
        }

        // Summarize logs to avoid token limit
        // Simple strategy: take first 50 and last 50 events if too many
        let sample = if logs.len() > 100 {
            let mut s = logs[0..50].to_vec();
            s.extend_from_slice(&logs[logs.len() - 50..]);
            s
        } else {
            logs.to_vec()
        };

        let prompt = format!(
            "Analyze the following user activity logs (JSON) from the last 24 hours. \
            Identify any repeating patterns, routines, or habits. \
            Output a concise summary bullet list.\n\nLogs:\n{}",
            sample.join("\n")
        );

        let messages = vec![
            json!({
                "role": "system",
                "content": "You are a helpful assistant that analyzes user behavior patterns."
            }),
            json!({"role": "user", "content": prompt}),
        ];
        self.chat_completion(messages).await
    }

    async fn recommend_automation(&self, logs: &[String]) -> Result<String> {
        if logs.is_empty() {
            return Ok("No data to assist recommendation.".to_string());
        }

        // Limit logs
        let sample = if logs.len() > 150 {
            let mut s = logs[0..50].to_vec();
            s.extend_from_slice(&logs[logs.len() - 100..]); // Bias towards recent
            s
        } else {
            logs.to_vec()
        };

        let prompt = format!(
            "Based on the user behavior logs (JSON) below, identify a repetitive manual task that can be automated.\n\
            Then, generate a robust BASH SCRIPT (or Python) to automate it.\n\
            \n\
            Output Format:\n\
            ### Problem\n\
            (Description)\n\
            \n\
            ### Solution\n\
            ```bash\n\
            #!/bin/bash\n\
            (Code)\n\
            ```\n\
            \n\
            Logs:\n\
            {}",
            sample.join("\n")
        );

        let messages = vec![
            json!({
                "role": "system",
                "content": "You are a pragmatic automation engineer. You write safe, effective scripts."
            }),
            json!({"role": "user", "content": prompt}),
        ];
        self.chat_completion(messages).await
    }

    /// Build n8n workflow JSON from user prompt
    /// `context` can include: available integrations, user preferences, project-specific nodes
    async fn build_n8n_workflow(&self, user_prompt: &str) -> Result<String> {
        // Dynamically build context based on what's available
        let dynamic_context = self.get_workflow_context();

        let base_prompt = r##"
You are an expert n8n Workflow Architect. Generate VALID, EXECUTABLE n8n workflow JSON.

## CRITICAL RULES
1. Output ONLY raw JSON. NO markdown, NO explanations.
2. Every node MUST have: name, type, typeVersion, position, parameters
3. Connections MUST reference existing node names exactly
4. Use REAL n8n node types from the AVAILABLE NODES section
5. ROBUSTNESS MATTERS:
   - For risky nodes (HTTP, OS Control), ensure valid inputs.
   - If using 'executeCommand', favor commands that fail gracefully or are checked.

## NODE FORMAT
{
  "name": "Unique Node Name",
  "type": "n8n-nodes-base.httpRequest",
  "typeVersion": 1,
  "position": [X, Y],
  "parameters": { ... }
}

## CONNECTION FORMAT
{
  "Source Node Name": {
    "main": [
      [{ "node": "Target Node Name", "type": "main", "index": 0 }]
    ]
  }
}

"##;

        // Combine base prompt with dynamic context
        let system_prompt = format!(
            "{}\n{}\n\nNow generate a workflow for the user request. Output ONLY the JSON.",
            base_prompt, dynamic_context
        );

        let body = json!({
            "model": self.model,
            "messages": [
                {"role": "system", "content": system_prompt},
                {"role": "user", "content": user_prompt}
            ]
        });

        let content = match self
            .post_with_retry("https://api.openai.com/v1/chat/completions", &body)
            .await
        {
            Ok(res_json) => res_json["choices"][0]["message"]["content"]
                .as_str()
                .unwrap_or("{}")
                .to_string(),
            Err(e) => {
                eprintln!(
                    "⚠️ [build_n8n_workflow] OpenAI error: {}. Invoking fallback...",
                    e
                );
                let messages = vec![
                    json!({"role": "system", "content": system_prompt}),
                    json!({"role": "user", "content": user_prompt}),
                ];
                let fallback_res = crate::cli_llm::fallback_chat_completion(&messages).await?;
                let parsed: Value = recover_json(&fallback_res)
                    .ok_or_else(|| anyhow::anyhow!("Failed to parse JSON from fallback"))?;
                parsed.to_string()
            }
        };

        // Clean up markdown if model disobeys
        let clean_json = content
            .trim()
            .trim_start_matches("```json")
            .trim_start_matches("```")
            .trim_end_matches("```");

        Ok(clean_json.to_string())
    }

    async fn fix_n8n_workflow(
        &self,
        user_prompt: &str,
        bad_json: &str,
        error_msg: &str,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let system_prompt = format!(
            r##"
You are an expert n8n Workflow Architect.
You previously generated a workflow that FAILED to validate or execute.
Your goal is to FIX the JSON based on the error message.

## ORIGINAL REQUEST
{}

## ERROR MESSAGE
{}

## CRITICAL RULES (RE-EMPHASIZED)
1. Output ONLY raw JSON. NO markdown.
2. Check node types and version compatibility.
3. Verify connections reference exact node names.
4. Ensure all required parameters are present.

Now output the CORRECTED JSON.
"##,
            user_prompt, error_msg
        );

        let body = json!({
            "model": self.model,
            "messages": [
                {"role": "system", "content": system_prompt},
                {"role": "user", "content": bad_json}
            ]
        });

        let res_json = self
            .post_with_retry("https://api.openai.com/v1/chat/completions", &body)
            .await?;
        let content = res_json["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("{}")
            .to_string();

        let clean_json = content
            .trim()
            .trim_start_matches("```json")
            .trim_start_matches("```")
            .trim_end_matches("```");
        Ok(clean_json.to_string())
    }

    /// Analyze screen content using Vision API
    async fn analyze_screen(
        &self,
        prompt: &str,
        image_b64: &str,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let vision_model = self.vision_model();
        let body = json!({
            "model": vision_model,
            "messages": [
                {
                    "role": "user",
                    "content": [
                        { "type": "text", "text": prompt },
                        {
                            "type": "image_url",
                            "image_url": {
                                "url": format!("data:image/jpeg;base64,{}", image_b64)
                            }
                        }
                    ]
                }
            ],
            "max_tokens": 500
        });

        let res_json = self
            .post_with_retry("https://api.openai.com/v1/chat/completions", &body)
            .await?;

        if let Some(err) = res_json.get("error") {
            return Err(anyhow::anyhow!("OpenAI API Error: {:?}", err).into());
        }

        let content = res_json["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("")
            .to_string();

        Ok(content)
    }

    /// Find coordinates of a UI element using Vision API
    async fn find_element_coordinates(
        &self,
        element_description: &str,
        image_b64: &str,
    ) -> Result<Option<(i32, i32)>> {
        let system_prompt = r#"
        You are a Screen Coordinate Locator.
        Analyze the screenshot and find the generic center coordinates (x, y) of the UI element described by the user.
        
        Output JSON ONLY:
        {
          "thinking": "Briefly describe the element's location (e.g. 'Found Blue button in top right')",
          "found": true, 
          "x": 123, 
          "y": 456
        }
        or
        { "found": false }
        
        DO NOT output markdown.
        "#;

        let user_msg = format!("Find this element: {}", element_description);

        let vision_model = self.vision_model();
        let body = json!({
            "model": vision_model,
            "messages": [
                { "role": "system", "content": system_prompt },
                {
                    "role": "user",
                    "content": [
                        { "type": "text", "text": user_msg },
                        {
                            "type": "image_url",
                            "image_url": {
                                "url": format!("data:image/jpeg;base64,{}", image_b64)
                            }
                        }
                    ]
                }
            ],
            "max_tokens": 100,
            "response_format": { "type": "json_object" }
        });

        let res_json = self
            .post_with_retry("https://api.openai.com/v1/chat/completions", &body)
            .await?;
        let content = res_json["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("{}");

        let parsed: Value = serde_json::from_str(content)?;

        if parsed["found"].as_bool().unwrap_or(false) {
            let x = parsed["x"].as_i64().unwrap_or(0) as i32;
            let y = parsed["y"].as_i64().unwrap_or(0) as i32;
            Ok(Some((x, y)))
        } else {
            Ok(None)
        }
    }

    async fn score_quality(
        &self,
        system_prompt: &str,
        payload: &serde_json::Value,
    ) -> Result<String> {
        let body = json!({
            "model": self.model,
            "messages": [
                { "role": "system", "content": system_prompt },
                { "role": "user", "content": payload.to_string() }
            ],
            "temperature": 0.2,
            "response_format": { "type": "json_object" }
        });

        let body = self
            .post_with_retry("https://api.openai.com/v1/chat/completions", &body)
            .await?;
        let content = body["choices"][0]["message"]["content"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("No content in quality scoring response"))?;
        Ok(content.to_string())
    }

    async fn propose_workflow(
        &self,
        logs: &[String],
    ) -> Result<AutomationProposal, Box<dyn std::error::Error>> {
        if logs.is_empty() {
            return Ok(AutomationProposal::default());
        }

        let sample = if logs.len() > 200 {
            let mut s = logs[0..50].to_vec();
            s.extend_from_slice(&logs[logs.len() - 150..]);
            s
        } else {
            logs.to_vec()
        };

        let system_prompt = r#"
You are an expert Workflow Analyst for general office workers (Marketing, HR, Finance, Dev).
Your goal is to detect Repetitive Manual Work (Toil) from user logs and propose n8n automations.

## WHAT TO LOOK FOR (OFFICE PATTERNS)
1. "Copy-Paste Loops": User switches between Excel/Sheets and a Web Form (CRM, ERP) repeatedly.
2. "Notification Fatigue": User checks Email/Slack constantly for specific keywords (e.g., "Invoice", "Approve").
3. "File Shuffling": User downloads files (PDF/CSV) -> Renames them -> Uploads to Drive/Slack.
4. "Meeting Prep": User opens Calendar -> Opens Notion/Docs -> Copies attendees -> Writes agenda.

## OUTPUT JSON FORMAT
{
  "title": "Clear, Benefit-focused Title (e.g., 'Auto-Save Invoices to Drive')",
  "summary": "Explain the pain point and the solution (e.g., 'You check email for invoices 5 times a day. This workflow saves them to GDrive automatically.')",
  "trigger": "Trigger event (e.g., 'New Gmail with attachment')",
  "actions": ["Save to Drive", "Notify Slack", "Log to Sheet"],
  "confidence": 0.0 to 1.0 (High if pattern is clear and repetitive),
  "n8n_prompt": "Create a workflow that triggers on [Trigger], then [Action 1], then [Action 2]. Handle errors."
}

## GUIDELINES
- Avoid developer jargon if possible. Use "Save file" instead of "Binary Write".
- If logs show random browsing (YouTube, News), return confidence 0.0.
- If logs show repeated "Cmd+C" / "Cmd+V" sequences across apps, that is a HIGH confidence signal.
"#;

        let prompt = format!(
            "Logs:\n{}\n\nDecide if a workflow should be recommended.",
            sample.join("\n")
        );

        let body = json!({
            "model": self.model,
            "messages": [
                {"role": "system", "content": system_prompt},
                {"role": "user", "content": prompt}
            ],
            "temperature": 0.2,
            "response_format": { "type": "json_object" }
        });

        let res_json = match self
            .post_with_retry("https://api.openai.com/v1/chat/completions", &body)
            .await
        {
            Ok(b) => b,
            Err(e) => {
                eprintln!(
                    "⚠️ [propose_workflow] OpenAI error: {}. Invoking fallback...",
                    e
                );
                let messages = vec![
                    json!({"role": "system", "content": system_prompt}),
                    json!({"role": "user", "content": prompt}),
                ];
                let fallback_res = crate::cli_llm::fallback_chat_completion(&messages).await?;
                let parsed: Value = recover_json(&fallback_res)
                    .ok_or_else(|| anyhow::anyhow!("Failed to parse JSON from fallback"))?;

                // Wrap in fake response to match structure expected below
                json!({
                    "choices": [{
                        "message": { "content": serde_json::to_string(&parsed).unwrap() }
                    }]
                })
            }
        };
        let content = res_json["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("{}");

        let proposal: AutomationProposal = serde_json::from_str(content)?;
        Ok(proposal)
    }

    async fn analyze_tendency(&self, logs: &[String]) -> Result<String> {
        let system_prompt = r#"
You are a User Behavior Analyst. 
Analyze the following stream of user interaction logs (key presses, clicks, app focus).
Identify the user's current INTENT and TENDENCY.

Output specific, actionable intents like:
- "Writing code in Rust"
- "Debugging a Swift build error"
- "Searching for documentation on n8n"
- "Idle / Browsing social media"

If the user seems to be performing a repetitive manual task (e.g., copying data from PDF to Excel), HIGHLIGHT IT as a candidate for automation.

Output format: Just the intent description in 1-2 sentences.
"#;

        let log_text = logs.join("\n");
        let user_msg = format!("LOGS:\n{}", log_text);
        let messages = vec![
            json!({"role": "system", "content": system_prompt}),
            json!({"role": "user", "content": user_msg}),
        ];
        self.chat_completion(messages).await
    }

    /// Parse natural language input into a structured command
    #[allow(dead_code)]
    async fn parse_intent(&self, user_input: &str) -> Result<Value> {
        self.parse_intent_with_history(user_input, &[]).await
    }

    async fn parse_intent_with_history(
        &self,
        user_input: &str,
        history: &[crate::db::ChatMessage],
    ) -> Result<Value> {
        let system_prompt = r#"
You are a command parser for a Local OS Agent. Convert natural language into structured commands.

Available commands:
- gmail_list: List recent emails. Params: count (number, default 5)
- gmail_read: Read a specific email. Params: id (string)
- gmail_send: Send email. Params: to, subject, body
- calendar_today: Show today's events. No params.
- calendar_week: Show this week's events. No params.
- calendar_add: Add calendar event. Params: title, start, end
- telegram_send: Send telegram message. Params: message
- notion_create: Create notion page. Params: title, content
- build_workflow: Create n8n automation. Params: description
- create_routine: Schedule recurring task. Params: cron (CRON format e.g., '0 9 * * *'), prompt (instruction), name (short title)
- system_status: Show system status. No params.
- help: Show help. No params.
- unknown: Cannot parse. Params: original_text

Return JSON only:
{
  "command": "command_name",
  "params": { ... },
  "confidence": 0.0-1.0
}
"#;

        // Construct messages array with history (pruned)
        let mut messages = Vec::new();
        messages.push(json!({ "role": "system", "content": system_prompt }));

        let pruned_history = context_pruning::prune_chat_history(history);

        // Add history (already pruned by TTL/max count)
        for msg in pruned_history.iter() {
            // Map 'role' to OpenAI roles
            let role = if msg.role == "user" {
                "user"
            } else {
                "assistant"
            };
            messages.push(json!({ "role": role, "content": msg.content }));
        }

        // Add current user message
        messages.push(json!({ "role": "user", "content": user_input }));

        if Self::cli_first_enabled() {
            let fallback_res = crate::cli_llm::fallback_chat_completion(&messages).await?;
            let parsed = recover_json(&fallback_res)
                .ok_or_else(|| anyhow::anyhow!("Failed to parse JSON from CLI-first fallback"))?;
            return Ok(parsed);
        }

        let request_body = json!({
            "model": self.model,
            "messages": messages.clone(),
            "temperature": 0.1,
            "response_format": { "type": "json_object" }
        });

        // Parse intent fallback
        let body: Value = match self
            .post_with_retry("https://api.openai.com/v1/chat/completions", &request_body)
            .await
        {
            Ok(b) => b,
            Err(e) => {
                eprintln!(
                    "⚠️ [parse_intent] OpenAI error: {}. Invoking fallback...",
                    e
                );
                // Fallback
                let fallback_res = crate::cli_llm::fallback_chat_completion(&messages).await?;
                let parsed = recover_json(&fallback_res)
                    .ok_or_else(|| anyhow::anyhow!("Failed to parse JSON from fallback"))?;
                return Ok(parsed);
            }
        };

        let content = body["choices"][0]["message"]["content"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("No content"))?;

        let parsed: Value = serde_json::from_str(content)?;
        Ok(parsed)
    }

    /// Generate embeddings for RAG
    async fn get_embedding(&self, text: &str) -> Result<Vec<f32>> {
        let request_body = json!({
            "model": "text-embedding-3-small",
            "input": text,
            //"dimensions": 1536 // Default
        });

        let body: Value = self
            .post_with_retry("https://api.openai.com/v1/embeddings", &request_body)
            .await?;

        let vector = body["data"][0]["embedding"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("Invalid embedding response"))?
            .iter()
            .map(|v| v.as_f64().unwrap_or(0.0) as f32)
            .collect();

        Ok(vector)
    }

    /// Generate workflow recommendation from detected pattern
    async fn generate_recommendation_from_pattern(
        &self,
        pattern_description: &str,
        sample_events: &[String],
    ) -> Result<AutomationProposal> {
        let system_prompt = r#"
You are a workflow automation expert. Based on the detected user behavior pattern, generate a workflow automation recommendation.

Output JSON schema:
{
  "title": "Short, descriptive title in Korean",
  "summary": "1-2 sentence description of what this automation does",
  "trigger": "What triggers this workflow (e.g., 'Gmail 새 이메일 도착', 'Downloads 폴더에 파일 생성')",
  "actions": ["Action 1", "Action 2", ...],
  "n8n_prompt": "Description for n8n workflow generation",
  "confidence": 0.0-1.0 (how confident you are this is useful)
}

Guidelines:
- Focus on practical, useful automations
- Keep it simple - 2-3 actions max
- Use Korean for user-facing text
- Set confidence low (< 0.7) if pattern seems random or not automatable
"#;

        let samples_str = sample_events
            .iter()
            .take(3)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");
        let user_msg = format!(
            "Pattern detected: {}\n\nSample events:\n{}",
            pattern_description, samples_str
        );

        let messages = vec![
            json!({ "role": "system", "content": system_prompt }),
            json!({ "role": "user", "content": user_msg }),
        ];

        if Self::cli_first_enabled() {
            let fallback_res = crate::cli_llm::fallback_chat_completion(&messages).await?;
            let parsed: Value = recover_json(&fallback_res)
                .ok_or_else(|| anyhow::anyhow!("Failed to recover JSON from CLI-first fallback"))?;
            return Ok(AutomationProposal {
                title: parsed["title"]
                    .as_str()
                    .unwrap_or("Unnamed Workflow")
                    .to_string(),
                summary: parsed["summary"].as_str().unwrap_or("").to_string(),
                trigger: parsed["trigger"].as_str().unwrap_or("manual").to_string(),
                actions: parsed["actions"]
                    .as_array()
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(|s| s.to_string()))
                            .collect()
                    })
                    .unwrap_or_default(),
                n8n_prompt: parsed["n8n_prompt"].as_str().unwrap_or("").to_string(),
                confidence: parsed["confidence"].as_f64().unwrap_or(0.5),
                evidence: vec![],
                pattern_id: None,
            });
        }

        let request_body = json!({
            "model": self.model,
            "messages": messages.clone(),
            "temperature": 0.3,
            "response_format": { "type": "json_object" }
        });

        let body: Value = match self
            .post_with_retry("https://api.openai.com/v1/chat/completions", &request_body)
            .await
        {
            Ok(b) => b,
            Err(e) => {
                eprintln!(
                    "⚠️ [generate_recommendation] OpenAI error: {}. Invoking fallback...",
                    e
                );
                let fallback_res = crate::cli_llm::fallback_chat_completion(&messages).await?;
                let parsed: Value = recover_json(&fallback_res)
                    .ok_or_else(|| anyhow::anyhow!("Failed to recover JSON from fallback"))?;

                json!({
                    "choices": [{
                        "message": {
                            "content": serde_json::to_string(&parsed).unwrap()
                        }
                    }]
                })
            }
        };

        let content = body["choices"][0]["message"]["content"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("No content"))?;

        let parsed: Value = serde_json::from_str(content)?;

        Ok(AutomationProposal {
            title: parsed["title"]
                .as_str()
                .unwrap_or("Unnamed Workflow")
                .to_string(),
            summary: parsed["summary"].as_str().unwrap_or("").to_string(),
            trigger: parsed["trigger"].as_str().unwrap_or("manual").to_string(),
            actions: parsed["actions"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default(),
            n8n_prompt: parsed["n8n_prompt"].as_str().unwrap_or("").to_string(),
            confidence: parsed["confidence"].as_f64().unwrap_or(0.5),
            evidence: vec![], // Populated by caller
            pattern_id: None, // Populated by caller
        })
    }

    /// Proactively suggest a tech stack or approach for a goal (Transformers7 feature)
    #[allow(dead_code)]
    async fn propose_solution_stack(&self, goal: &str) -> Result<Value> {
        let prompt = format!(
            "Analyze the goal and recommend a technical solution stack.\n\
            GOAL: {}\n\
            \n\
            Output JSON:\n\
            {{\n\
                \"recommended\": \"Primary Tech Stack (e.g. React + FastAPI)\",\n\
                \"alternatives\": [\"Option 2\", \"Option 3\"],\n\
                \"reasoning\": \"Why this stack is best for this goal\"\n\
            }}",
            goal
        );

        let body = json!({
            "model": self.model,
            "messages": [
                { "role": "system", "content": "You are a Solution Architect. Propose the best stack for the user's goal." },
                { "role": "user", "content": prompt }
            ],
            "response_format": { "type": "json_object" }
        });

        let res_json = self
            .post_with_retry("https://api.openai.com/v1/chat/completions", &body)
            .await?;

        let content = res_json["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("{}");

        let parsed: Value = serde_json::from_str(content)?;
        Ok(parsed)
    }

    /// Run inference on Local LLM (Ollama)
    #[allow(dead_code)]
    async fn inference_local(&self, prompt: &str, model: Option<&str>) -> Result<String> {
        let model_name = model.unwrap_or("llama3"); // Default to llama3 or user pref

        let body = json!({
            "model": model_name,
            "prompt": prompt,
            "stream": false
        });

        // Default Ollama local URL
        let url = "http://localhost:11434/api/generate";

        let res = self.client.post(url).json(&body).send().await;

        match res {
            Ok(response) => {
                if !response.status().is_success() {
                    let err_text = response.text().await.unwrap_or_default();
                    return Err(anyhow::anyhow!("Ollama API Error: {}", err_text));
                }

                let val: Value = response.json().await?;
                let content = val["response"].as_str().unwrap_or("").to_string();
                Ok(content)
            }
            Err(e) => {
                // Connecting to localhost might fail if Ollama is not running.
                Err(anyhow::anyhow!(
                    "Failed to connect to Local LLM (Ollama): {}",
                    e
                ))
            }
        }
    }

    /// Smart Router: Decide between Cloud (OpenAI) and Local (Ollama)
    /// Returns: (use_local: bool, model_name: &str)
    fn route_task(&self, task_description: &str, pii_detected: bool) -> (bool, String) {
        // Rule 1: Privacy First
        if pii_detected {
            return (true, "llama3".to_string());
        }

        // Rule 2: Complexity Check (Simple Heuristic for Phase 4)
        // If task is short/simple -> Local
        // If task implies deep reasoning ("Plan", "Analyze", "Code") -> Cloud
        let lower = task_description.to_lowercase();
        if lower.contains("plan")
            || lower.contains("analyze")
            || lower.contains("code")
            || lower.contains("debug")
        {
            return (false, self.model.clone());
        }

        // Default to Local for simple chat/summarization to save cost
        (true, "llama3".to_string())
    }

    async fn analyze_user_feedback(
        &self,
        feedback: &str,
        history_summary: &str,
    ) -> Result<FeedbackAnalysis> {
        let system_prompt = r#"
You are a product assistant. Analyze user feedback and decide whether to refine the goal.
Output JSON:
{
  "action": "refine" | "complete",
  "new_goal": "..." // only when action=refine
}
Guidelines:
- If feedback requests changes, clarify or adjust goal -> action=refine.
- If feedback says it's good or done -> action=complete.
- Keep new_goal short and concrete.
"#;

        let user_msg = format!("History: {}\nUser feedback: {}", history_summary, feedback);

        let request_body = json!({
            "model": self.model,
            "messages": [
                { "role": "system", "content": system_prompt },
                { "role": "user", "content": user_msg }
            ],
            "temperature": 0.2,
            "response_format": { "type": "json_object" }
        });

        let response = self
            .client
            .post("https://api.openai.com/v1/chat/completions")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&request_body)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            return Err(anyhow::anyhow!("Feedback analysis error: {}", error_text));
        }

        let body: Value = response.json().await?;
        let content = body["choices"][0]["message"]["content"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("No content"))?;
        let parsed: Value = serde_json::from_str(content)?;

        let action = parsed["action"].as_str().unwrap_or("complete").to_string();
        let new_goal = parsed["new_goal"].as_str().map(|s| s.to_string());

        Ok(FeedbackAnalysis { action, new_goal })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedbackAnalysis {
    pub action: String,
    pub new_goal: Option<String>,
}

/// [Phase 28] Streaming Chat Completion
/// Returns chunks via callback for real-time UI updates
impl OpenAILLMClient {
    pub async fn chat_completion_stream<F>(
        &self,
        messages: Vec<Value>,
        mut on_chunk: F,
    ) -> Result<String>
    where
        F: FnMut(&str) + Send,
    {
        use futures::StreamExt;

        let body = json!({
            "model": self.model,
            "messages": messages,
            "stream": true
        });

        let response: reqwest::Response = self
            .client
            .post("https://api.openai.com/v1/chat/completions")
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            return Err(anyhow::anyhow!("Stream API Error: {}", error_text));
        }

        let mut full_content = String::new();
        let mut stream = response.bytes_stream();

        while let Some(chunk_result) = stream.next().await {
            let chunk = chunk_result?;
            let chunk_str = String::from_utf8_lossy(&chunk);

            // Parse SSE lines
            for line in chunk_str.lines() {
                if let Some(data) = line.strip_prefix("data: ") {
                    if data == "[DONE]" {
                        break;
                    }
                    if let Ok(parsed) = serde_json::from_str::<Value>(data) {
                        if let Some(delta) = parsed["choices"][0]["delta"]["content"].as_str() {
                            full_content.push_str(delta);
                            on_chunk(delta);
                        }
                    }
                }
            }
        }

        Ok(full_content)
    }
}
