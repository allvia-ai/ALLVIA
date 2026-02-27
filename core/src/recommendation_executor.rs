use crate::{db, env_flag, llm_gateway::LLMClient, n8n_api};
use anyhow::{anyhow, Result};
use reqwest::Method;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug, Clone, serde::Serialize)]
pub struct ApprovalExecutionOutcome {
    pub workflow_id: String,
    pub approved_now: bool,
    pub reused_existing: bool,
}

#[derive(Debug, Clone)]
pub struct PreclaimedProvisioning {
    pub claim_token: Option<String>,
    pub provision_op_id: i64,
    pub force_recreate: bool,
}

fn mock_workflow_json(name: &str) -> serde_json::Value {
    n8n_api::build_orchestrator_fallback_workflow(name, None, "test_mock_template")
}

fn workflow_has_nodes(value: &serde_json::Value) -> bool {
    value
        .get("nodes")
        .and_then(|n| n.as_array())
        .map(|nodes| !nodes.is_empty())
        .unwrap_or(false)
}

fn mark_provision_progress(op_id: i64, detail: &str) {
    if let Err(error) = db::mark_workflow_provision_in_progress(op_id, Some(detail)) {
        eprintln!(
            "⚠️ Failed to update workflow provision progress: op_id={} detail='{}' error={}",
            op_id, detail, error
        );
    }
}

fn parse_bool_env(key: &str, default: bool) -> bool {
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

fn n8n_create_active_default() -> bool {
    // Default to active so production webhook routes are immediately available.
    parse_bool_env("STEER_N8N_ACTIVE_ON_CREATE", true)
}

fn auto_trigger_on_approve_default() -> bool {
    parse_bool_env("STEER_N8N_AUTO_TRIGGER_ON_APPROVE", true)
}

fn auto_trigger_timeout_secs() -> u64 {
    std::env::var("STEER_N8N_AUTO_TRIGGER_TIMEOUT_SEC")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .map(|v| v.clamp(3, 60))
        .unwrap_or(8)
}

fn auto_trigger_retry_attempts() -> u32 {
    std::env::var("STEER_N8N_AUTO_TRIGGER_RETRIES")
        .ok()
        .and_then(|v| v.trim().parse::<u32>().ok())
        .map(|v| v.clamp(1, 12))
        .unwrap_or(6)
}

fn auto_trigger_retry_delay_ms() -> u64 {
    std::env::var("STEER_N8N_AUTO_TRIGGER_RETRY_DELAY_MS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .map(|v| v.clamp(200, 5_000))
        .unwrap_or(600)
}

fn auto_trigger_execution_wait_secs() -> u64 {
    std::env::var("STEER_N8N_AUTO_TRIGGER_EXECUTION_WAIT_SEC")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .map(|v| v.clamp(10, 900))
        .unwrap_or(120)
}

fn auto_trigger_execution_poll_ms() -> u64 {
    std::env::var("STEER_N8N_AUTO_TRIGGER_EXECUTION_POLL_MS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .map(|v| v.clamp(300, 10_000))
        .unwrap_or(1_500)
}

fn is_execution_success_status(status: &str) -> bool {
    matches!(
        status.trim().to_ascii_lowercase().as_str(),
        "success" | "completed"
    )
}

fn is_execution_failure_status(status: &str) -> bool {
    matches!(
        status.trim().to_ascii_lowercase().as_str(),
        "error" | "failed" | "failure" | "crashed" | "cancelled" | "canceled" | "timeout"
    )
}

async fn wait_for_execution_success(
    n8n: &n8n_api::N8nApi,
    workflow_id: &str,
    execution_id_hint: Option<&str>,
) -> Result<n8n_api::ExecutionResult> {
    let timeout_sec = auto_trigger_execution_wait_secs();
    let poll_ms = auto_trigger_execution_poll_ms();
    let deadline = std::time::Instant::now() + Duration::from_secs(timeout_sec);
    let hinted = execution_id_hint
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(|v| v.to_string());

    let mut last_seen: Option<n8n_api::ExecutionResult> = None;
    while std::time::Instant::now() <= deadline {
        let executions = n8n.list_executions(workflow_id, 20).await?;
        let selected = if let Some(hint) = hinted.as_deref() {
            executions.into_iter().find(|e| e.id == hint)
        } else {
            executions.into_iter().next()
        };

        if let Some(exec) = selected {
            if exec.finished {
                if is_execution_success_status(&exec.status) {
                    return Ok(exec);
                }
                return Err(anyhow!(
                    "n8n execution finished with non-success status: workflow_id={} execution_id={} status={}",
                    workflow_id,
                    exec.id,
                    exec.status
                ));
            }
            if is_execution_failure_status(&exec.status) {
                return Err(anyhow!(
                    "n8n execution entered failure status before finish: workflow_id={} execution_id={} status={}",
                    workflow_id,
                    exec.id,
                    exec.status
                ));
            }
            last_seen = Some(exec);
        }
        tokio::time::sleep(Duration::from_millis(poll_ms)).await;
    }

    if let Some(last) = last_seen {
        return Err(anyhow!(
            "n8n execution completion timeout: workflow_id={} execution_id={} status={} finished={}",
            workflow_id,
            last.id,
            last.status,
            last.finished
        ));
    }
    Err(anyhow!(
        "n8n execution completion timeout: workflow_id={} (no execution observed)",
        workflow_id
    ))
}

fn normalize_localhost_base_url(input: &str) -> String {
    let mut base = input.trim().trim_end_matches('/').to_string();
    for (from, to) in [
        ("http://127.0.0.1", "http://localhost"),
        ("https://127.0.0.1", "https://localhost"),
        ("http://0.0.0.0", "http://localhost"),
        ("https://0.0.0.0", "https://localhost"),
        ("http://[::1]", "http://localhost"),
        ("https://[::1]", "https://localhost"),
    ] {
        if base.starts_with(from) {
            base = base.replacen(from, to, 1);
            break;
        }
    }
    base
}

fn resolve_n8n_webhook_base_url() -> String {
    if let Ok(raw) = std::env::var("STEER_N8N_WEBHOOK_BASE_URL") {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            return normalize_localhost_base_url(trimmed);
        }
    }
    if let Ok(raw) = std::env::var("N8N_EDITOR_URL") {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            return normalize_localhost_base_url(trimmed);
        }
    }
    let api = std::env::var("STEER_N8N_API_URL")
        .or_else(|_| std::env::var("N8N_API_URL"))
        .ok()
        .map(|v| v.trim().trim_end_matches('/').to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "http://localhost:5678/api/v1".to_string());
    for suffix in ["/api/v1", "/api/v2", "/api"] {
        if let Some(stripped) = api.strip_suffix(suffix) {
            let candidate = stripped.trim_end_matches('/');
            if !candidate.is_empty() {
                return normalize_localhost_base_url(candidate);
            }
        }
    }
    normalize_localhost_base_url(&api)
}

#[derive(Debug, Clone)]
struct WebhookTarget {
    method: Method,
    path: String,
    node_name: String,
}

fn parse_http_method(raw: Option<&str>) -> Method {
    let candidate = raw.unwrap_or("POST").trim().to_uppercase();
    Method::from_bytes(candidate.as_bytes()).unwrap_or(Method::POST)
}

fn extract_program_webhook_target(workflow: &Value) -> Option<WebhookTarget> {
    let nodes = workflow.get("nodes")?.as_array()?;
    let mut fallback: Option<WebhookTarget> = None;
    for node in nodes {
        let node_type = node.get("type").and_then(|v| v.as_str()).unwrap_or("");
        if node_type != "n8n-nodes-base.webhook" {
            continue;
        }
        let params = node.get("parameters").and_then(|v| v.as_object());
        let raw_path = params
            .and_then(|p| p.get("path"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .trim_start_matches('/')
            .to_string();
        if raw_path.is_empty() {
            continue;
        }
        let method = parse_http_method(
            params
                .and_then(|p| p.get("httpMethod").and_then(|v| v.as_str()))
                .or_else(|| params.and_then(|p| p.get("method").and_then(|v| v.as_str()))),
        );
        let candidate = WebhookTarget {
            method,
            path: raw_path,
            node_name: node
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_string(),
        };
        let name = node
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_lowercase();
        if name.contains("program webhook") || name.contains("programwebhook") {
            return Some(candidate);
        }
        if fallback.is_none() {
            fallback = Some(candidate);
        }
    }
    fallback
}

async fn activate_and_auto_trigger_workflow(
    recommendation_id: i64,
    workflow_id: &str,
    workflow_json: Option<&Value>,
    trigger_payload_hint: Option<&str>,
) -> Result<()> {
    if env_flag("STEER_N8N_MOCK") || !auto_trigger_on_approve_default() {
        return Ok(());
    }

    let n8n = n8n_api::N8nApi::from_env()?;
    n8n.activate_workflow(workflow_id).await?;

    let mut webhook_trigger_sent = false;
    if let Some(target) = workflow_json.and_then(extract_program_webhook_target) {
        let base = resolve_n8n_webhook_base_url();
        let node_segment = if target.node_name.trim().is_empty() {
            "programwebhook".to_string()
        } else {
            target
                .node_name
                .trim()
                .to_lowercase()
                .replace([' ', '\t'], "")
        };
        let legacy_node_segment_for_url = "program%2520webhook".to_string();
        let compact_node_segment_for_url = node_segment.replace('%', "%25");
        let path_segment_for_url = target.path.replace('%', "%25");
        let candidate_urls = [
            format!("{}/webhook/{}", base, target.path),
            format!(
                "{}/webhook/{}/{}/{}",
                base, workflow_id, compact_node_segment_for_url, path_segment_for_url
            ),
            format!(
                "{}/webhook/{}/{}/{}",
                base, workflow_id, legacy_node_segment_for_url, path_segment_for_url
            ),
        ];
        let prompt = trigger_payload_hint
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| format!("recommendation {}", recommendation_id));
        let notion_parent_page_id = std::env::var("NOTION_PAGE_ID")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .unwrap_or_default();
        let notion_token = std::env::var("NOTION_TOKEN")
            .ok()
            .or_else(|| std::env::var("NOTION_API_KEY").ok())
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .unwrap_or_default();
        let telegram_chat_id = std::env::var("TELEGRAM_CHAT_ID")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .unwrap_or_default();
        let telegram_bot_token = std::env::var("TELEGRAM_BOT_TOKEN")
            .ok()
            .or_else(|| std::env::var("TELEGRAM_ACCESS_TOKEN").ok())
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .unwrap_or_default();
        let payload = json!({
            "source": "allvia.auto_approve",
            "recommendation_id": recommendation_id,
            "workflow_id": workflow_id,
            "triggered_at": chrono::Utc::now().to_rfc3339(),
            "prompt": prompt,
            "request_text": prompt,
            "top_n": 5,
            "notion_parent_page_id": notion_parent_page_id,
            "notion_token": notion_token,
            "telegram_chat_id": telegram_chat_id,
            "telegram_bot_token": telegram_bot_token,
        });
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(auto_trigger_timeout_secs()))
            .build()?;
        let retry_attempts = auto_trigger_retry_attempts();
        let retry_delay_ms = auto_trigger_retry_delay_ms();
        let mut webhook_errors: Vec<String> = Vec::new();
        'trigger_attempts: for attempt in 1..=retry_attempts {
            for url in candidate_urls.iter() {
                let response = match client
                    .request(target.method.clone(), url)
                    .json(&payload)
                    .send()
                    .await
                {
                    Ok(resp) => resp,
                    Err(error) => {
                        webhook_errors.push(format!(
                            "attempt={} {} {} => request error: {}",
                            attempt, target.method, url, error
                        ));
                        continue;
                    }
                };
                let status = response.status();
                if status.is_success() {
                    println!(
                        "✅ Auto webhook trigger sent: recommendation={} workflow_id={} method={} url={} attempt={}",
                        recommendation_id, workflow_id, target.method, url, attempt
                    );
                    webhook_trigger_sent = true;
                    break 'trigger_attempts;
                }
                let body = response.text().await.unwrap_or_default();
                webhook_errors.push(format!(
                    "attempt={} {} {} => HTTP {} ({})",
                    attempt,
                    target.method,
                    url,
                    status.as_u16(),
                    body
                ));
            }
            if attempt < retry_attempts {
                tokio::time::sleep(Duration::from_millis(retry_delay_ms * (attempt as u64))).await;
            }
        }
        if !webhook_trigger_sent {
            eprintln!(
                "⚠️ Auto webhook trigger failed for recommendation={} workflow_id={}: {}",
                recommendation_id,
                workflow_id,
                webhook_errors.join(" | ")
            );
        }
    }

    if webhook_trigger_sent {
        match wait_for_execution_success(&n8n, workflow_id, None).await {
            Ok(execution) => {
                println!(
                    "✅ Auto webhook execution completed: recommendation={} workflow_id={} execution_id={} status={}",
                    recommendation_id, workflow_id, execution.id, execution.status
                );
                return Ok(());
            }
            Err(e) => {
                eprintln!(
                    "⚠️ Webhook trigger sent but completion not observed: recommendation={} workflow_id={} error={}. Falling back to direct execute.",
                    recommendation_id, workflow_id, e
                );
            }
        }
    }

    let execution = n8n.execute_workflow(workflow_id).await?;
    if execution.finished {
        if is_execution_success_status(&execution.status) {
            println!(
                "✅ Auto execute fallback completed immediately: recommendation={} workflow_id={} execution_id={} status={}",
                recommendation_id, workflow_id, execution.id, execution.status
            );
            return Ok(());
        }
        return Err(anyhow!(
            "n8n execute returned finished non-success status: workflow_id={} execution_id={} status={}",
            workflow_id,
            execution.id,
            execution.status
        ));
    }
    let execution_id = execution.id.trim().to_string();
    let completed = wait_for_execution_success(
        &n8n,
        workflow_id,
        if execution_id.is_empty() {
            None
        } else {
            Some(execution_id.as_str())
        },
    )
    .await?;
    println!(
        "✅ Auto execute fallback completed: recommendation={} workflow_id={} execution_id={} status={}",
        recommendation_id, workflow_id, completed.id, completed.status
    );
    Ok(())
}

fn should_use_test_mock_workflow() -> bool {
    env_flag("STEER_TEST_ASSUME_APPROVED") && env_flag("STEER_N8N_MOCK")
}

fn provisioning_claim_ttl_millis() -> i64 {
    std::env::var("STEER_PROVISIONING_CLAIM_TTL_SECONDS")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .map(|seconds| seconds.clamp(1, 86_400) * 1_000)
        .unwrap_or(10 * 60 * 1_000)
}

fn parse_claim_timestamp_millis(token: &str) -> Option<i64> {
    if !token.starts_with("provisioning:") {
        return None;
    }
    token.rsplit(':').next()?.parse::<i64>().ok()
}

fn is_stale_provisioning_claim(token: &str) -> bool {
    let ts = match parse_claim_timestamp_millis(token) {
        Some(v) => v,
        None => return false,
    };
    let now = chrono::Utc::now().timestamp_millis();
    now.saturating_sub(ts) > provisioning_claim_ttl_millis()
}

fn make_claim_token(id: i64) -> String {
    format!(
        "provisioning:{}:{}",
        id,
        chrono::Utc::now().timestamp_millis()
    )
}

fn claim_or_get_existing(id: i64, claim_token: &str) -> Result<Option<String>> {
    for _ in 0..2 {
        match db::claim_recommendation_provisioning(id, claim_token)? {
            Some(existing) => {
                let trimmed = existing.trim();
                if trimmed.is_empty() {
                    continue;
                }
                if trimmed == claim_token {
                    return Ok(None);
                }
                if trimmed.starts_with("provisioning:") {
                    if is_stale_provisioning_claim(trimmed) {
                        let _ = db::release_recommendation_provisioning_claim(id, trimmed);
                        continue;
                    }
                    return Err(anyhow!(
                        "recommendation {} is already being provisioned ({})",
                        id,
                        trimmed
                    ));
                }
                return Ok(Some(trimmed.to_string()));
            }
            None => return Ok(None),
        }
    }
    Err(anyhow!(
        "failed to acquire provisioning claim for recommendation {}",
        id
    ))
}

pub fn precreate_async_provisioning(id: i64) -> Result<PreclaimedProvisioning> {
    let rec =
        db::get_recommendation(id)?.ok_or_else(|| anyhow!("recommendation {} not found", id))?;

    if rec.status.eq_ignore_ascii_case("rejected") {
        return Err(anyhow!(
            "recommendation {} is rejected and cannot be created",
            id
        ));
    }
    if !rec.status.eq_ignore_ascii_case("approved") {
        return Err(anyhow!(
            "recommendation {} is '{}' (approval required before creation)",
            id,
            rec.status
        ));
    }

    let force_recreate = env_flag("STEER_APPROVE_FORCE_RECREATE");
    let claim_token = if force_recreate {
        None
    } else {
        Some(make_claim_token(id))
    };

    if !force_recreate {
        if let Some(existing_id) = rec
            .workflow_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            if existing_id.starts_with("provisioning:") {
                let our_token = claim_token
                    .as_deref()
                    .ok_or_else(|| anyhow!("missing provisioning claim token"))?;
                if existing_id != our_token {
                    if is_stale_provisioning_claim(existing_id) {
                        let _ = db::release_recommendation_provisioning_claim(id, existing_id);
                    } else {
                        return Err(anyhow!(
                            "recommendation {} is already being provisioned ({})",
                            id,
                            existing_id
                        ));
                    }
                }
            } else {
                return Err(anyhow!(
                    "recommendation {} already provisioned ({})",
                    id,
                    existing_id
                ));
            }
        }

        let token = claim_token
            .as_deref()
            .ok_or_else(|| anyhow!("missing provisioning claim token"))?;
        if let Some(existing) = claim_or_get_existing(id, token)? {
            return Err(anyhow!(
                "recommendation {} already provisioned ({})",
                id,
                existing
            ));
        }
    }

    let provision_op_id = db::create_workflow_provision_op(id, claim_token.as_deref())?;
    Ok(PreclaimedProvisioning {
        claim_token,
        provision_op_id,
        force_recreate,
    })
}

pub fn maybe_assume_approved_for_test(id: i64) -> Result<()> {
    if !env_flag("STEER_TEST_ASSUME_APPROVED") {
        return Err(anyhow!(
            "STEER_TEST_ASSUME_APPROVED=1 is required for approve_test path"
        ));
    }
    if !env_flag("STEER_N8N_MOCK") {
        return Err(anyhow!(
            "approve_test path requires STEER_N8N_MOCK=1 to prevent real workflow creation"
        ));
    }

    let rec =
        db::get_recommendation(id)?.ok_or_else(|| anyhow!("recommendation {} not found", id))?;
    if rec.status.eq_ignore_ascii_case("rejected") {
        return Err(anyhow!(
            "recommendation {} is rejected and cannot be assumed approved",
            id
        ));
    }
    if !rec.status.eq_ignore_ascii_case("approved") {
        db::update_recommendation_review_status(id, "approved")?;
        println!("🧪 [TEST] Assumed approval for recommendation {}.", id);
    }
    Ok(())
}

pub async fn execute_approved_recommendation(
    id: i64,
    llm_client: Option<Arc<dyn LLMClient>>,
) -> Result<String> {
    execute_approved_recommendation_internal(id, llm_client, None).await
}

pub async fn execute_approved_recommendation_with_preclaim(
    id: i64,
    llm_client: Option<Arc<dyn LLMClient>>,
    preclaim: PreclaimedProvisioning,
) -> Result<String> {
    execute_approved_recommendation_internal(id, llm_client, Some(preclaim)).await
}

async fn execute_approved_recommendation_internal(
    id: i64,
    llm_client: Option<Arc<dyn LLMClient>>,
    preclaim: Option<PreclaimedProvisioning>,
) -> Result<String> {
    let rec =
        db::get_recommendation(id)?.ok_or_else(|| anyhow!("recommendation {} not found", id))?;

    if rec.status.eq_ignore_ascii_case("rejected") {
        return Err(anyhow!(
            "recommendation {} is rejected and cannot be created",
            id
        ));
    }
    if !rec.status.eq_ignore_ascii_case("approved") {
        return Err(anyhow!(
            "recommendation {} is '{}' (approval required before creation)",
            id,
            rec.status
        ));
    }

    // Idempotency guard: if this recommendation already has a workflow id,
    // do not create another workflow unless explicitly forced.
    let force_recreate = preclaim
        .as_ref()
        .map(|p| p.force_recreate)
        .unwrap_or_else(|| env_flag("STEER_APPROVE_FORCE_RECREATE"));
    let claim_token = if force_recreate {
        None
    } else {
        preclaim
            .as_ref()
            .and_then(|p| p.claim_token.clone())
            .or_else(|| Some(make_claim_token(id)))
    };

    if !force_recreate {
        if let Some(existing_id) = rec
            .workflow_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            if existing_id.starts_with("provisioning:") {
                let our_token = claim_token
                    .as_deref()
                    .ok_or_else(|| anyhow!("missing provisioning claim token"))?;
                if existing_id != our_token {
                    if is_stale_provisioning_claim(existing_id) {
                        let _ = db::release_recommendation_provisioning_claim(id, existing_id);
                    } else {
                        return Err(anyhow!(
                            "recommendation {} is already being provisioned ({})",
                            id,
                            existing_id
                        ));
                    }
                }
            }
            if !existing_id.starts_with("provisioning:") {
                if let Some(pre) = preclaim.as_ref() {
                    let _ = db::commit_workflow_provision_success(
                        pre.provision_op_id,
                        id,
                        existing_id,
                        rec.workflow_json.as_deref(),
                    );
                }
                let existing_json = rec
                    .workflow_json
                    .as_deref()
                    .and_then(|raw| serde_json::from_str::<Value>(raw).ok());
                if let Err(error) = activate_and_auto_trigger_workflow(
                    id,
                    existing_id,
                    existing_json.as_ref(),
                    Some(&rec.n8n_prompt),
                )
                .await
                {
                    if let Some(pre) = preclaim.as_ref() {
                        let _ = db::mark_workflow_provision_failed(
                            pre.provision_op_id,
                            &format!("existing workflow auto-trigger failed: {}", error),
                        );
                    }
                    let _ = db::mark_recommendation_failed(
                        id,
                        &format!("existing workflow auto-trigger failed: {}", error),
                    );
                    return Err(anyhow!(
                        "existing workflow auto-trigger failed: recommendation={} workflow_id={} error={}",
                        id,
                        existing_id,
                        error
                    ));
                }
                println!(
                    "ℹ️ Recommendation {} already provisioned. Reusing workflow_id={}",
                    id, existing_id
                );
                return Ok(existing_id.to_string());
            }
        }

        let token = claim_token
            .as_deref()
            .ok_or_else(|| anyhow!("missing provisioning claim token"))?;
        if let Some(existing) = claim_or_get_existing(id, token)? {
            if let Some(pre) = preclaim.as_ref() {
                let _ = db::commit_workflow_provision_success(
                    pre.provision_op_id,
                    id,
                    &existing,
                    rec.workflow_json.as_deref(),
                );
            }
            let existing_json = rec
                .workflow_json
                .as_deref()
                .and_then(|raw| serde_json::from_str::<Value>(raw).ok());
            if let Err(error) = activate_and_auto_trigger_workflow(
                id,
                &existing,
                existing_json.as_ref(),
                Some(&rec.n8n_prompt),
            )
            .await
            {
                if let Some(pre) = preclaim.as_ref() {
                    let _ = db::mark_workflow_provision_failed(
                        pre.provision_op_id,
                        &format!("existing workflow auto-trigger failed: {}", error),
                    );
                }
                let _ = db::mark_recommendation_failed(
                    id,
                    &format!("existing workflow auto-trigger failed: {}", error),
                );
                return Err(anyhow!(
                    "existing workflow auto-trigger failed: recommendation={} workflow_id={} error={}",
                    id,
                    existing,
                    error
                ));
            }
            println!(
                "ℹ️ Recommendation {} already provisioned. Reusing workflow_id={}",
                id, existing
            );
            return Ok(existing);
        }
    }

    let provision_op_id = match preclaim.as_ref() {
        Some(p) => p.provision_op_id,
        None => db::create_workflow_provision_op(id, claim_token.as_deref())?,
    };
    mark_provision_progress(provision_op_id, "provisioning_started");

    let workflow_json_str_result: Result<String> = async {
        mark_provision_progress(provision_op_id, "preparing_workflow_json");
        if should_use_test_mock_workflow() {
            return serde_json::to_string(&mock_workflow_json(&rec.title))
                .map_err(|e| anyhow!("mock workflow serialization failed: {}", e));
        }

        if let Some(existing_json) = rec
            .workflow_json
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            return Ok(existing_json.to_string());
        }

        let brain = llm_client
            .clone()
            .ok_or_else(|| anyhow!("LLM Client not available"))?;
        let generated = brain
            .build_n8n_workflow(&rec.n8n_prompt)
            .await
            .map_err(|e| anyhow!("workflow generation failed: {}", e))?;
        mark_provision_progress(provision_op_id, "workflow_json_generated");
        Ok(generated)
    }
    .await;
    let workflow_json_str = match workflow_json_str_result {
        Ok(v) => v,
        Err(e) => {
            if parse_bool_env("STEER_N8N_MINIMAL_ON_LLM_FAILURE", true) {
                eprintln!(
                    "⚠️ workflow generation failed for recommendation {}: {}. Falling back to orchestrator template.",
                    id, e
                );
                match serde_json::to_string(&n8n_api::build_orchestrator_fallback_workflow(
                    &rec.title,
                    Some(&rec.n8n_prompt),
                    "llm_generation_failed",
                )) {
                    Ok(fallback) => fallback,
                    Err(serr) => {
                        let _ = db::mark_workflow_provision_failed(
                            provision_op_id,
                            &format!(
                                "workflow json generation failed: {}; fallback serialization failed: {}",
                                e, serr
                            ),
                        );
                        if !force_recreate {
                            if let Some(token) = claim_token.as_deref() {
                                let _ = db::release_recommendation_provisioning_claim(id, token);
                            }
                        }
                        return Err(anyhow!(
                            "workflow generation failed: {}; fallback serialization failed: {}",
                            e,
                            serr
                        ));
                    }
                }
            } else {
                let _ = db::mark_workflow_provision_failed(
                    provision_op_id,
                    &format!("workflow json generation failed: {}", e),
                );
                if !force_recreate {
                    if let Some(token) = claim_token.as_deref() {
                        let _ = db::release_recommendation_provisioning_claim(id, token);
                    }
                }
                return Err(e);
            }
        }
    };

    let workflow_val_result = serde_json::from_str::<serde_json::Value>(&workflow_json_str)
        .map_err(|e| {
            anyhow!(
                "generated workflow JSON is invalid for recommendation {}: {}",
                id,
                e
            )
        });
    let mut workflow_val = match workflow_val_result {
        Ok(v) => v,
        Err(e) => {
            let _ = db::mark_workflow_provision_failed(
                provision_op_id,
                &format!("workflow json parse failed: {}", e),
            );
            if !force_recreate {
                if let Some(token) = claim_token.as_deref() {
                    let _ = db::release_recommendation_provisioning_claim(id, token);
                }
            }
            return Err(e);
        }
    };

    if !workflow_has_nodes(&workflow_val) {
        eprintln!(
            "⚠️ workflow for recommendation {} had empty/missing nodes before normalization.",
            id
        );
    }
    // Keep local persisted JSON + auto-trigger extraction aligned with n8n create-time normalization.
    workflow_val = match n8n_api::normalize_workflow_for_create(&rec.title, &workflow_val) {
        Ok(v) => v,
        Err(e) => {
            let _ = db::mark_workflow_provision_failed(
                provision_op_id,
                &format!("workflow normalization failed: {}", e),
            );
            if !force_recreate {
                if let Some(token) = claim_token.as_deref() {
                    let _ = db::release_recommendation_provisioning_claim(id, token);
                }
            }
            return Err(anyhow!(
                "workflow normalization failed for recommendation {}: {}",
                id,
                e
            ));
        }
    };
    let workflow_json_str = serde_json::to_string(&workflow_val).map_err(|e| {
        anyhow!(
            "workflow serialization failed for recommendation {}: {}",
            id,
            e
        )
    })?;
    mark_provision_progress(provision_op_id, "workflow_json_normalized");

    let n8n = match n8n_api::N8nApi::from_env() {
        Ok(v) => v,
        Err(e) => {
            let _ = db::mark_workflow_provision_failed(
                provision_op_id,
                &format!("n8n initialization failed: {}", e),
            );
            if !force_recreate {
                if let Some(token) = claim_token.as_deref() {
                    let _ = db::release_recommendation_provisioning_claim(id, token);
                }
            }
            return Err(e);
        }
    };
    let active = n8n_create_active_default();
    mark_provision_progress(provision_op_id, "creating_workflow_in_n8n");

    match n8n.create_workflow(&rec.title, &workflow_val, active).await {
        Ok(workflow_id) => {
            mark_provision_progress(provision_op_id, "workflow_created");
            if let Err(e) = db::mark_workflow_provision_created(
                provision_op_id,
                &workflow_id,
                Some(&workflow_json_str),
            ) {
                let _ = db::mark_workflow_provision_reconcile_needed(
                    provision_op_id,
                    &format!("workflow created but op log update failed: {}", e),
                );
                return Err(anyhow!(
                    "workflow created (id={}) but failed to persist operation log: {}",
                    workflow_id,
                    e
                ));
            }
            if let Err(error) = activate_and_auto_trigger_workflow(
                id,
                &workflow_id,
                Some(&workflow_val),
                Some(&rec.n8n_prompt),
            )
            .await
            {
                let _ = db::mark_workflow_provision_failed(
                    provision_op_id,
                    &format!(
                        "workflow created but auto activate/trigger failed: {}",
                        error
                    ),
                );
                let _ = db::mark_recommendation_failed(
                    id,
                    &format!(
                        "workflow created but auto activate/trigger failed: {}",
                        error
                    ),
                );
                if !force_recreate {
                    if let Some(token) = claim_token.as_deref() {
                        let _ = db::release_recommendation_provisioning_claim(id, token);
                    }
                }
                return Err(anyhow!(
                    "workflow created but auto activate/trigger failed: recommendation={} workflow_id={} error={}",
                    id,
                    workflow_id,
                    error
                ));
            }
            if let Err(e) = db::commit_workflow_provision_success(
                provision_op_id,
                id,
                &workflow_id,
                Some(&workflow_json_str),
            ) {
                let _ = db::mark_workflow_provision_reconcile_needed(
                    provision_op_id,
                    &format!("workflow executed but recommendation commit failed: {}", e),
                );
                let _ = db::mark_recommendation_failed(
                    id,
                    &format!("workflow executed but commit failed: {}", e),
                );
                return Err(anyhow!(
                    "workflow executed (id={}) but local commit failed: {}",
                    workflow_id,
                    e
                ));
            }
            println!(
                "✅ Workflow execution completed: recommendation={} workflow_id={}",
                id, workflow_id
            );
            Ok(workflow_id)
        }
        Err(e) => {
            let _ = db::mark_workflow_provision_failed(
                provision_op_id,
                &format!("workflow creation failed: {}", e),
            );
            if !force_recreate {
                if let Some(token) = claim_token.as_deref() {
                    let _ = db::release_recommendation_provisioning_claim(id, token);
                }
            }
            let _ = db::mark_recommendation_failed(id, &e.to_string());
            Err(anyhow!("workflow creation failed: {}", e))
        }
    }
}

pub async fn approve_and_execute_recommendation(
    id: i64,
    llm_client: Option<Arc<dyn LLMClient>>,
) -> Result<ApprovalExecutionOutcome> {
    let rec =
        db::get_recommendation(id)?.ok_or_else(|| anyhow!("recommendation {} not found", id))?;

    if rec.status.eq_ignore_ascii_case("rejected") {
        return Err(anyhow!(
            "recommendation {} is rejected and cannot be approved",
            id
        ));
    }

    let approved_now = !rec.status.eq_ignore_ascii_case("approved");
    if approved_now {
        db::update_recommendation_review_status(id, "approved")?;
    }

    let preexisting_workflow = rec
        .workflow_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .filter(|s| !s.starts_with("provisioning:"))
        .map(|s| s.to_string());

    let workflow_id = execute_approved_recommendation(id, llm_client).await?;
    let reused_existing = preexisting_workflow
        .as_deref()
        .map(|existing| existing == workflow_id)
        .unwrap_or(false);

    Ok(ApprovalExecutionOutcome {
        workflow_id,
        approved_now,
        reused_existing,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recommendation::AutomationProposal;
    use serial_test::serial;

    fn insert_test_recommendation(title: &str) -> Option<i64> {
        let _ = db::init();
        let proposal = AutomationProposal {
            title: title.to_string(),
            summary: "test summary".to_string(),
            trigger: format!("trigger-{}", title),
            actions: vec!["noop".to_string()],
            confidence: 0.9,
            n8n_prompt: format!("Create workflow {}", title),
            evidence: vec!["test".to_string()],
            pattern_id: None,
        };
        let _ = db::insert_recommendation(&proposal);
        db::get_recommendations_with_filter(Some("pending"))
            .ok()
            .and_then(|recs| recs.into_iter().find(|r| r.title == title))
            .map(|r| r.id)
    }

    #[test]
    #[serial]
    fn test_assume_approved_requires_flag() {
        std::env::remove_var("STEER_TEST_ASSUME_APPROVED");
        let result = maybe_assume_approved_for_test(1);
        assert!(result.is_err());
    }

    #[test]
    #[serial]
    fn test_assume_approved_requires_mock_flag() {
        std::env::set_var("STEER_TEST_ASSUME_APPROVED", "1");
        std::env::remove_var("STEER_N8N_MOCK");
        let result = maybe_assume_approved_for_test(1);
        assert!(result.is_err());
    }

    #[tokio::test]
    #[serial]
    async fn test_execute_requires_approved_status() {
        let title = format!(
            "rec-exec-requires-{}",
            chrono::Utc::now().timestamp_millis()
        );
        let Some(id) = insert_test_recommendation(&title) else {
            return;
        };
        std::env::remove_var("STEER_TEST_ASSUME_APPROVED");
        std::env::set_var("STEER_N8N_MOCK", "1");
        let res = execute_approved_recommendation(id, None).await;
        assert!(res.is_err());
    }

    #[tokio::test]
    #[serial]
    async fn test_approve_assumed_pipeline_with_mock() {
        let title = format!(
            "rec-approve-assumed-{}",
            chrono::Utc::now().timestamp_millis()
        );
        let Some(id) = insert_test_recommendation(&title) else {
            return;
        };
        std::env::set_var("STEER_TEST_ASSUME_APPROVED", "1");
        std::env::set_var("STEER_N8N_MOCK", "1");

        assert!(maybe_assume_approved_for_test(id).is_ok());
        let workflow_id = execute_approved_recommendation(id, None)
            .await
            .expect("approve-assumed mock path should return workflow_id");
        assert!(!workflow_id.trim().is_empty());

        let rec = db::get_recommendation(id).ok().flatten();
        assert!(rec.is_some());
        let rec = rec.unwrap();
        assert_eq!(rec.status, "approved");
        assert!(rec.workflow_id.unwrap_or_default().starts_with("mock-wf-"));
    }

    #[tokio::test]
    #[serial]
    async fn test_execute_is_idempotent_for_existing_workflow() {
        let title = format!("rec-idempotent-{}", chrono::Utc::now().timestamp_millis());
        let Some(id) = insert_test_recommendation(&title) else {
            return;
        };
        std::env::set_var("STEER_TEST_ASSUME_APPROVED", "1");
        std::env::set_var("STEER_N8N_MOCK", "1");
        std::env::remove_var("STEER_APPROVE_FORCE_RECREATE");

        assert!(maybe_assume_approved_for_test(id).is_ok());
        let first = execute_approved_recommendation(id, None)
            .await
            .expect("first execution should provision workflow");
        assert!(!first.is_empty());

        let second = execute_approved_recommendation(id, None)
            .await
            .expect("second execution should reuse existing workflow");
        assert_eq!(first, second);
    }

    #[tokio::test]
    #[serial]
    async fn test_stale_provisioning_claim_is_recovered() {
        let title = format!("rec-stale-claim-{}", chrono::Utc::now().timestamp_millis());
        let Some(id) = insert_test_recommendation(&title) else {
            return;
        };
        std::env::set_var("STEER_TEST_ASSUME_APPROVED", "1");
        std::env::set_var("STEER_N8N_MOCK", "1");
        std::env::set_var("STEER_PROVISIONING_CLAIM_TTL_SECONDS", "1");
        std::env::remove_var("STEER_APPROVE_FORCE_RECREATE");
        assert!(maybe_assume_approved_for_test(id).is_ok());

        let stale_token = format!("provisioning:{}:1", id);
        let _ = db::claim_recommendation_provisioning(id, &stale_token);
        let workflow_id = execute_approved_recommendation(id, None)
            .await
            .expect("stale provisioning claim should be recovered");
        assert!(!workflow_id.trim().is_empty());
        assert!(!workflow_id.starts_with("provisioning:"));
    }

    #[tokio::test]
    #[serial]
    async fn test_approve_and_execute_reports_status() {
        let title = format!("rec-approve-exec-{}", chrono::Utc::now().timestamp_millis());
        let Some(id) = insert_test_recommendation(&title) else {
            return;
        };
        std::env::set_var("STEER_TEST_ASSUME_APPROVED", "1");
        std::env::set_var("STEER_N8N_MOCK", "1");
        std::env::remove_var("STEER_APPROVE_FORCE_RECREATE");

        let first = approve_and_execute_recommendation(id, None)
            .await
            .expect("first approve-and-execute should provision workflow");
        assert!(!first.workflow_id.trim().is_empty());
        assert!(first.approved_now);

        let second = approve_and_execute_recommendation(id, None)
            .await
            .expect("second approve-and-execute should reuse workflow");
        assert_eq!(first.workflow_id, second.workflow_id);
        assert!(!second.approved_now);
        assert!(second.reused_existing);
    }

    #[test]
    fn execution_status_classifier_matches_expected_values() {
        assert!(is_execution_success_status("success"));
        assert!(is_execution_success_status("completed"));
        assert!(!is_execution_success_status("running"));

        assert!(is_execution_failure_status("failed"));
        assert!(is_execution_failure_status("error"));
        assert!(is_execution_failure_status("cancelled"));
        assert!(!is_execution_failure_status("running"));
    }
}
