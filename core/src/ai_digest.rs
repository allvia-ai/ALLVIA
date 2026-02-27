use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::time::Duration;

const DEFAULT_REQUEST_TEXT: &str = "뉴스 5개 요약해서 노션에 정리해줘. 유튜브 링크 포함.";
const DEFAULT_RUNBOOK_ROOT: &str = "/tmp/steer_master_runbook";
const DEFAULT_WEBHOOK_TIMEOUT_SECS: u64 = 180;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiDigestTriggerResult {
    pub ok: bool,
    pub status_code: u16,
    pub webhook_url: String,
    pub scope_marker: String,
    pub notion_url: Option<String>,
    pub response_json: Option<Value>,
    pub response_text: Option<String>,
}

pub fn default_request_text() -> &'static str {
    DEFAULT_REQUEST_TEXT
}

pub fn normalize_request_text(raw: Option<&str>) -> String {
    let trimmed = raw.unwrap_or_default().trim();
    if trimmed.is_empty() {
        return DEFAULT_REQUEST_TEXT.to_string();
    }
    trimmed.to_string()
}

pub fn build_scope_marker() -> String {
    format!(
        "RUN_SCOPE_TELEGRAM_AI_DIGEST_{}",
        Utc::now().format("%Y%m%d_%H%M%S")
    )
}

pub fn looks_like_news_digest_request(message: &str) -> bool {
    let trimmed = message.trim();
    if trimmed.is_empty() {
        return false;
    }

    let lower = message.to_lowercase();
    let has_news = lower.contains("news")
        || lower.contains("headline")
        || lower.contains("trend")
        || lower.contains("digest")
        || message.contains("뉴스")
        || message.contains("기사")
        || message.contains("헤드라인")
        || message.contains("트렌드")
        || message.contains("브리핑");
    let has_notion = lower.contains("notion") || message.contains("노션");
    let has_digest = lower.contains("digest")
        || lower.contains("summary")
        || lower.contains("brief")
        || lower.contains("top")
        || lower.contains("latest")
        || message.contains("요약")
        || message.contains("정리")
        || message.contains("브리핑")
        || message.contains("선정")
        || message.contains("모아")
        || message.contains("저장");
    let has_youtube = lower.contains("youtube") || message.contains("유튜브");
    let has_count = Regex::new(r"(?i)(\d{1,2})\s*개|(?:top|latest)\s*(\d{1,2})")
        .ok()
        .is_some_and(|re| re.is_match(trimmed));

    has_news && has_notion && (has_digest || has_youtube || has_count)
}

pub fn looks_like_ai_digest_request(message: &str) -> bool {
    looks_like_news_digest_request(message)
}

pub fn extract_explicit_n8n_request(message: &str) -> Option<String> {
    let trimmed = message.trim();
    if trimmed.is_empty() {
        return None;
    }

    let exact_markers = ["/n8n", "n8n", "/workflow", "workflow", "/digest", "digest"];
    if exact_markers
        .iter()
        .any(|marker| trimmed.eq_ignore_ascii_case(marker))
    {
        return Some(DEFAULT_REQUEST_TEXT.to_string());
    }

    let prefix_re =
        Regex::new(r"(?i)^(?:/n8n|n8n|/workflow|workflow|/digest|digest)(?:\s+|[:\-\|]\s*)(.+)$")
            .ok();
    if let Some(re) = prefix_re {
        if let Some(captures) = re.captures(trimmed) {
            if let Some(rest) = captures.get(1) {
                let cleaned = rest.as_str().trim();
                if !cleaned.is_empty() {
                    return Some(cleaned.to_string());
                }
            }
            return Some(DEFAULT_REQUEST_TEXT.to_string());
        }
    }

    let lower = trimmed.to_lowercase();
    let explicit_n8n_hint = lower.contains("n8n으로")
        || lower.contains("workflow로")
        || lower.contains("#n8n")
        || lower.contains("use n8n")
        || lower.contains("via n8n");
    if explicit_n8n_hint && looks_like_news_digest_request(trimmed) {
        return Some(trimmed.to_string());
    }

    None
}

pub fn strip_local_execution_prefix(message: &str) -> String {
    let trimmed = message.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    let prefix_re =
        Regex::new(r"(?i)^(?:/local|local|/llm|llm|/surf|surf)(?:\s+|[:\-\|]\s*)(.+)$").ok();
    if let Some(re) = prefix_re {
        if let Some(captures) = re.captures(trimmed) {
            if let Some(rest) = captures.get(1) {
                let cleaned = rest.as_str().trim();
                if !cleaned.is_empty() {
                    return cleaned.to_string();
                }
            }
        }
    }

    trimmed.to_string()
}

fn env_non_empty(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn runbook_root() -> String {
    env_non_empty("STEER_MASTER_RUNBOOK_ROOT").unwrap_or_else(|| DEFAULT_RUNBOOK_ROOT.to_string())
}

fn resolve_workflow_id_from_runbook() -> Option<String> {
    let latest = PathBuf::from(runbook_root()).join("latest_run_dir.txt");
    let run_dir = std::fs::read_to_string(latest).ok()?;
    let run_dir = run_dir.trim();
    if run_dir.is_empty() {
        return None;
    }

    let status_json_path = PathBuf::from(run_dir).join("status.json");
    let raw = std::fs::read_to_string(status_json_path).ok()?;
    let status: Value = serde_json::from_str(&raw).ok()?;
    status
        .get("n8n_workflow_id")
        .and_then(|v| v.as_str())
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

pub fn resolve_program_webhook_url() -> Result<String> {
    if let Some(url) = env_non_empty("STEER_AI_DIGEST_PROGRAM_WEBHOOK_URL") {
        return Ok(url);
    }

    let workflow_id =
        env_non_empty("STEER_AI_DIGEST_WORKFLOW_ID").or_else(resolve_workflow_id_from_runbook);

    if let Some(workflow_id) = workflow_id {
        return Ok(format!(
            "http://localhost:5678/webhook/{}/programtrigger/ai-digest-program",
            workflow_id
        ));
    }

    Err(anyhow!(
        "AI digest webhook not configured. Set STEER_AI_DIGEST_PROGRAM_WEBHOOK_URL or \
STEER_AI_DIGEST_WORKFLOW_ID, or keep /tmp/steer_master_runbook status.json with n8n_workflow_id."
    ))
}

fn webhook_timeout_secs() -> u64 {
    std::env::var("STEER_AI_DIGEST_TIMEOUT_SEC")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .map(|v| v.clamp(30, 900))
        .unwrap_or(DEFAULT_WEBHOOK_TIMEOUT_SECS)
}

fn maybe_string(value: Option<&Value>) -> Option<String> {
    value
        .and_then(|v| v.as_str())
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn dedupe_push(vec: &mut Vec<String>, value: &str) {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return;
    }
    if vec.iter().any(|v| v.eq_ignore_ascii_case(trimmed)) {
        return;
    }
    vec.push(trimmed.to_string());
}

fn collect_string_fields_by_keys(payload: &Value, keys: &[&str], max: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![payload];
    while let Some(node) = stack.pop() {
        match node {
            Value::Object(map) => {
                for key in keys {
                    if let Some(value) = maybe_string(map.get(*key)) {
                        dedupe_push(&mut out, &value);
                        if out.len() >= max {
                            return out;
                        }
                    }
                }
                for value in map.values() {
                    stack.push(value);
                }
            }
            Value::Array(items) => {
                for item in items.iter().rev() {
                    stack.push(item);
                }
            }
            _ => {}
        }
    }
    out
}

fn extract_headline_lines(text: &str, out: &mut Vec<String>) {
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let normalized = trimmed
            .trim_start_matches("- ")
            .trim_start_matches("* ")
            .trim();
        let candidate = if let Some((num, rest)) = normalized.split_once('.') {
            if num.chars().all(|ch| ch.is_ascii_digit()) && !rest.trim().is_empty() {
                rest.trim()
            } else {
                normalized
            }
        } else {
            normalized
        };
        if candidate.is_empty() {
            continue;
        }
        if candidate.starts_with("http://") || candidate.starts_with("https://") {
            continue;
        }
        dedupe_push(out, candidate);
        if out.len() >= 5 {
            return;
        }
    }
}

fn extract_headlines(payload: &Value) -> Vec<String> {
    let mut out = Vec::new();

    for value in collect_string_fields_by_keys(payload, &["top_headlines_text"], 4) {
        extract_headline_lines(&value, &mut out);
        if out.len() >= 5 {
            return out;
        }
    }

    for value in collect_string_fields_by_keys(payload, &["telegram_summary"], 4) {
        extract_headline_lines(&value, &mut out);
        if out.len() >= 5 {
            return out;
        }
    }

    let mut stack = vec![payload];
    while let Some(node) = stack.pop() {
        match node {
            Value::Object(map) => {
                if let Some(Value::Array(items)) = map.get("top_headlines") {
                    for item in items {
                        if let Some(title) = item.as_str() {
                            dedupe_push(&mut out, title);
                            if out.len() >= 5 {
                                return out;
                            }
                        }
                    }
                }
                if let Some(title) = maybe_string(map.get("title")) {
                    let has_news_link = map
                        .get("source_url")
                        .and_then(|v| v.as_str())
                        .is_some()
                        || map.get("link").and_then(|v| v.as_str()).is_some();
                    if has_news_link {
                        dedupe_push(&mut out, &title);
                        if out.len() >= 5 {
                            return out;
                        }
                    }
                }
                for value in map.values() {
                    stack.push(value);
                }
            }
            Value::Array(items) => {
                for item in items.iter().rev() {
                    stack.push(item);
                }
            }
            _ => {}
        }
    }

    out
}

fn extract_notion_url(payload: &Value) -> Option<String> {
    let mut candidates = Vec::new();
    for key in ["notion_url", "notion_page_url", "page_url", "url"] {
        for value in collect_string_fields_by_keys(payload, &[key], 24) {
            dedupe_push(&mut candidates, &value);
        }
    }
    if let Some(value) = maybe_string(payload.pointer("/notion/url")) {
        dedupe_push(&mut candidates, &value);
    }
    if let Some(value) = maybe_string(payload.pointer("/result/notion_url")) {
        dedupe_push(&mut candidates, &value);
    }
    if let Some(value) = maybe_string(payload.pointer("/data/notion_url")) {
        dedupe_push(&mut candidates, &value);
    }

    for candidate in candidates {
        let trimmed = candidate
            .trim_matches(|c: char| c == '"' || c == '\'' || c == '`')
            .trim();
        if !(trimmed.starts_with("http://") || trimmed.starts_with("https://")) {
            continue;
        }
        if trimmed.to_ascii_lowercase().contains("notion.so") {
            return Some(trimmed.to_string());
        }
    }

    let raw = payload.to_string();
    let re = Regex::new(r#"https?://[^\s"\\]+notion\.so[^\s"\\]*"#).ok()?;
    let found = re.find(&raw)?.as_str().trim();
    Some(found.to_string())
}

fn extract_scope_marker(payload: &Value) -> Option<String> {
    collect_string_fields_by_keys(payload, &["scope_marker"], 4)
        .into_iter()
        .next()
        .or_else(|| maybe_string(payload.pointer("/result/scope_marker")))
        .or_else(|| maybe_string(payload.pointer("/data/scope_marker")))
}

fn extract_status(payload: &Value) -> Option<String> {
    collect_string_fields_by_keys(payload, &["status"], 4)
        .into_iter()
        .next()
        .or_else(|| maybe_string(payload.pointer("/result/status")))
        .or_else(|| maybe_string(payload.pointer("/data/status")))
        .map(|s| s.to_lowercase())
}

fn truncate_chars(raw: &str, max_chars: usize) -> String {
    if raw.chars().count() <= max_chars {
        return raw.to_string();
    }
    let mut out = String::new();
    for ch in raw.chars().take(max_chars) {
        out.push(ch);
    }
    out.push_str("...");
    out
}

pub async fn trigger_program_webhook(
    request_text: &str,
    scope_marker_override: Option<String>,
) -> Result<AiDigestTriggerResult> {
    let webhook_url = resolve_program_webhook_url()?;
    let request_text = normalize_request_text(Some(request_text));
    let scope_marker = scope_marker_override.unwrap_or_else(build_scope_marker);

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(webhook_timeout_secs()))
        .build()
        .context("failed to build HTTP client")?;

    let payload = json!({
        "text": request_text,
        "scope_marker": scope_marker,
        "source": "local_os_agent.program",
    });

    let response = client
        .post(&webhook_url)
        .json(&payload)
        .send()
        .await
        .with_context(|| format!("failed to call webhook {}", webhook_url))?;
    let status_code = response.status().as_u16();
    let body = response.text().await.unwrap_or_default();
    let parsed_json = serde_json::from_str::<Value>(&body).ok();

    if !(200..300).contains(&status_code) {
        return Err(anyhow!(
            "webhook returned HTTP {}: {}",
            status_code,
            truncate_chars(body.trim(), 600)
        ));
    }

    if let Some(status) = parsed_json.as_ref().and_then(extract_status) {
        if matches!(status.as_str(), "error" | "failed" | "failure") {
            return Err(anyhow!(
                "webhook returned failed status='{}': {}",
                status,
                truncate_chars(body.trim(), 600)
            ));
        }
    }

    let notion_url = parsed_json.as_ref().and_then(extract_notion_url);
    let scope_marker = parsed_json
        .as_ref()
        .and_then(extract_scope_marker)
        .unwrap_or(scope_marker);

    Ok(AiDigestTriggerResult {
        ok: true,
        status_code,
        webhook_url,
        scope_marker,
        notion_url,
        response_json: parsed_json,
        response_text: if body.trim().is_empty() {
            None
        } else {
            Some(truncate_chars(body.trim(), 1200))
        },
    })
}

pub fn format_human_summary(result: &AiDigestTriggerResult) -> String {
    let extracted_notion_url = result
        .response_json
        .as_ref()
        .and_then(extract_notion_url)
        .or_else(|| result.notion_url.clone());
    let highlights = result
        .response_json
        .as_ref()
        .map(extract_headlines)
        .unwrap_or_default();

    let mut lines = Vec::new();
    lines.push("상태: ✅ 완료".to_string());
    lines.push("결과: AI 트렌드 요약/노션 정리 실행됨".to_string());
    if let Some(url) = extracted_notion_url {
        lines.push(format!("노션 링크: {}", url));
    } else {
        lines.push("노션 링크: (응답에서 확인되지 않음)".to_string());
    }
    if !highlights.is_empty() {
        lines.push("핵심 뉴스:".to_string());
        for (idx, title) in highlights.iter().take(5).enumerate() {
            lines.push(format!("{}. {}", idx + 1, title));
        }
    }
    lines.push(format!("run marker: {}", result.scope_marker));
    lines.push(format!("webhook 응답: HTTP {}", result.status_code));
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn detects_ai_digest_request_keywords() {
        assert!(looks_like_news_digest_request(
            "AI뉴스 5개 요약해서 노션에 정리해줘"
        ));
        assert!(looks_like_news_digest_request(
            "Please create an AI news digest and send to notion"
        ));
        assert!(looks_like_news_digest_request(
            "스포츠 뉴스 5개 선정해서 노션에 정리해줘"
        ));
        assert!(!looks_like_news_digest_request("open finder"));
    }

    #[test]
    fn explicit_n8n_request_detection() {
        assert_eq!(
            extract_explicit_n8n_request("/n8n 스포츠 뉴스 5개 요약"),
            Some("스포츠 뉴스 5개 요약".to_string())
        );
        assert_eq!(
            extract_explicit_n8n_request("workflow: 경제 뉴스 3개 노션 정리"),
            Some("경제 뉴스 3개 노션 정리".to_string())
        );
        assert_eq!(
            extract_explicit_n8n_request("/n8n"),
            Some(DEFAULT_REQUEST_TEXT.to_string())
        );
        assert_eq!(
            extract_explicit_n8n_request("스포츠 뉴스 5개 노션에 정리해줘"),
            None
        );
    }

    #[test]
    fn local_prefix_strip_works() {
        assert_eq!(
            strip_local_execution_prefix("/local 스포츠 뉴스 5개 요약해줘"),
            "스포츠 뉴스 5개 요약해줘"
        );
        assert_eq!(
            strip_local_execution_prefix("LLM: 메모장 열고 테스트 입력"),
            "메모장 열고 테스트 입력"
        );
        assert_eq!(
            strip_local_execution_prefix("그냥 일반 요청"),
            "그냥 일반 요청"
        );
    }

    #[test]
    fn extracts_notion_url_from_nested_payload() {
        let payload = json!({
            "status": "completed",
            "result": {
                "notion_page_url": "https://www.notion.so/abc123def456"
            }
        });
        assert_eq!(
            extract_notion_url(&payload),
            Some("https://www.notion.so/abc123def456".to_string())
        );
    }

    #[test]
    fn format_human_summary_contains_headlines_and_notion_link() {
        let payload = json!({
            "status": "completed",
            "notion_page_url": "https://www.notion.so/page123",
            "top_headlines_text": "1. 헤드라인 A\n2. 헤드라인 B\n3. 헤드라인 C"
        });
        let result = AiDigestTriggerResult {
            ok: true,
            status_code: 200,
            webhook_url: "http://localhost:5678/webhook/test".to_string(),
            scope_marker: "RUN_SCOPE_TEST".to_string(),
            notion_url: None,
            response_json: Some(payload),
            response_text: None,
        };
        let summary = format_human_summary(&result);
        assert!(summary.contains("노션 링크: https://www.notion.so/page123"));
        assert!(summary.contains("핵심 뉴스:"));
        assert!(summary.contains("1. 헤드라인 A"));
    }

    #[test]
    #[serial]
    fn resolves_webhook_url_from_runbook_status() {
        let tmp = tempdir().expect("tempdir");
        let root = tmp.path().join("runbook");
        fs::create_dir_all(&root).expect("mkdir runbook");

        let run_dir = root.join("run_1");
        fs::create_dir_all(&run_dir).expect("mkdir run");
        fs::write(
            root.join("latest_run_dir.txt"),
            run_dir.to_string_lossy().to_string(),
        )
        .expect("write latest");
        fs::write(
            run_dir.join("status.json"),
            r#"{"n8n_workflow_id":"wf_123"}"#,
        )
        .expect("write status");

        std::env::set_var(
            "STEER_MASTER_RUNBOOK_ROOT",
            root.to_string_lossy().to_string(),
        );
        std::env::remove_var("STEER_AI_DIGEST_PROGRAM_WEBHOOK_URL");
        std::env::remove_var("STEER_AI_DIGEST_WORKFLOW_ID");

        let url = resolve_program_webhook_url().expect("resolve webhook");
        assert_eq!(
            url,
            "http://localhost:5678/webhook/wf_123/programtrigger/ai-digest-program"
        );

        std::env::remove_var("STEER_MASTER_RUNBOOK_ROOT");
    }
}
