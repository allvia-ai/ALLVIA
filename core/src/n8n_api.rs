#![allow(dead_code)]
use anyhow::Result;
use reqwest::{Client, Response, StatusCode};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowStatus {
    pub id: String,
    pub name: String,
    pub active: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionResult {
    pub id: String,
    pub finished: bool,
    pub status: String,
    pub started_at: String,
    pub stopped_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Credential {
    pub id: String,
    pub name: String,
    pub type_name: String,
}

#[allow(dead_code)]
pub struct N8nApi {
    base_url: String,
    api_key: String,
    client: Client,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum N8nRuntime {
    Docker,
    Npx,
    Manual,
}

impl N8nRuntime {
    fn from_env() -> Self {
        let raw = std::env::var("STEER_N8N_RUNTIME")
            .unwrap_or_else(|_| "manual".to_string())
            .trim()
            .to_lowercase();
        let test_context = parse_bool_env_with_default("STEER_TEST_MODE", false)
            || parse_bool_env_with_default("CI", false);
        match raw.as_str() {
            "npx" => {
                if !parse_bool_env_with_default("STEER_N8N_ENABLE_NPX_RUNTIME", false) {
                    eprintln!(
                        "⚠️ STEER_N8N_RUNTIME=npx ignored: set STEER_N8N_ENABLE_NPX_RUNTIME=1 to opt in."
                    );
                    return Self::Manual;
                }
                if !test_context
                    && !parse_bool_env_with_default("STEER_N8N_ALLOW_NPX_NON_TEST", false)
                {
                    eprintln!(
                        "⚠️ STEER_N8N_RUNTIME=npx ignored outside test mode. \
Set STEER_N8N_ALLOW_NPX_NON_TEST=1 to force npx runtime."
                    );
                    return Self::Manual;
                }
                Self::Npx
            }
            "manual" | "none" => Self::Manual,
            _ => Self::Docker,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Docker => "docker",
            Self::Npx => "npx",
            Self::Manual => "manual",
        }
    }
}

fn parse_bool_env_with_default(key: &str, default: bool) -> bool {
    std::env::var(key)
        .ok()
        .map(|v| {
            matches!(
                v.trim().to_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(default)
}

fn parse_u32_env_with_default(key: &str, default: u32, min: u32, max: u32) -> u32 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.trim().parse::<u32>().ok())
        .map(|v| v.clamp(min, max))
        .unwrap_or(default.clamp(min, max))
}

fn parse_u64_env_with_default(key: &str, default: u64, min: u64, max: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .map(|v| v.clamp(min, max))
        .unwrap_or(default.clamp(min, max))
}

fn slugify_for_path(input: &str) -> String {
    let mut out = String::new();
    let mut last_dash = false;
    for ch in input.chars() {
        let normalized = ch.to_ascii_lowercase();
        if normalized.is_ascii_alphanumeric() {
            out.push(normalized);
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
        if out.len() >= 48 {
            break;
        }
    }
    let trimmed = out.trim_matches('-');
    if trimmed.is_empty() {
        "steer-workflow".to_string()
    } else {
        trimmed.to_string()
    }
}

fn compact_prompt_seed(prompt: Option<&str>) -> String {
    prompt
        .unwrap_or_default()
        .trim()
        .replace('\n', " ")
        .chars()
        .take(240)
        .collect()
}

fn workflow_is_too_simple(value: &Value) -> bool {
    let nodes = match value.get("nodes").and_then(|n| n.as_array()) {
        Some(v) if !v.is_empty() => v,
        _ => return true,
    };

    let min_nodes = std::env::var("STEER_N8N_MIN_NODE_COUNT")
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .map(|v| v.clamp(6, 30))
        .unwrap_or(8);

    if nodes.len() < min_nodes {
        return true;
    }

    let mut has_trigger = false;
    let mut has_transform = false;
    let mut has_validation = false;
    let mut has_branch = false;
    let mut has_error_path = false;
    let mut has_observability = false;
    let mut has_external_io = false;

    for node in nodes {
        let Some(raw_ty) = node.get("type").and_then(|v| v.as_str()) else {
            continue;
        };
        let ty = raw_ty.to_ascii_lowercase();
        let node_name = node
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_ascii_lowercase();

        match ty.as_str() {
            "n8n-nodes-base.manualtrigger" | "n8n-nodes-base.webhook" => has_trigger = true,
            "n8n-nodes-base.set"
            | "n8n-nodes-base.code"
            | "n8n-nodes-base.function"
            | "n8n-nodes-base.functionitem"
            | "n8n-nodes-base.itemlists" => has_transform = true,
            "n8n-nodes-base.if" | "n8n-nodes-base.switch" => {
                has_branch = true;
                if node_name.contains("error") || node_name.contains("fail") {
                    has_error_path = true;
                }
            }
            "n8n-nodes-base.httprequest"
            | "n8n-nodes-base.notion"
            | "n8n-nodes-base.telegram"
            | "n8n-nodes-base.emailsend"
            | "n8n-nodes-base.slack" => has_external_io = true,
            _ => {}
        }

        if node_name.contains("validat") || node_name.contains("schema") {
            has_validation = true;
        }
        if node_name.contains("observability")
            || node_name.contains("metric")
            || node_name.contains("telemetry")
            || node_name.contains("log")
        {
            has_observability = true;
        }
        if node_name.contains("error") || node_name.contains("fallback") {
            has_error_path = true;
        }
    }

    !(has_trigger
        && has_transform
        && has_validation
        && has_branch
        && has_error_path
        && has_observability
        && has_external_io)
}

pub fn normalize_workflow_for_create(name: &str, workflow_json: &Value) -> Result<Value> {
    let mut normalized = workflow_json.clone();
    let allow_fallback =
        parse_bool_env_with_default("STEER_N8N_ALLOW_SIMPLE_WORKFLOW_FALLBACK", true);

    let is_empty = normalized
        .get("nodes")
        .and_then(|n| n.as_array())
        .map(|arr| arr.is_empty())
        .unwrap_or(true);

    if is_empty {
        if allow_fallback {
            println!("⚠️ Workflow nodes empty. Falling back to orchestrator workflow template.");
            normalized = build_orchestrator_fallback_workflow(name, None, "empty_nodes");
        } else {
            return Err(anyhow::anyhow!(
                "workflow validation failed: nodes are empty. \
Set STEER_N8N_ALLOW_SIMPLE_WORKFLOW_FALLBACK=1 to allow orchestrator fallback."
            ));
        }
    } else if workflow_is_too_simple(&normalized) {
        if allow_fallback {
            println!(
                "⚠️ Workflow nodes too simple. Replacing with orchestrator workflow template."
            );
            normalized = build_orchestrator_fallback_workflow(name, None, "too_simple_nodes");
        } else {
            return Err(anyhow::anyhow!(
                "workflow validation failed: nodes are too simple. \
Set STEER_N8N_ALLOW_SIMPLE_WORKFLOW_FALLBACK=1 to allow orchestrator fallback."
            ));
        }
    }

    if let Some(nodes) = normalized.get_mut("nodes").and_then(|n| n.as_array_mut()) {
        for node in nodes.iter_mut() {
            let is_webhook = node
                .get("type")
                .and_then(|v| v.as_str())
                .map(|t| t == "n8n-nodes-base.webhook")
                .unwrap_or(false);
            if !is_webhook {
                continue;
            }
            if let Some(node_obj) = node.as_object_mut() {
                let missing_webhook_id = node_obj
                    .get("webhookId")
                    .and_then(|v| v.as_str())
                    .map(|v| v.trim().is_empty())
                    .unwrap_or(true);
                if missing_webhook_id {
                    node_obj.insert(
                        "webhookId".to_string(),
                        Value::String(uuid::Uuid::new_v4().to_string()),
                    );
                }
                if node_obj
                    .get("disabled")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
                {
                    node_obj.insert("disabled".to_string(), Value::Bool(false));
                }
            }
        }
    }

    Ok(normalized)
}

pub fn build_orchestrator_fallback_workflow(
    name: &str,
    prompt: Option<&str>,
    reason: &str,
) -> Value {
    let webhook_path = format!("steer-{}", slugify_for_path(name));
    let prompt_seed = compact_prompt_seed(prompt);
    let env_or_empty = |keys: &[&str]| -> String {
        keys.iter()
            .filter_map(|key| std::env::var(key).ok())
            .map(|v| v.trim().to_string())
            .find(|v| !v.is_empty())
            .unwrap_or_default()
    };
    let notion_parent_default = env_or_empty(&["NOTION_PAGE_ID"]);
    let notion_token_default = env_or_empty(&["NOTION_TOKEN", "NOTION_API_KEY"]);
    let telegram_chat_default = env_or_empty(&["TELEGRAM_CHAT_ID"]);
    let telegram_bot_default = env_or_empty(&["TELEGRAM_BOT_TOKEN", "TELEGRAM_ACCESS_TOKEN"]);
    let notion_parent_default_json =
        serde_json::to_string(&notion_parent_default).unwrap_or_else(|_| "\"\"".to_string());
    let notion_token_default_json =
        serde_json::to_string(&notion_token_default).unwrap_or_else(|_| "\"\"".to_string());
    let telegram_chat_default_json =
        serde_json::to_string(&telegram_chat_default).unwrap_or_else(|_| "\"\"".to_string());
    let telegram_bot_default_json =
        serde_json::to_string(&telegram_bot_default).unwrap_or_else(|_| "\"\"".to_string());
    let notion_parent_expr =
        "={{$json.body && $json.body.notion_parent_page_id ? $json.body.notion_parent_page_id : ($json.notion_parent_page_id || __DEFAULT__)}}"
            .replace("__DEFAULT__", &notion_parent_default_json);
    let notion_token_expr =
        "={{$json.body && $json.body.notion_token ? $json.body.notion_token : ($json.notion_token || __DEFAULT__)}}"
            .replace("__DEFAULT__", &notion_token_default_json);
    let telegram_chat_expr =
        "={{$json.body && $json.body.telegram_chat_id ? $json.body.telegram_chat_id : ($json.telegram_chat_id || __DEFAULT__)}}"
            .replace("__DEFAULT__", &telegram_chat_default_json);
    let telegram_bot_expr =
        "={{$json.body && $json.body.telegram_bot_token ? $json.body.telegram_bot_token : ($json.telegram_bot_token || __DEFAULT__)}}"
            .replace("__DEFAULT__", &telegram_bot_default_json);
    let idempotency_code = r#"return items.map((item, index) => {
  const base = `${item.json.request_title || ""}::${item.json.request_text || ""}::${item.json.topic || ""}::${item.json.timeframe || ""}`;
  const safeBase64 = Buffer.from(base).toString("base64").replace(/[^a-zA-Z0-9]/g, "").slice(0, 32);
  return {
    json: {
      ...item.json,
      idempotency_key: `wf_${safeBase64 || "default"}_${index}`,
      run_started_at: new Date().toISOString(),
      pipeline_contract: "trigger -> fetch -> classify -> summarize -> validate -> publish -> observe"
    }
  };
});"#;
    let extract_code = r#"const fetchPayload = items[0]?.json || {};
const envelope = ($items("Idempotency Guard", 0, 0)?.[0]?.json) || {};
const topN = Math.max(1, Math.min(10, Number(envelope.top_n || 5)));
const hits =
  Array.isArray(fetchPayload.hits) ? fetchPayload.hits :
  Array.isArray(fetchPayload.body?.hits) ? fetchPayload.body.hits :
  [];
const articles = hits
  .map((hit) => ({
    title: String(hit.title || hit.story_title || "").trim(),
    source_url: String(hit.url || hit.story_url || "").trim(),
    published_at: hit.created_at || "",
    source: "hn.algolia",
  }))
  .filter((a) => a.title && a.source_url)
  .slice(0, topN);
return [{
  json: {
    ...envelope,
    request_text: String(envelope.request_text || envelope.request_seed || "").trim(),
    topic: String(envelope.topic || "AI").trim() || "AI",
    notion_token: String(envelope.notion_token || "").trim(),
    notion_parent_page_id: String(envelope.notion_parent_page_id || "").trim(),
    telegram_chat_id: String(envelope.telegram_chat_id || "").trim(),
    telegram_bot_token: String(envelope.telegram_bot_token || "").trim(),
    has_notion_config:
      String(envelope.notion_token || "").trim() && String(envelope.notion_parent_page_id || "").trim()
        ? "true"
        : "false",
    has_telegram_config:
      String(envelope.telegram_bot_token || "").trim() && String(envelope.telegram_chat_id || "").trim()
        ? "true"
        : "false",
    fetch_ok: articles.length >= topN,
    article_count: articles.length,
    articles,
    source_provider: "hn.algolia.search_by_date"
  }
}];"#;
    let classify_code = r#"const root = items[0]?.json || {};
const classifyStack = (title) => {
  const t = String(title || "").toLowerCase();
  const stacks = [];
  if (/(gpu|cuda|nvidia|chip|hardware|accelerator)/.test(t)) stacks.push("GPU/Accelerator");
  if (/(llm|model|transformer|agent)/.test(t)) stacks.push("LLM/Model Serving");
  if (/(vector|rag|embedding|retrieval)/.test(t)) stacks.push("RAG/Retrieval");
  if (/(api|sdk|platform|cloud)/.test(t)) stacks.push("API/Cloud Platform");
  if (/(security|privacy|governance|policy)/.test(t)) stacks.push("Security/Governance");
  return stacks.length ? stacks : ["General AI Application"];
};
const enriched = (root.articles || []).map((article) => ({
  ...article,
  trend_alignment: "current_trend",
  technical_stack: classifyStack(article.title),
}));
return [{ json: { ...root, articles: enriched } }];"#;
    let summarize_code = r#"const root = items[0]?.json || {};
const summarized = (root.articles || []).map((article, idx) => {
  const prevParadigm = "Rule-based/static automation";
  const newParadigm = "LLM-native adaptive workflows";
  const diff = `${article.title}: ${prevParadigm} -> ${newParadigm}`;
  return {
    ...article,
    comparative_summary: {
      previous_paradigm: prevParadigm,
      new_paradigm: newParadigm,
      key_difference: diff
    },
    importance: `Why it matters #${idx + 1}: impacts tooling, deployment, and developer workflow.`
  };
});
return [{ json: { ...root, articles: summarized } }];"#;
    let validate_code = r#"const root = items[0]?.json || {};
const errors = [];
if (!Array.isArray(root.articles) || root.articles.length < 5) {
  errors.push(`expected >=5 articles, got ${Array.isArray(root.articles) ? root.articles.length : 0}`);
}
for (const [idx, article] of (root.articles || []).entries()) {
  if (!article.title) errors.push(`article[${idx}] missing title`);
  if (!article.source_url) errors.push(`article[${idx}] missing source_url`);
  if (!Array.isArray(article.technical_stack) || article.technical_stack.length === 0) {
    errors.push(`article[${idx}] missing technical_stack`);
  }
}
return [{
  json: {
    ...root,
    validation_ok: errors.length === 0 ? "true" : "false",
    validation_errors: errors
  }
}];"#;
    let markdown_code = r#"const root = items[0]?.json || {};
const articles = Array.isArray(root.articles) ? root.articles : [];
const lines = [];
const notionChildren = [];
const nowIso = new Date().toISOString();
const digestDate = nowIso.slice(0, 10);
const topicLabel = String(root.topic || "AI").trim() || "AI";
const toText = (content, url = "") => {
  const text = { content: String(content || "").slice(0, 1800) };
  if (url) {
    text.link = { url };
  }
  return { type: "text", text };
};

lines.push(`# ${root.request_title || "AllvIa AI Trend Digest"}`);
lines.push(`Generated: ${nowIso}`);
lines.push("");
notionChildren.push({
  object: "block",
  type: "heading_1",
  heading_1: { rich_text: [toText(`AllvIa AI Trend Digest (${digestDate})`)] }
});
notionChildren.push({
  object: "block",
  type: "paragraph",
  paragraph: { rich_text: [toText(`주제: ${topicLabel}`)] }
});

for (const [idx, article] of articles.entries()) {
  lines.push(`## ${idx + 1}. [${article.title}](${article.source_url})`);
  lines.push(`- Source: ${article.source_url}`);
  lines.push(`- Published: ${article.published_at || "n/a"}`);
  lines.push(`- Technical Stack: ${(article.technical_stack || []).join(", ")}`);
  lines.push(`- Previous Paradigm: ${article.comparative_summary?.previous_paradigm || "n/a"}`);
  lines.push(`- New Paradigm: ${article.comparative_summary?.new_paradigm || "n/a"}`);
  lines.push(`- Difference: ${article.comparative_summary?.key_difference || "n/a"}`);
  lines.push(`- Why It Matters: ${article.importance || "n/a"}`);
  lines.push("");

  notionChildren.push({
    object: "block",
    type: "heading_2",
    heading_2: { rich_text: [toText(`${idx + 1}. ${article.title || "Untitled"}`)] }
  });
  notionChildren.push({
    object: "block",
    type: "paragraph",
    paragraph: {
      rich_text: [
        toText("원문 링크: "),
        toText("기사 원문 열기", String(article.source_url || ""))
      ]
    }
  });
  notionChildren.push({
    object: "block",
    type: "bulleted_list_item",
    bulleted_list_item: {
      rich_text: [toText(`핵심: ${article.importance || "n/a"}`)]
    }
  });
  notionChildren.push({
    object: "block",
    type: "bulleted_list_item",
    bulleted_list_item: {
      rich_text: [toText(`차이점: ${article.comparative_summary?.key_difference || "n/a"}`)]
    }
  });
  if (idx < articles.length - 1) {
    notionChildren.push({ object: "block", type: "divider", divider: {} });
  }
}

const markdown = lines.join("\n");
const topHeadlinesText = articles
  .slice(0, 5)
  .map((a, idx) => `${idx + 1}. ${a.title}`)
  .join("\n");

return [{
  json: {
    ...root,
    notion_title: `AllvIa AI Trend Digest ${digestDate}`,
    markdown_report: markdown,
    notion_excerpt: markdown.slice(0, 1800),
    notion_children: notionChildren.slice(0, 95),
    telegram_summary: topHeadlinesText,
    top_headlines_text: topHeadlinesText
  }
}];"#;
    let notion_result_code = r#"const base = ($items("Build Markdown Report", 0, 0)?.[0]?.json) || {};
const current = items[0]?.json || {};
const notionError = current?.error?.message ? String(current.error.message) : "";
return [{
  json: {
    ...base,
    notion_status: notionError ? "failed" : "completed",
    notion_error: notionError,
    notion_page_id: String(current.id || current.page_id || ""),
    notion_page_url: String(current.url || "")
  }
}];"#;
    let telegram_result_code = r#"const base = ($items("If Has Telegram Config", 0, 0)?.[0]?.json) || {};
const current = items[0]?.json || {};
const telegramError = current?.error?.message ? String(current.error.message) : "";
const messageId = current?.result?.message_id || current?.message_id || "";
return [{
  json: {
    ...base,
    telegram_status: telegramError ? "failed" : "completed",
    telegram_error: telegramError,
    telegram_message_id: String(messageId || "")
  }
}];"#;

    json!({
        "name": name,
        "nodes": [
            {
                "id": uuid::Uuid::new_v4().to_string(),
                "name": "Manual Trigger",
                "type": "n8n-nodes-base.manualTrigger",
                "typeVersion": 1,
                "position": [-420, -120],
                "parameters": {}
            },
            {
                "id": uuid::Uuid::new_v4().to_string(),
                "name": "ProgramWebhook",
                "type": "n8n-nodes-base.webhook",
                "typeVersion": 2,
                "position": [-420, 120],
                "parameters": {
                    "httpMethod": "POST",
                    "path": webhook_path,
                    "responseMode": "lastNode"
                }
            },
            {
                "id": uuid::Uuid::new_v4().to_string(),
                "name": "Input Envelope",
                "type": "n8n-nodes-base.set",
                "typeVersion": 2,
                "position": [-150, 0],
                "parameters": {
                    "keepOnlySet": false,
                    "values": {
                        "string": [
                            { "name": "request_title", "value": name },
                            { "name": "request_seed", "value": prompt_seed },
                            { "name": "request_text", "value": "={{$json.body && $json.body.prompt ? $json.body.prompt : ($json.body && $json.body.request_text ? $json.body.request_text : ($json.body && $json.body.text ? $json.body.text : ($json.body && $json.body.query ? $json.body.query : ($json.request_text || $json.request_seed || ''))))}}" },
                            { "name": "topic", "value": "={{$json.body && $json.body.topic ? $json.body.topic : ($json.topic || 'AI')}}" },
                            { "name": "timeframe", "value": "={{$json.body && $json.body.timeframe ? $json.body.timeframe : ($json.timeframe || '7d')}}" },
                            { "name": "language", "value": "={{$json.body && $json.body.language ? $json.body.language : ($json.language || 'en')}}" },
                            { "name": "top_n", "value": "={{$json.body && $json.body.top_n ? String($json.body.top_n) : ($json.body && $json.body.limit ? String($json.body.limit) : ($json.top_n || '5'))}}" },
                            { "name": "notion_parent_page_id", "value": notion_parent_expr },
                            { "name": "notion_token", "value": notion_token_expr },
                            { "name": "telegram_chat_id", "value": telegram_chat_expr },
                            { "name": "telegram_bot_token", "value": telegram_bot_expr },
                            { "name": "source_trigger", "value": "={{$json.headers ? 'webhook' : 'manual'}}" },
                            { "name": "orchestrator_version", "value": "steer_fallback_v4" },
                            { "name": "fallback_reason", "value": reason },
                            { "name": "started_at", "value": "={{$now}}" }
                        ]
                    }
                }
            },
            {
                "id": uuid::Uuid::new_v4().to_string(),
                "name": "Idempotency Guard",
                "type": "n8n-nodes-base.code",
                "typeVersion": 2,
                "position": [130, 0],
                "parameters": {
                    "mode": "runOnceForAllItems",
                    "jsCode": idempotency_code
                }
            },
            {
                "id": uuid::Uuid::new_v4().to_string(),
                "name": "Fetch Trend Feed",
                "type": "n8n-nodes-base.httpRequest",
                "typeVersion": 4.2,
                "position": [390, 0],
                "continueOnFail": true,
                "parameters": {
                    "method": "GET",
                    "url": "={{'https://hn.algolia.com/api/v1/search_by_date?tags=story&query=' + encodeURIComponent($json.topic || 'AI')}}",
                    "options": {
                        "timeout": 12000,
                        "retry": {
                            "maxTries": 3,
                            "waitBetweenTries": 1000
                        }
                    }
                }
            },
            {
                "id": uuid::Uuid::new_v4().to_string(),
                "name": "Extract Top 5 Articles",
                "type": "n8n-nodes-base.code",
                "typeVersion": 2,
                "position": [650, 0],
                "parameters": {
                    "mode": "runOnceForAllItems",
                    "jsCode": extract_code
                }
            },
            {
                "id": uuid::Uuid::new_v4().to_string(),
                "name": "Classify Trend Alignment",
                "type": "n8n-nodes-base.code",
                "typeVersion": 2,
                "position": [910, 0],
                "parameters": {
                    "mode": "runOnceForAllItems",
                    "jsCode": classify_code
                }
            },
            {
                "id": uuid::Uuid::new_v4().to_string(),
                "name": "Build Comparative Summary",
                "type": "n8n-nodes-base.code",
                "typeVersion": 2,
                "position": [1170, 0],
                "parameters": {
                    "mode": "runOnceForAllItems",
                    "jsCode": summarize_code
                }
            },
            {
                "id": uuid::Uuid::new_v4().to_string(),
                "name": "Validate Output Schema",
                "type": "n8n-nodes-base.code",
                "typeVersion": 2,
                "position": [1430, 0],
                "parameters": {
                    "mode": "runOnceForAllItems",
                    "jsCode": validate_code
                }
            },
            {
                "id": uuid::Uuid::new_v4().to_string(),
                "name": "If Validation OK",
                "type": "n8n-nodes-base.if",
                "typeVersion": 2,
                "position": [1680, 0],
                "parameters": {
                    "conditions": {
                        "string": [
                            {
                                "value1": "={{$json.validation_ok}}",
                                "operation": "equal",
                                "value2": "true"
                            }
                        ]
                    }
                }
            },
            {
                "id": uuid::Uuid::new_v4().to_string(),
                "name": "Build Markdown Report",
                "type": "n8n-nodes-base.code",
                "typeVersion": 2,
                "position": [1930, -120],
                "parameters": {
                    "mode": "runOnceForAllItems",
                    "jsCode": markdown_code
                }
            },
            {
                "id": uuid::Uuid::new_v4().to_string(),
                "name": "If Has Notion Config",
                "type": "n8n-nodes-base.if",
                "typeVersion": 2,
                "position": [2180, -120],
                "parameters": {
                    "conditions": {
                        "string": [
                            {
                                "value1": "={{$json.has_notion_config || 'false'}}",
                                "operation": "equal",
                                "value2": "true"
                            }
                        ]
                    }
                }
            },
            {
                "id": uuid::Uuid::new_v4().to_string(),
                "name": "Notion Create Page",
                "type": "n8n-nodes-base.httpRequest",
                "typeVersion": 4.2,
                "position": [2430, -240],
                "continueOnFail": true,
                "parameters": {
                    "method": "POST",
                    "url": "https://api.notion.com/v1/pages",
                    "sendHeaders": true,
                    "headerParameters": {
                        "parameters": [
                            { "name": "Authorization", "value": "={{'Bearer ' + ($json.notion_token || '')}}" },
                            { "name": "Notion-Version", "value": "2022-06-28" },
                            { "name": "Content-Type", "value": "application/json" }
                        ]
                    },
                    "sendBody": true,
                    "specifyBody": "json",
                    "jsonBody": "={{ { \"parent\": { \"type\": \"page_id\", \"page_id\": $json.notion_parent_page_id }, \"properties\": { \"title\": { \"title\": [ { \"text\": { \"content\": $json.notion_title || 'AllvIa AI Trend Digest' } } ] } }, \"children\": ((Array.isArray($json.notion_children) && $json.notion_children.length > 0) ? $json.notion_children.slice(0, 95) : [ { \"object\": \"block\", \"type\": \"paragraph\", \"paragraph\": { \"rich_text\": [ { \"type\": \"text\", \"text\": { \"content\": $json.notion_excerpt || '' } } ] } } ]) } }}",
                    "options": {
                        "timeout": 15000,
                        "retry": {
                            "maxTries": 3,
                            "waitBetweenTries": 1500
                        }
                    }
                }
            },
            {
                "id": uuid::Uuid::new_v4().to_string(),
                "name": "Normalize Notion Result",
                "type": "n8n-nodes-base.code",
                "typeVersion": 2,
                "position": [2670, -240],
                "parameters": {
                    "mode": "runOnceForAllItems",
                    "jsCode": notion_result_code
                }
            },
            {
                "id": uuid::Uuid::new_v4().to_string(),
                "name": "If Has Telegram Config",
                "type": "n8n-nodes-base.if",
                "typeVersion": 2,
                "position": [2920, -120],
                "parameters": {
                    "conditions": {
                        "string": [
                            {
                                "value1": "={{$json.has_telegram_config || 'false'}}",
                                "operation": "equal",
                                "value2": "true"
                            }
                        ]
                    }
                }
            },
            {
                "id": uuid::Uuid::new_v4().to_string(),
                "name": "Telegram Notify",
                "type": "n8n-nodes-base.httpRequest",
                "typeVersion": 4.2,
                "position": [3160, -240],
                "continueOnFail": true,
                "parameters": {
                    "method": "POST",
                    "url": "={{'https://api.telegram.org/bot' + ($json.telegram_bot_token || '') + '/sendMessage'}}",
                    "sendBody": true,
                    "specifyBody": "json",
                    "jsonBody": "={{ { \"chat_id\": $json.telegram_chat_id, \"text\": ($json.telegram_summary || 'Workflow completed'), \"disable_web_page_preview\": true } }}",
                    "options": {
                        "timeout": 12000,
                        "retry": {
                            "maxTries": 3,
                            "waitBetweenTries": 1200
                        }
                    }
                }
            },
            {
                "id": uuid::Uuid::new_v4().to_string(),
                "name": "Normalize Telegram Result",
                "type": "n8n-nodes-base.code",
                "typeVersion": 2,
                "position": [3400, -240],
                "parameters": {
                    "mode": "runOnceForAllItems",
                    "jsCode": telegram_result_code
                }
            },
            {
                "id": uuid::Uuid::new_v4().to_string(),
                "name": "Validation Error Payload",
                "type": "n8n-nodes-base.set",
                "typeVersion": 2,
                "position": [1930, 120],
                "parameters": {
                    "keepOnlySet": false,
                    "values": {
                        "string": [
                            { "name": "status", "value": "failed_validation" },
                            { "name": "error_summary", "value": "={{Array.isArray($json.validation_errors) ? $json.validation_errors.join('; ') : 'validation failed'}}" }
                        ]
                    }
                }
            },
            {
                "id": uuid::Uuid::new_v4().to_string(),
                "name": "Observability Log",
                "type": "n8n-nodes-base.set",
                "typeVersion": 2,
                "position": [3640, -120],
                "parameters": {
                    "keepOnlySet": false,
                    "values": {
                        "string": [
                            { "name": "status", "value": "={{$json.validation_ok === 'true' ? (($json.notion_status === 'failed' || $json.telegram_status === 'failed') ? 'completed_with_warnings' : 'completed') : ($json.status || 'failed')}}" },
                            { "name": "pipeline", "value": "allvia_fallback_orchestrator_v4" },
                            { "name": "idempotency_key", "value": "={{$json.idempotency_key || ''}}" },
                            { "name": "article_count", "value": "={{String($json.article_count || 0)}}" },
                            { "name": "fetch_ok", "value": "={{String($json.fetch_ok === true)}}" },
                            { "name": "notion_status", "value": "={{$json.notion_status || ($json.has_notion_config === 'true' ? 'unknown' : 'skipped')}}" },
                            { "name": "telegram_status", "value": "={{$json.telegram_status || ($json.has_telegram_config === 'true' ? 'unknown' : 'skipped')}}" },
                            { "name": "notion_error", "value": "={{$json.notion_error || ''}}" },
                            { "name": "telegram_error", "value": "={{$json.telegram_error || ''}}" },
                            { "name": "fallback_reason", "value": "={{$json.fallback_reason || ''}}" },
                            { "name": "completed_at", "value": "={{$now}}" }
                        ]
                    }
                }
            }
        ],
        "connections": {
            "Manual Trigger": {
                "main": [[{ "node": "Input Envelope", "type": "main", "index": 0 }]]
            },
            "ProgramWebhook": {
                "main": [[{ "node": "Input Envelope", "type": "main", "index": 0 }]]
            },
            "Input Envelope": {
                "main": [[{ "node": "Idempotency Guard", "type": "main", "index": 0 }]]
            },
            "Idempotency Guard": {
                "main": [[{ "node": "Fetch Trend Feed", "type": "main", "index": 0 }]]
            },
            "Fetch Trend Feed": {
                "main": [[{ "node": "Extract Top 5 Articles", "type": "main", "index": 0 }]]
            },
            "Extract Top 5 Articles": {
                "main": [[{ "node": "Classify Trend Alignment", "type": "main", "index": 0 }]]
            },
            "Classify Trend Alignment": {
                "main": [[{ "node": "Build Comparative Summary", "type": "main", "index": 0 }]]
            },
            "Build Comparative Summary": {
                "main": [[{ "node": "Validate Output Schema", "type": "main", "index": 0 }]]
            },
            "Validate Output Schema": {
                "main": [[{ "node": "If Validation OK", "type": "main", "index": 0 }]]
            },
            "If Validation OK": {
                "main": [
                    [{ "node": "Build Markdown Report", "type": "main", "index": 0 }],
                    [{ "node": "Validation Error Payload", "type": "main", "index": 0 }]
                ]
            },
            "Build Markdown Report": {
                "main": [[{ "node": "If Has Notion Config", "type": "main", "index": 0 }]]
            },
            "If Has Notion Config": {
                "main": [
                    [{ "node": "Notion Create Page", "type": "main", "index": 0 }],
                    [{ "node": "If Has Telegram Config", "type": "main", "index": 0 }]
                ]
            },
            "Notion Create Page": {
                "main": [[{ "node": "Normalize Notion Result", "type": "main", "index": 0 }]]
            },
            "Normalize Notion Result": {
                "main": [[{ "node": "If Has Telegram Config", "type": "main", "index": 0 }]]
            },
            "If Has Telegram Config": {
                "main": [
                    [{ "node": "Telegram Notify", "type": "main", "index": 0 }],
                    [{ "node": "Observability Log", "type": "main", "index": 0 }]
                ]
            },
            "Telegram Notify": {
                "main": [[{ "node": "Normalize Telegram Result", "type": "main", "index": 0 }]]
            },
            "Normalize Telegram Result": {
                "main": [[{ "node": "Observability Log", "type": "main", "index": 0 }]]
            },
            "Validation Error Payload": {
                "main": [[{ "node": "Observability Log", "type": "main", "index": 0 }]]
            }
        },
        "settings": {
            "saveManualExecutions": true,
            "executionTimeout": 300
        },
        "active": false,
        "meta": {
            "source": "steer-fallback-orchestrator-v4"
        }
    })
}

fn retry_after_ms_from_status_and_body(status: StatusCode, body: &str) -> Option<u64> {
    if status != StatusCode::TOO_MANY_REQUESTS && !status.is_server_error() {
        return None;
    }
    crate::retry_policy::parse_retry_after_ms(body)
}

fn parse_f64_env_with_default(key: &str, default: f64, min: f64, max: f64) -> f64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.trim().parse::<f64>().ok())
        .map(|v| v.clamp(min, max))
        .unwrap_or(default.clamp(min, max))
}

fn n8n_test_context() -> bool {
    parse_bool_env_with_default("STEER_TEST_MODE", false)
        || parse_bool_env_with_default("CI", false)
}

impl N8nApi {
    fn local_db_candidates() -> Vec<PathBuf> {
        if let Ok(custom) = std::env::var("STEER_N8N_DB_PATH") {
            let trimmed = custom.trim();
            if !trimmed.is_empty() {
                return vec![PathBuf::from(trimmed)];
            }
        }

        let home = match std::env::var("HOME") {
            Ok(value) if !value.trim().is_empty() => PathBuf::from(value),
            _ => return Vec::new(),
        };

        vec![
            home.join(".steer").join("n8n").join("database.sqlite"),
            home.join(".n8n").join("database.sqlite"),
        ]
    }

    fn local_latest_api_key_from_db() -> Option<String> {
        for db_path in Self::local_db_candidates() {
            if !db_path.exists() {
                continue;
            }

            let conn = match Connection::open(&db_path) {
                Ok(conn) => conn,
                Err(_) => continue,
            };
            let key: String = match conn.query_row(
                "SELECT apiKey FROM user_api_keys ORDER BY createdAt DESC LIMIT 1",
                [],
                |row| row.get::<_, String>(0),
            ) {
                Ok(value) => value,
                Err(_) => continue,
            };
            let trimmed = key.trim();
            if trimmed.is_empty() {
                continue;
            }
            return Some(trimmed.to_string());
        }

        None
    }

    fn n8n_cli_binary_available() -> bool {
        Command::new("n8n")
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }

    fn resolve_cli_invocation(&self) -> Result<(String, Vec<String>)> {
        if Self::n8n_cli_binary_available() {
            return Ok(("n8n".to_string(), Vec::new()));
        }

        let allow_npx_cli = parse_bool_env_with_default("STEER_N8N_ALLOW_NPX_CLI", false);
        if allow_npx_cli || matches!(self.runtime_mode(), N8nRuntime::Npx) {
            let test_context = n8n_test_context();
            if allow_npx_cli
                && !test_context
                && !parse_bool_env_with_default("STEER_N8N_ALLOW_NPX_CLI_NON_TEST", false)
                && !matches!(self.runtime_mode(), N8nRuntime::Npx)
            {
                return Err(anyhow::anyhow!(
                    "npx CLI fallback is test/CI-only by default. \
Set STEER_N8N_ALLOW_NPX_CLI_NON_TEST=1 to allow outside test mode."
                ));
            }
            if !self.local_target()
                && !parse_bool_env_with_default("STEER_N8N_ALLOW_NPX_CLI_REMOTE", false)
            {
                return Err(anyhow::anyhow!(
                    "npx CLI fallback is blocked for remote N8N_API_URL (set STEER_N8N_ALLOW_NPX_CLI_REMOTE=1 to override)."
                ));
            }
            if !parse_bool_env_with_default("STEER_N8N_ENABLE_NPX_RUNTIME", false) && !allow_npx_cli
            {
                return Err(anyhow::anyhow!(
                    "n8n CLI binary is missing and npx CLI fallback is disabled. \
Set STEER_N8N_ALLOW_NPX_CLI=1 (or enable npx runtime explicitly)."
                ));
            }
            return Ok(("npx".to_string(), vec!["-y".to_string(), "n8n".to_string()]));
        }

        Err(anyhow::anyhow!(
            "n8n CLI binary not found in PATH. Install n8n or set STEER_N8N_ALLOW_NPX_CLI=1"
        ))
    }

    fn build_http_client() -> Client {
        let prefer_no_proxy =
            cfg!(test) || parse_bool_env_with_default("STEER_HTTP_NO_SYSTEM_PROXY", false);
        if prefer_no_proxy {
            if let Ok(client) = Client::builder().no_proxy().build() {
                return client;
            }
        }

        if let Ok(client) = std::panic::catch_unwind(Client::new) {
            return client;
        }

        eprintln!("⚠️ reqwest default client init panicked; falling back to no-proxy client");
        Client::builder()
            .no_proxy()
            .build()
            .unwrap_or_else(|_| Client::new())
    }

    pub fn new(base_url: &str, api_key: &str) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: api_key.to_string(),
            client: Self::build_http_client(),
        }
    }

    pub fn from_env() -> anyhow::Result<Self> {
        let base_url = std::env::var("STEER_N8N_API_URL")
            .or_else(|_| std::env::var("N8N_API_URL"))
            .unwrap_or_else(|_| "http://localhost:5678/api/v1".to_string());
        // Allow missing key only when CLI fallback is explicitly enabled.
        let mut api_key = std::env::var("STEER_N8N_API_KEY")
            .or_else(|_| std::env::var("N8N_API_KEY"))
            .unwrap_or_default();
        let prefer_db_key = parse_bool_env_with_default("STEER_N8N_PREFER_LOCAL_DB_KEY", true);
        let local_target = base_url.contains("localhost") || base_url.contains("127.0.0.1");
        if prefer_db_key && local_target {
            if let Some(db_key) = Self::local_latest_api_key_from_db() {
                if api_key.trim() != db_key {
                    api_key = db_key;
                }
            }
        }
        Ok(Self::new(&base_url, &api_key))
    }

    fn runtime_mode(&self) -> N8nRuntime {
        N8nRuntime::from_env()
    }

    fn auto_start_enabled(&self, runtime: N8nRuntime) -> bool {
        let default = matches!(runtime, N8nRuntime::Docker);
        parse_bool_env_with_default("STEER_N8N_AUTO_START", default)
    }

    fn cli_fallback_enabled(&self, runtime: N8nRuntime) -> bool {
        let default = matches!(runtime, N8nRuntime::Npx);
        parse_bool_env_with_default("STEER_N8N_ALLOW_CLI_FALLBACK", default)
    }

    fn local_target(&self) -> bool {
        self.base_url.contains("localhost")
            || self.base_url.contains("127.0.0.1")
            || self.base_url.contains("0.0.0.0")
            || self.base_url.contains("::1")
    }

    fn reserve_ephemeral_local_port() -> Option<u16> {
        std::net::TcpListener::bind(("127.0.0.1", 0))
            .ok()
            .and_then(|listener| listener.local_addr().ok().map(|addr| addr.port()))
    }

    fn health_urls(&self) -> (String, String) {
        let root_url = self
            .base_url
            .replace("localhost", "127.0.0.1")
            .replace("/api/v1", "/");
        let root_trimmed = root_url.trim_end_matches('/');
        let healthz = format!("{}/healthz", root_trimmed);
        (healthz, format!("{}/", root_trimmed))
    }

    fn http_retry_attempts(&self) -> u32 {
        parse_u32_env_with_default("STEER_N8N_HTTP_RETRY_ATTEMPTS", 4, 1, 8)
    }

    fn http_retry_min_backoff_ms(&self) -> u64 {
        parse_u64_env_with_default("STEER_N8N_HTTP_RETRY_MIN_BACKOFF_MS", 400, 100, 60_000)
    }

    fn http_retry_max_backoff_ms(&self) -> u64 {
        parse_u64_env_with_default("STEER_N8N_HTTP_RETRY_MAX_BACKOFF_MS", 10_000, 500, 120_000)
    }

    fn http_retry_jitter(&self) -> f64 {
        parse_f64_env_with_default("STEER_N8N_HTTP_RETRY_JITTER", 0.1, 0.0, 0.5)
    }

    fn http_request_timeout_ms(&self) -> u64 {
        parse_u64_env_with_default("STEER_N8N_HTTP_REQUEST_TIMEOUT_MS", 12_000, 1_000, 120_000)
    }

    async fn send_with_retry<F>(&self, label: &str, mut build: F) -> Result<Response>
    where
        F: FnMut() -> reqwest::RequestBuilder,
    {
        let policy = crate::retry_policy::RetryPolicy::new(
            self.http_retry_attempts(),
            self.http_retry_min_backoff_ms(),
            self.http_retry_max_backoff_ms(),
            self.http_retry_jitter(),
        );
        let request_timeout = Duration::from_millis(self.http_request_timeout_ms());

        for attempt in 1..=policy.attempts {
            match build().timeout(request_timeout).send().await {
                Ok(resp) => {
                    if resp.status().is_success() {
                        return Ok(resp);
                    }
                    let status = resp.status();
                    let body = resp.text().await.unwrap_or_default();
                    let retryable = crate::retry_policy::retryable_http_status(status);
                    if retryable && attempt < policy.attempts {
                        let retry_after = retry_after_ms_from_status_and_body(status, &body);
                        let delay = crate::retry_policy::compute_backoff_delay_ms(
                            policy,
                            attempt - 1,
                            retry_after,
                            label,
                        );
                        crate::diagnostic_events::emit(
                            "n8n.http.retry",
                            json!({
                                "label": label,
                                "attempt": attempt,
                                "status": status.as_u16(),
                                "delay_ms": delay,
                                "retry_after_ms": retry_after
                            }),
                        );
                        tokio::time::sleep(Duration::from_millis(delay)).await;
                        continue;
                    }
                    return Err(anyhow::anyhow!(
                        "{} failed (status={}): {}",
                        label,
                        status,
                        body
                    ));
                }
                Err(err) => {
                    let retryable = err.is_timeout() || err.is_connect() || err.is_request();
                    if retryable && attempt < policy.attempts {
                        let delay = crate::retry_policy::compute_backoff_delay_ms(
                            policy,
                            attempt - 1,
                            None,
                            label,
                        );
                        crate::diagnostic_events::emit(
                            "n8n.http.retry",
                            json!({
                                "label": label,
                                "attempt": attempt,
                                "error": err.to_string(),
                                "delay_ms": delay
                            }),
                        );
                        tokio::time::sleep(Duration::from_millis(delay)).await;
                        continue;
                    }
                    return Err(anyhow::anyhow!("{} request failed: {}", label, err));
                }
            }
        }

        Err(anyhow::anyhow!(
            "{} failed after {} attempt(s)",
            label,
            policy.attempts
        ))
    }

    async fn is_server_reachable(&self, healthz: &str, root: &str) -> bool {
        let timeout = std::time::Duration::from_secs(2);
        let ok_health = self
            .client
            .get(healthz)
            .timeout(timeout)
            .send()
            .await
            .map(|resp| resp.status().is_success())
            .unwrap_or(false);
        if ok_health {
            return true;
        }

        self.client
            .get(root)
            .timeout(timeout)
            .send()
            .await
            .map(|resp| resp.status().is_success())
            .unwrap_or(false)
    }

    fn resolve_compose_file() -> Option<PathBuf> {
        if let Ok(raw) = std::env::var("STEER_N8N_COMPOSE_FILE") {
            let trimmed = raw.trim();
            if !trimmed.is_empty() {
                return Some(PathBuf::from(trimmed));
            }
        }

        let cwd = std::env::current_dir().ok()?;
        let candidates = [
            cwd.join("docker-compose.yml"),
            cwd.join("../docker-compose.yml"),
        ];
        candidates.into_iter().find(|p| p.is_file())
    }

    fn run_docker_compose(compose_file: &Path, args: &[&str]) -> Result<()> {
        let compose_file_str = compose_file.display().to_string();
        let run_primary = std::process::Command::new("docker")
            .arg("compose")
            .arg("-f")
            .arg(&compose_file_str)
            .args(args)
            .output();

        match run_primary {
            Ok(out) if out.status.success() => return Ok(()),
            Ok(out) => {
                let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
                let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
                let detail = if !stderr.is_empty() { stderr } else { stdout };
                eprintln!(
                    "⚠️ docker compose failed, trying docker-compose fallback: {}",
                    detail
                );
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(anyhow::anyhow!("failed to run docker compose: {}", e)),
        }

        let legacy = std::process::Command::new("docker-compose")
            .arg("-f")
            .arg(&compose_file_str)
            .args(args)
            .output();
        match legacy {
            Ok(out) if out.status.success() => Ok(()),
            Ok(out) => {
                let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
                let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
                let detail = if !stderr.is_empty() { stderr } else { stdout };
                Err(anyhow::anyhow!("docker-compose failed: {}", detail))
            }
            Err(e) => Err(anyhow::anyhow!(
                "docker compose unavailable (tried docker compose + docker-compose): {}",
                e
            )),
        }
    }

    fn start_with_docker(&self) -> Result<()> {
        let compose_file = Self::resolve_compose_file().ok_or_else(|| {
            anyhow::anyhow!(
                "docker-compose.yml not found. Place it at repo root or set STEER_N8N_COMPOSE_FILE"
            )
        })?;
        println!(
            "🐳 Starting n8n via Docker Compose (runtime=docker, file={})...",
            compose_file.display()
        );
        Self::run_docker_compose(&compose_file, &["up", "-d", "n8n"])
    }

    fn start_with_npx(&self) -> Result<()> {
        if !parse_bool_env_with_default("STEER_N8N_ENABLE_NPX_RUNTIME", false) {
            return Err(anyhow::anyhow!(
                "npx runtime is disabled by default. Set STEER_N8N_ENABLE_NPX_RUNTIME=1 to enable."
            ));
        }
        let test_context = n8n_test_context();
        let requested_tunnel = crate::env_flag("STEER_N8N_USE_TUNNEL");
        if requested_tunnel
            && !test_context
            && !parse_bool_env_with_default("STEER_N8N_ALLOW_NPX_TUNNEL_NON_TEST", false)
        {
            crate::diagnostic_events::emit(
                "n8n.runtime.npx.blocked",
                json!({
                    "reason": "tunnel_non_test_blocked",
                    "test_context": test_context
                }),
            );
            return Err(anyhow::anyhow!(
                "npx --tunnel is test/CI-only by default. \
Set STEER_N8N_ALLOW_NPX_TUNNEL_NON_TEST=1 to override."
            ));
        }
        println!("⚠️  Starting n8n with npx fallback runtime...");
        let mut args = vec!["-y", "n8n", "start"];
        if requested_tunnel {
            args.push("--tunnel");
        }
        crate::diagnostic_events::emit(
            "n8n.runtime.npx.start",
            json!({
                "tunnel": requested_tunnel,
                "test_context": test_context
            }),
        );

        Command::new("npx")
            .args(args)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| anyhow::anyhow!("Failed to auto-start n8n with npx: {}", e))?;
        Ok(())
    }

    fn start_runtime(&self, runtime: N8nRuntime) -> Result<()> {
        match runtime {
            N8nRuntime::Docker => self.start_with_docker(),
            N8nRuntime::Npx => self.start_with_npx(),
            N8nRuntime::Manual => Err(anyhow::anyhow!(
                "runtime=manual: start n8n yourself and set N8N_API_URL/N8N_API_KEY"
            )),
        }
    }

    pub async fn restart_server(&self) -> Result<()> {
        if crate::env_flag("STEER_N8N_MOCK") {
            println!("🧪 STEER_N8N_MOCK=1: skipping n8n restart");
            return Ok(());
        }

        let runtime = self.runtime_mode();
        if !self.local_target() && !matches!(runtime, N8nRuntime::Manual) {
            return Err(anyhow::anyhow!(
                "runtime={} cannot restart remote n8n target ({})",
                runtime.as_str(),
                self.base_url
            ));
        }

        match runtime {
            N8nRuntime::Docker => {
                let compose_file = Self::resolve_compose_file().ok_or_else(|| {
                    anyhow::anyhow!(
                        "docker-compose.yml not found. Place it at repo root or set STEER_N8N_COMPOSE_FILE"
                    )
                })?;
                println!(
                    "🐳 Restarting n8n via Docker Compose (file={})...",
                    compose_file.display()
                );
                if let Err(restart_err) =
                    Self::run_docker_compose(&compose_file, &["restart", "n8n"])
                {
                    eprintln!(
                        "⚠️ Docker restart failed ({}). Trying `up -d n8n`...",
                        restart_err
                    );
                    Self::run_docker_compose(&compose_file, &["up", "-d", "n8n"])?;
                }
            }
            N8nRuntime::Npx => {
                let _ = std::process::Command::new("pkill")
                    .arg("-f")
                    .arg("n8n")
                    .output();
                self.start_with_npx()?;
            }
            N8nRuntime::Manual => {
                return Err(anyhow::anyhow!(
                    "runtime=manual: cannot restart automatically"
                ));
            }
        }

        let (healthz, root) = self.health_urls();
        for _ in 0..30 {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            if self.is_server_reachable(&healthz, &root).await {
                println!("✅ n8n restart completed.");
                return Ok(());
            }
        }
        Err(anyhow::anyhow!("Timed out waiting for n8n after restart"))
    }

    /// Check if n8n is running, and start it if not
    pub async fn ensure_server_running(&self) -> Result<()> {
        if crate::env_flag("STEER_N8N_MOCK") {
            println!("🧪 STEER_N8N_MOCK=1: skipping n8n health/start checks");
            return Ok(());
        }

        let runtime = self.runtime_mode();
        let (healthz, root) = self.health_urls();
        println!(
            "🔎 Checking n8n health (runtime={}, healthz={})...",
            runtime.as_str(),
            healthz
        );

        if self.is_server_reachable(&healthz, &root).await {
            println!("✅ n8n server is running.");
            return Ok(());
        }

        if !self.auto_start_enabled(runtime) {
            return Err(anyhow::anyhow!(
                "n8n server is not reachable at {}. Enable auto-start with STEER_N8N_AUTO_START=1 or run n8n manually.",
                healthz
            ));
        }

        if !self.local_target() && !matches!(runtime, N8nRuntime::Manual) {
            return Err(anyhow::anyhow!(
                "runtime={} cannot auto-start remote n8n target ({})",
                runtime.as_str(),
                self.base_url
            ));
        }

        println!("⚠️  n8n server NOT found. Starting automatically...");
        self.start_runtime(runtime)?;

        println!("⏳ Waiting for n8n to initialize (this may take 60s)...");
        for i in 0..30 {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            if self.is_server_reachable(&healthz, &root).await {
                println!("🚀 n8n server started successfully!");
                if !self.api_key.is_empty() && self.api_key != "placeholder" {
                    self.verify_auth().await?;
                }
                return Ok(());
            }
            if i % 5 == 0 {
                println!("... still waiting ({}/60s)", i * 2);
            }
        }

        Err(anyhow::anyhow!("Timed out waiting for n8n to start."))
    }

    /// Helper: Verify API Key works
    pub async fn verify_auth(&self) -> Result<()> {
        println!("🔐 Verifying n8n API Key...");
        // Try a lightweight authenticated call
        let url = format!("{}/workflows?limit=1", self.base_url);

        let resp = self
            .send_with_retry("n8n verify_auth", || {
                self.client
                    .get(&url)
                    .header("X-N8N-API-KEY", &self.api_key)
                    .timeout(std::time::Duration::from_secs(3))
            })
            .await?;

        if resp.status().is_success() {
            println!("✅ API Key is valid.");
            Ok(())
        } else if resp.status() == reqwest::StatusCode::UNAUTHORIZED
            || resp.status() == reqwest::StatusCode::FORBIDDEN
        {
            Err(anyhow::anyhow!(
                "❌ n8n API Key is INVALID ({}). Check core/.env or secrets.",
                resp.status()
            ))
        } else {
            Err(anyhow::anyhow!(
                "❌ n8n auth verification failed with status {}",
                resp.status()
            ))
        }
    }

    /// List available credentials
    pub async fn list_credentials(&self) -> Result<Vec<Credential>> {
        if crate::env_flag("STEER_N8N_MOCK") {
            return Ok(Vec::new());
        }

        let url = format!("{}/credentials", self.base_url);
        let resp = self
            .send_with_retry("n8n list_credentials", || {
                self.client.get(&url).header("X-N8N-API-KEY", &self.api_key)
            })
            .await?;

        if !resp.status().is_success() {
            return Ok(Vec::new()); // Return empty if failed (e.g. auth error)
        }

        let json: Value = resp.json().await?;
        let data = json["data"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("Invalid credentials response"))?;

        // n8n API structure differs by version, trying to extract minimal info
        let credentials = data
            .iter()
            .map(|c| Credential {
                id: c["id"].as_str().unwrap_or("").to_string(),
                name: c["name"].as_str().unwrap_or("").to_string(),
                type_name: c["type"].as_str().unwrap_or("").to_string(),
            })
            .collect();

        Ok(credentials)
    }

    /// Create a new workflow (Hybrid: API first, then CLI fallback with ID retrieval)
    pub async fn create_workflow(
        &self,
        name: &str,
        workflow_json: &Value,
        active: bool,
    ) -> Result<String> {
        if crate::env_flag("STEER_N8N_MOCK") {
            let mock_id = format!("mock-wf-{}", chrono::Utc::now().timestamp_millis());
            println!(
                "🧪 STEER_N8N_MOCK=1: skipping n8n network/CLI calls and returning {}",
                mock_id
            );
            return Ok(mock_id);
        }

        // 1. Validate JSON and normalize once for consistent downstream behavior.
        let normalized = normalize_workflow_for_create(name, workflow_json)?;

        // 2. Validate Credentials (Prevent broken workflows)
        // Only if API key is present (we need API to list creds)
        if !self.api_key.is_empty() && self.api_key != "placeholder" {
            if let Ok(creds) = self.list_credentials().await {
                let valid_ids: Vec<String> = creds.iter().map(|c| c.id.clone()).collect();

                if let Some(nodes) = normalized.get("nodes").and_then(|n| n.as_array()) {
                    for node in nodes {
                        if let Some(cred_map) = node.get("credentials") {
                            if let Some(obj) = cred_map.as_object() {
                                for (_, v) in obj {
                                    if let Some(id) = v.get("id").and_then(|i: &Value| i.as_str()) {
                                        if !valid_ids.contains(&id.to_string()) {
                                            return Err(anyhow::anyhow!("❌ Validation Failed: Credential ID '{}' does not exist in n8n.", id));
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        let runtime = self.runtime_mode();
        let allow_cli_fallback = self.cli_fallback_enabled(runtime);

        // 3. Try API
        if !self.api_key.is_empty() && self.api_key != "placeholder" {
            println!("🌐 Attempting to create workflow via API...");
            match self.create_workflow_api(name, &normalized, active).await {
                Ok(id) => return Ok(id),
                Err(e) => {
                    if !allow_cli_fallback {
                        return Err(anyhow::anyhow!(
                            "n8n API creation failed and CLI fallback is disabled (runtime={}): {}",
                            runtime.as_str(),
                            e
                        ));
                    }
                    println!("⚠️ API creation failed ({}). Falling back to CLI...", e);
                }
            }
        } else {
            if !allow_cli_fallback {
                return Err(anyhow::anyhow!(
                    "N8N_API_KEY is not set and CLI fallback is disabled (runtime={}). Set N8N_API_KEY or enable STEER_N8N_ALLOW_CLI_FALLBACK=1",
                    runtime.as_str()
                ));
            }
            println!("ℹ️ No API Key configured. Using CLI fallback mode.");
        }

        // 4. Fallback to CLI (Strict Local Check)
        if !self.local_target() {
            return Err(anyhow::anyhow!(
                "❌ CLI Fallback aborted: n8n is remote ({}). CLI only works for local instances.",
                self.base_url
            ));
        }

        let import_marker = format!("steer-import-{}", uuid::Uuid::new_v4());

        // 5. Run CLI Import
        if let Err(e) = self
            .create_workflow_cli(name, &normalized, active, &import_marker)
            .await
        {
            return Err(anyhow::anyhow!("❌ CLI Fallback Failed: {}", e));
        }

        // 6. Retrieve ID via CLI export (no direct SQLite coupling).
        self.retrieve_workflow_id_via_cli_export(name, &import_marker)
            .await
    }

    async fn create_workflow_api(
        &self,
        name: &str,
        workflow_json: &Value,
        active: bool,
    ) -> Result<String> {
        let url = format!("{}/workflows", self.base_url);

        let body = json!({
            "name": name,
            "nodes": workflow_json.get("nodes").cloned().unwrap_or(json!([])),
            "connections": workflow_json.get("connections").cloned().unwrap_or(json!({})),
            "settings": workflow_json.get("settings").cloned().unwrap_or(json!({"saveManualExecutions": true}))
        });
        // NOTE: Some n8n versions reject `active` as read-only on create.
        // We always create inactive here; activation can be done via a separate endpoint if needed.

        let body_for_req = body.clone();
        let resp = self
            .send_with_retry("n8n create_workflow", || {
                self.client
                    .post(&url)
                    .header("X-N8N-API-KEY", &self.api_key)
                    .json(&body_for_req)
            })
            .await?;

        if !resp.status().is_success() {
            let error_text = resp.text().await?;
            return Err(anyhow::anyhow!("n8n API Error: {}", error_text));
        }

        let resp_json: Value = resp.json().await?;
        let id = resp_json["id"].as_str().unwrap_or("unknown").to_string();
        if let Err(webhook_fix_err) = self.enable_disabled_webhook_nodes(&id).await {
            eprintln!(
                "⚠️ failed to normalize webhook trigger state for workflow {}: {}",
                id, webhook_fix_err
            );
        }
        if active {
            if let Err(activate_err) = self.activate_workflow(&id).await {
                // Some n8n versions expose activation differently; attempt generic update fallback.
                self.update_workflow_active(&id, true).await.map_err(|patch_err| {
                    anyhow::anyhow!(
                        "workflow created (id={}) but activation failed: {} | fallback update failed: {}",
                        id,
                        activate_err,
                        patch_err
                    )
                })?;
            }
        }
        Ok(id)
    }

    async fn enable_disabled_webhook_nodes(&self, id: &str) -> Result<bool> {
        let url = format!("{}/workflows/{}", self.base_url, id);
        let workflow_resp = self
            .send_with_retry("n8n get_workflow_for_webhook_fix", || {
                self.client.get(&url).header("X-N8N-API-KEY", &self.api_key)
            })
            .await?;
        let workflow: Value = workflow_resp.json().await?;

        let nodes = workflow
            .get("nodes")
            .and_then(|v| v.as_array())
            .ok_or_else(|| anyhow::anyhow!("workflow {} has invalid nodes payload", id))?;

        let mut has_webhook = false;
        let mut changed = false;
        let mut normalized_nodes: Vec<Value> = Vec::with_capacity(nodes.len());
        for node in nodes {
            let mut node_value = node.clone();
            if node
                .get("type")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                == "n8n-nodes-base.webhook"
            {
                has_webhook = true;
                if let Some(obj) = node_value.as_object_mut() {
                    let is_disabled = obj
                        .get("disabled")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    if is_disabled {
                        obj.insert("disabled".to_string(), Value::Bool(false));
                        changed = true;
                    }
                    let missing_webhook_id = obj
                        .get("webhookId")
                        .and_then(|v| v.as_str())
                        .map(|v| v.trim().is_empty())
                        .unwrap_or(true);
                    if missing_webhook_id {
                        obj.insert(
                            "webhookId".to_string(),
                            Value::String(uuid::Uuid::new_v4().to_string()),
                        );
                        changed = true;
                    }
                }
            }
            normalized_nodes.push(node_value);
        }

        if !has_webhook || !changed {
            return Ok(false);
        }

        let update_body = json!({
            "name": workflow.get("name").cloned().unwrap_or_else(|| json!("workflow")),
            "nodes": normalized_nodes,
            "connections": workflow.get("connections").cloned().unwrap_or_else(|| json!({})),
            "settings": workflow.get("settings").cloned().unwrap_or_else(|| json!({}))
        });
        let _ = self
            .send_with_retry("n8n fix_disabled_webhook_nodes", || {
                self.client
                    .put(&url)
                    .header("X-N8N-API-KEY", &self.api_key)
                    .json(&update_body)
            })
            .await?;
        Ok(true)
    }

    async fn update_workflow_active(&self, id: &str, active: bool) -> Result<()> {
        let url = format!("{}/workflows/{}", self.base_url, id);
        let workflow_resp = self
            .send_with_retry("n8n get_workflow_for_active_update", || {
                self.client.get(&url).header("X-N8N-API-KEY", &self.api_key)
            })
            .await?;
        let workflow: Value = workflow_resp.json().await?;
        let update_body = json!({
            "name": workflow.get("name").cloned().unwrap_or_else(|| json!("workflow")),
            "nodes": workflow.get("nodes").cloned().unwrap_or_else(|| json!([])),
            "connections": workflow.get("connections").cloned().unwrap_or_else(|| json!({})),
            "settings": workflow.get("settings").cloned().unwrap_or_else(|| json!({})),
            "active": active
        });
        let _ = self
            .send_with_retry("n8n update_workflow_active", || {
                self.client
                    .put(&url)
                    .header("X-N8N-API-KEY", &self.api_key)
                    .json(&update_body)
            })
            .await?;
        if active {
            let _ = self.enable_disabled_webhook_nodes(id).await;
        }
        Ok(())
    }

    async fn create_workflow_cli(
        &self,
        name: &str,
        workflow_json: &Value,
        active: bool,
        import_marker: &str,
    ) -> Result<String> {
        // Prepare JSON file
        let mut final_json = workflow_json.clone();
        final_json["name"] = json!(name);
        final_json["active"] = json!(active);
        let mut settings = final_json
            .get("settings")
            .cloned()
            .unwrap_or_else(|| json!({}));
        if !settings.is_object() {
            settings = json!({});
        }
        settings["steerImportId"] = json!(import_marker);
        final_json["settings"] = settings;

        // Ensure nodes exist
        if final_json["nodes"].as_array().is_none_or(|n| n.is_empty()) {
            return Err(anyhow::anyhow!("Refusing to import empty workflow via CLI"));
        }

        let path = format!("/tmp/n8n_import_{}.json", uuid::Uuid::new_v4());
        tokio::fs::write(&path, serde_json::to_string(&final_json)?).await?;

        println!("📥 Importing workflow via CLI from {}...", path);

        let (cli_bin, cli_prefix) = self.resolve_cli_invocation()?;
        let mut cmd = tokio::process::Command::new(&cli_bin);
        cmd.args(&cli_prefix);
        cmd.args(["import:workflow", "--input", &path]);
        let output = cmd.output().await?;

        // Cleanup
        if let Err(e) = tokio::fs::remove_file(&path).await {
            eprintln!("⚠️ Failed to clean up temp file: {}", e);
        }

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let code = output
                .status
                .code()
                .map(|c| c.to_string())
                .unwrap_or_else(|| "signal".to_string());
            let detail = if !stderr.is_empty() { stderr } else { stdout };
            return Err(anyhow::anyhow!(
                "CLI Import failed (bin={}, exit {}): {}",
                cli_bin,
                code,
                detail
            ));
        }

        println!("✅ CLI Import successful!");

        Ok("cli-imported".to_string())
    }

    fn workflow_id_from_value(value: Option<&Value>) -> Option<String> {
        match value {
            Some(Value::String(s)) if !s.trim().is_empty() => Some(s.trim().to_string()),
            Some(Value::Number(n)) => Some(n.to_string()),
            _ => None,
        }
    }

    fn export_items_from_value(value: &Value) -> Vec<Value> {
        match value {
            Value::Array(items) => items.clone(),
            Value::Object(map) => {
                if let Some(Value::Array(items)) = map.get("data") {
                    items.clone()
                } else if map.contains_key("nodes") || map.contains_key("connections") {
                    vec![value.clone()]
                } else {
                    Vec::new()
                }
            }
            _ => Vec::new(),
        }
    }

    async fn retrieve_workflow_id_via_cli_export(
        &self,
        name: &str,
        import_marker: &str,
    ) -> Result<String> {
        let path = format!("/tmp/n8n_export_{}.json", uuid::Uuid::new_v4());
        let (cli_bin, cli_prefix) = self.resolve_cli_invocation()?;
        let mut cmd = tokio::process::Command::new(&cli_bin);
        cmd.args(&cli_prefix);
        cmd.args(["export:workflow", "--all", "--output", &path]);
        let output = cmd.output().await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let detail = if !stderr.is_empty() { stderr } else { stdout };
            return Err(anyhow::anyhow!(
                "CLI workflow id lookup failed after import (bin={}): {}",
                cli_bin,
                detail
            ));
        }

        let raw = tokio::fs::read_to_string(&path).await?;
        if let Err(e) = tokio::fs::remove_file(&path).await {
            eprintln!("⚠️ Failed to clean up export file {}: {}", path, e);
        }

        let parsed: Value = serde_json::from_str(&raw).map_err(|e| {
            anyhow::anyhow!(
                "Failed to parse exported workflows while resolving imported workflow id: {}",
                e
            )
        })?;
        let items = Self::export_items_from_value(&parsed);
        if items.is_empty() {
            return Err(anyhow::anyhow!(
                "No workflows found in n8n CLI export while resolving imported workflow id"
            ));
        }

        let mut marker_match: Option<String> = None;
        let mut name_matches: Vec<String> = Vec::new();
        for item in items {
            let Some(id) = Self::workflow_id_from_value(item.get("id")) else {
                continue;
            };
            let settings_text = item
                .get("settings")
                .map(|v| v.to_string())
                .unwrap_or_default();
            if !import_marker.trim().is_empty() && settings_text.contains(import_marker) {
                marker_match = Some(id);
                break;
            }

            if item
                .get("name")
                .and_then(|v| v.as_str())
                .map(|wf_name| wf_name == name)
                .unwrap_or(false)
            {
                name_matches.push(id);
            }
        }

        if let Some(id) = marker_match {
            return Ok(id);
        }
        let allow_name_fallback =
            parse_bool_env_with_default("STEER_N8N_ALLOW_NAME_ID_FALLBACK", false);
        if allow_name_fallback {
            if let Some(id) = name_matches.first() {
                if name_matches.len() > 1
                    && !parse_bool_env_with_default(
                        "STEER_N8N_ALLOW_AMBIGUOUS_NAME_ID_FALLBACK",
                        false,
                    )
                {
                    return Err(anyhow::anyhow!(
                        "Ambiguous name-based fallback for '{}': {} matches. \
Set STEER_N8N_ALLOW_AMBIGUOUS_NAME_ID_FALLBACK=1 only for controlled test environments.",
                        name,
                        name_matches.len()
                    ));
                }
                if name_matches.len() > 1 {
                    eprintln!(
                        "⚠️ Ambiguous name fallback explicitly allowed for '{}'; using first exported id={}",
                        name, id
                    );
                }
                return Ok(id.clone());
            }
        }

        if !name_matches.is_empty() {
            return Err(anyhow::anyhow!(
                "Import marker not found in exported workflows; {} name match(es) exist for '{}'. \
Set STEER_N8N_ALLOW_NAME_ID_FALLBACK=1 to allow name-based fallback.",
                name_matches.len(),
                name
            ));
        }
        Err(anyhow::anyhow!(
            "Could not resolve imported workflow id via n8n CLI export"
        ))
    }

    fn build_minimal_workflow(name: &str) -> Value {
        build_orchestrator_fallback_workflow(name, None, "legacy_minimal_template")
    }

    async fn activate_workflow_cli(&self, id: &str, active: bool) -> Result<()> {
        let (cli_bin, cli_prefix) = self.resolve_cli_invocation()?;
        let mut cmd = tokio::process::Command::new(&cli_bin);
        cmd.args(&cli_prefix);
        cmd.args([
            "update:workflow",
            "--id",
            id,
            "--active",
            if active { "true" } else { "false" },
        ]);
        let output = cmd.output().await?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let detail = if !stderr.is_empty() { stderr } else { stdout };
            return Err(anyhow::anyhow!(
                "n8n CLI update:workflow failed for id={} active={}: {}",
                id,
                active,
                detail
            ));
        }
        Ok(())
    }

    async fn execute_workflow_cli(&self, id: &str) -> Result<ExecutionResult> {
        let (cli_bin, cli_prefix) = self.resolve_cli_invocation()?;
        let mut cmd = tokio::process::Command::new(&cli_bin);
        cmd.args(&cli_prefix);
        let broker_port = std::env::var("N8N_RUNNERS_BROKER_PORT")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .or_else(|| Self::reserve_ephemeral_local_port().map(|port| port.to_string()));
        if let Some(port) = broker_port {
            cmd.env("N8N_RUNNERS_BROKER_PORT", port);
        }
        cmd.args(["execute", "--id", id, "--rawOutput"]);
        let output = cmd.output().await?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let detail = if !stderr.is_empty() { stderr } else { stdout };
            return Err(anyhow::anyhow!(
                "n8n CLI execute failed for id={}: {}",
                id,
                detail
            ));
        }

        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let parsed = serde_json::from_str::<Value>(&stdout).ok();
        let now = chrono::Utc::now().to_rfc3339();
        Ok(ExecutionResult {
            id: parsed
                .as_ref()
                .and_then(|v| v.get("id").and_then(|x| x.as_str()))
                .unwrap_or("")
                .to_string(),
            finished: parsed
                .as_ref()
                .and_then(|v| v.get("finished").and_then(|x| x.as_bool()))
                .unwrap_or(true),
            status: parsed
                .as_ref()
                .and_then(|v| v.get("status").and_then(|x| x.as_str()))
                .unwrap_or("success")
                .to_string(),
            started_at: parsed
                .as_ref()
                .and_then(|v| v.get("startedAt").and_then(|x| x.as_str()))
                .unwrap_or(&now)
                .to_string(),
            stopped_at: parsed
                .as_ref()
                .and_then(|v| v.get("stoppedAt").and_then(|x| x.as_str()))
                .map(|s| s.to_string()),
        })
    }

    /// Activate a workflow
    pub async fn activate_workflow(&self, id: &str) -> Result<()> {
        let url = format!("{}/workflows/{}/activate", self.base_url, id);
        match self
            .send_with_retry("n8n activate_workflow", || {
                self.client
                    .post(&url)
                    .header("X-N8N-API-KEY", &self.api_key)
            })
            .await
        {
            Ok(_) => Ok(()),
            Err(api_err) => {
                if self.local_target() {
                    if let Ok(true) = self.enable_disabled_webhook_nodes(id).await {
                        match self
                            .send_with_retry(
                                "n8n activate_workflow_retry_after_webhook_fix",
                                || {
                                    self.client
                                        .post(&url)
                                        .header("X-N8N-API-KEY", &self.api_key)
                                },
                            )
                            .await
                        {
                            Ok(_) => return Ok(()),
                            Err(retry_err) => {
                                eprintln!(
                                    "⚠️ activate retry after webhook-fix failed: {}",
                                    retry_err
                                );
                            }
                        }
                    }
                    match self.activate_workflow_cli(id, true).await {
                        Ok(_) => {
                            println!(
                                "⚠️ n8n API activate failed ({}), recovered with CLI update:workflow",
                                api_err
                            );
                            return Ok(());
                        }
                        Err(cli_err) => {
                            return Err(anyhow::anyhow!(
                                "n8n activate error: {} | CLI fallback failed: {}",
                                api_err,
                                cli_err
                            ));
                        }
                    }
                }
                Err(anyhow::anyhow!("n8n activate error: {}", api_err))
            }
        }
    }

    /// Deactivate a workflow
    pub async fn deactivate_workflow(&self, id: &str) -> Result<()> {
        let url = format!("{}/workflows/{}/deactivate", self.base_url, id);
        match self
            .send_with_retry("n8n deactivate_workflow", || {
                self.client
                    .post(&url)
                    .header("X-N8N-API-KEY", &self.api_key)
            })
            .await
        {
            Ok(_) => Ok(()),
            Err(api_err) => {
                if self.local_target() {
                    match self.activate_workflow_cli(id, false).await {
                        Ok(_) => {
                            println!(
                                "⚠️ n8n API deactivate failed ({}), recovered with CLI update:workflow",
                                api_err
                            );
                            return Ok(());
                        }
                        Err(cli_err) => {
                            return Err(anyhow::anyhow!(
                                "n8n deactivate error: {} | CLI fallback failed: {}",
                                api_err,
                                cli_err
                            ));
                        }
                    }
                }
                Err(anyhow::anyhow!("n8n deactivate error: {}", api_err))
            }
        }
    }

    /// Get workflow status
    pub async fn get_workflow(&self, id: &str) -> Result<WorkflowStatus> {
        let url = format!("{}/workflows/{}", self.base_url, id);
        let resp = self
            .send_with_retry("n8n get_workflow", || {
                self.client.get(&url).header("X-N8N-API-KEY", &self.api_key)
            })
            .await?;

        if !resp.status().is_success() {
            let error_text = resp.text().await?;
            return Err(anyhow::anyhow!("n8n get workflow error: {}", error_text));
        }

        let data: Value = resp.json().await?;
        Ok(WorkflowStatus {
            id: data["id"].as_str().unwrap_or("").to_string(),
            name: data["name"].as_str().unwrap_or("").to_string(),
            active: data["active"].as_bool().unwrap_or(false),
            created_at: data["createdAt"].as_str().unwrap_or("").to_string(),
            updated_at: data["updatedAt"].as_str().unwrap_or("").to_string(),
        })
    }

    /// List all workflows
    pub async fn list_workflows(&self) -> Result<Vec<WorkflowStatus>> {
        let url = format!("{}/workflows", self.base_url);
        let resp = self
            .send_with_retry("n8n list_workflows", || {
                self.client.get(&url).header("X-N8N-API-KEY", &self.api_key)
            })
            .await?;

        if !resp.status().is_success() {
            let error_text = resp.text().await?;
            return Err(anyhow::anyhow!("n8n list workflows error: {}", error_text));
        }

        let data: Value = resp.json().await?;
        let workflows = data["data"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .map(|w| WorkflowStatus {
                        id: w["id"].as_str().unwrap_or("").to_string(),
                        name: w["name"].as_str().unwrap_or("").to_string(),
                        active: w["active"].as_bool().unwrap_or(false),
                        created_at: w["createdAt"].as_str().unwrap_or("").to_string(),
                        updated_at: w["updatedAt"].as_str().unwrap_or("").to_string(),
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok(workflows)
    }

    /// Execute a workflow manually
    pub async fn execute_workflow(&self, id: &str) -> Result<ExecutionResult> {
        let url = format!("{}/workflows/{}/run", self.base_url, id);
        let run_body = json!({});
        let resp = match self
            .send_with_retry("n8n execute_workflow", || {
                self.client
                    .post(&url)
                    .header("X-N8N-API-KEY", &self.api_key)
                    .json(&run_body)
            })
            .await
        {
            Ok(resp) => resp,
            Err(api_err) => {
                if self.local_target() {
                    match self.execute_workflow_cli(id).await {
                        Ok(execution) => {
                            println!(
                                "⚠️ n8n API execute failed ({}), recovered with CLI execute",
                                api_err
                            );
                            let _ = self.enable_disabled_webhook_nodes(id).await;
                            return Ok(execution);
                        }
                        Err(cli_err) => {
                            return Err(anyhow::anyhow!(
                                "n8n execute error: {} | CLI fallback failed: {}",
                                api_err,
                                cli_err
                            ));
                        }
                    }
                }
                return Err(anyhow::anyhow!("n8n execute error: {}", api_err));
            }
        };

        if !resp.status().is_success() {
            let error_text = resp.text().await?;
            if self.local_target() {
                match self.execute_workflow_cli(id).await {
                    Ok(execution) => {
                        println!(
                            "⚠️ n8n API execute failed ({}), recovered with CLI execute",
                            error_text
                        );
                        let _ = self.enable_disabled_webhook_nodes(id).await;
                        return Ok(execution);
                    }
                    Err(cli_err) => {
                        return Err(anyhow::anyhow!(
                            "n8n execute error: {} | CLI fallback failed: {}",
                            error_text,
                            cli_err
                        ));
                    }
                }
            }
            return Err(anyhow::anyhow!("n8n execute error: {}", error_text));
        }

        let data: Value = resp.json().await?;
        let result = ExecutionResult {
            id: data["id"].as_str().unwrap_or("").to_string(),
            finished: data["finished"].as_bool().unwrap_or(false),
            status: data["status"].as_str().unwrap_or("unknown").to_string(),
            started_at: data["startedAt"].as_str().unwrap_or("").to_string(),
            stopped_at: data["stoppedAt"].as_str().map(|s| s.to_string()),
        };
        let _ = self.enable_disabled_webhook_nodes(id).await;
        Ok(result)
    }

    /// List executions for a workflow
    pub async fn list_executions(
        &self,
        workflow_id: &str,
        limit: u32,
    ) -> Result<Vec<ExecutionResult>> {
        let url = format!(
            "{}/executions?workflowId={}&limit={}",
            self.base_url, workflow_id, limit
        );

        let resp = self
            .send_with_retry("n8n list_executions", || {
                self.client.get(&url).header("X-N8N-API-KEY", &self.api_key)
            })
            .await?;

        if !resp.status().is_success() {
            let error_text = resp.text().await?;
            return Err(anyhow::anyhow!("n8n list executions error: {}", error_text));
        }

        let data: Value = resp.json().await?;
        let executions = data["data"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .map(|e| ExecutionResult {
                        id: e["id"].as_str().unwrap_or("").to_string(),
                        finished: e["finished"].as_bool().unwrap_or(false),
                        status: e["status"].as_str().unwrap_or("unknown").to_string(),
                        started_at: e["startedAt"].as_str().unwrap_or("").to_string(),
                        stopped_at: e["stoppedAt"].as_str().map(|s| s.to_string()),
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok(executions)
    }

    /// Delete a workflow
    pub async fn delete_workflow(&self, id: &str) -> Result<()> {
        let url = format!("{}/workflows/{}", self.base_url, id);
        let resp = self
            .send_with_retry("n8n delete_workflow", || {
                self.client
                    .delete(&url)
                    .header("X-N8N-API-KEY", &self.api_key)
            })
            .await?;

        if !resp.status().is_success() {
            let error_text = resp.text().await?;
            return Err(anyhow::anyhow!("n8n delete error: {}", error_text));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    #[tokio::test]
    #[serial]
    async fn create_workflow_uses_mock_path_when_enabled() {
        unsafe {
            std::env::set_var("STEER_N8N_MOCK", "1");
        }
        let api = N8nApi::new("http://127.0.0.1:5678/api/v1", "");
        let wf = json!({
            "nodes": [],
            "connections": {}
        });
        let result = api.create_workflow("mock-test", &wf, true).await;
        unsafe {
            std::env::remove_var("STEER_N8N_MOCK");
        }

        assert!(result.is_ok());
        let id = result.unwrap_or_default();
        assert!(id.starts_with("mock-wf-"));
    }
}
