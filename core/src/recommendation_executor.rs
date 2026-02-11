use crate::{db, env_flag, llm_gateway::LLMClient, n8n_api};
use anyhow::{anyhow, Result};
use serde_json::json;
use std::sync::Arc;

fn mock_workflow_json(name: &str) -> serde_json::Value {
    json!({
        "name": name,
        "nodes": [{
            "id": "manual-trigger-1",
            "name": "Manual Trigger",
            "type": "n8n-nodes-base.manualTrigger",
            "typeVersion": 1,
            "position": [240, 300],
            "parameters": {}
        }],
        "connections": {},
        "settings": {},
        "meta": {
            "source": "steer-approve-assumed-test"
        }
    })
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

pub fn maybe_assume_approved_for_test(id: i64) -> Result<()> {
    if !env_flag("STEER_TEST_ASSUME_APPROVED") {
        return Err(anyhow!(
            "STEER_TEST_ASSUME_APPROVED=1 is required for approve_test path"
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
    let force_recreate = env_flag("STEER_APPROVE_FORCE_RECREATE");
    if !force_recreate {
        if let Some(existing_id) = rec
            .workflow_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            println!(
                "ℹ️ Recommendation {} already provisioned. Reusing workflow_id={}",
                id, existing_id
            );
            return Ok(existing_id.to_string());
        }
    }

    let workflow_json_str = if should_use_test_mock_workflow() {
        serde_json::to_string(&mock_workflow_json(&rec.title))?
    } else if let Some(existing_json) = rec
        .workflow_json
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        existing_json.to_string()
    } else {
        let brain = llm_client.ok_or_else(|| anyhow!("LLM Client not available"))?;
        brain
            .build_n8n_workflow(&rec.n8n_prompt)
            .await
            .map_err(|e| anyhow!("workflow generation failed: {}", e))?
    };

    let workflow_val = serde_json::from_str::<serde_json::Value>(&workflow_json_str).map_err(|e| {
        anyhow!(
            "generated workflow JSON is invalid for recommendation {}: {}",
            id,
            e
        )
    })?;

    let n8n = n8n_api::N8nApi::from_env()?;
    let active = n8n_create_active_default();

    match n8n.create_workflow(&rec.title, &workflow_val, active).await {
        Ok(workflow_id) => {
            db::mark_recommendation_approved(id, &workflow_id, &workflow_json_str)?;
            Ok(workflow_id)
        }
        Err(e) => {
            let _ = db::mark_recommendation_failed(id, &e.to_string());
            Err(anyhow!("workflow creation failed: {}", e))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recommendation::AutomationProposal;

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
    fn test_assume_approved_requires_flag() {
        std::env::remove_var("STEER_TEST_ASSUME_APPROVED");
        let result = maybe_assume_approved_for_test(1);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_execute_requires_approved_status() {
        let title = format!("rec-exec-requires-{}", chrono::Utc::now().timestamp_millis());
        let Some(id) = insert_test_recommendation(&title) else {
            return;
        };
        std::env::remove_var("STEER_TEST_ASSUME_APPROVED");
        std::env::set_var("STEER_N8N_MOCK", "1");
        let res = execute_approved_recommendation(id, None).await;
        assert!(res.is_err());
    }

    #[tokio::test]
    async fn test_approve_assumed_pipeline_with_mock() {
        let title = format!("rec-approve-assumed-{}", chrono::Utc::now().timestamp_millis());
        let Some(id) = insert_test_recommendation(&title) else {
            return;
        };
        std::env::set_var("STEER_TEST_ASSUME_APPROVED", "1");
        std::env::set_var("STEER_N8N_MOCK", "1");

        assert!(maybe_assume_approved_for_test(id).is_ok());
        let workflow_id = execute_approved_recommendation(id, None).await.unwrap_or_default();
        assert!(!workflow_id.trim().is_empty());

        let rec = db::get_recommendation(id).ok().flatten();
        assert!(rec.is_some());
        let rec = rec.unwrap();
        assert_eq!(rec.status, "approved");
        assert!(rec.workflow_id.unwrap_or_default().starts_with("mock-wf-"));
    }
}
