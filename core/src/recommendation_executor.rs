use crate::{db, env_flag, llm_gateway::LLMClient, n8n_api};
use anyhow::{anyhow, Result};
use std::sync::Arc;

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
    // Safe default: create inactive unless explicitly requested.
    parse_bool_env("STEER_N8N_ACTIVE_ON_CREATE", false)
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

    let workflow_json_str_result: Result<String> = (|| async {
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
        Ok(generated)
    })()
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
            "⚠️ workflow for recommendation {} had empty/missing nodes. Replacing with orchestrator fallback.",
            id
        );
        workflow_val = n8n_api::build_orchestrator_fallback_workflow(
            &rec.title,
            Some(&rec.n8n_prompt),
            "invalid_or_empty_nodes",
        );
    }
    let workflow_json_str = serde_json::to_string(&workflow_val).map_err(|e| {
        anyhow!(
            "workflow serialization failed for recommendation {}: {}",
            id,
            e
        )
    })?;

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

    match n8n.create_workflow(&rec.title, &workflow_val, active).await {
        Ok(workflow_id) => {
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
            if let Err(e) = db::commit_workflow_provision_success(
                provision_op_id,
                id,
                &workflow_id,
                Some(&workflow_json_str),
            ) {
                let _ = db::mark_workflow_provision_reconcile_needed(
                    provision_op_id,
                    &format!("workflow created but recommendation commit failed: {}", e),
                );
                let _ = db::mark_recommendation_failed(
                    id,
                    &format!("workflow created but commit failed: {}", e),
                );
                return Err(anyhow!(
                    "workflow created (id={}) but local commit failed: {}",
                    workflow_id,
                    e
                ));
            }
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
}
