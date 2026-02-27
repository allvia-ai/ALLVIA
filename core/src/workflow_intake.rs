use crate::{collector_pipeline, db, recommendation::AutomationProposal};
use anyhow::{anyhow, Result};
use serde_json::Value;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct ManualWorkflowQueueOutcome {
    pub recommendation_id: i64,
    pub inserted: bool,
}

#[derive(Debug, Clone)]
pub struct CollectorHandoffIngestOutcome {
    pub status: String,
    pub detail: String,
    pub package_id: Option<String>,
    pub recommendation_id: Option<i64>,
    pub inserted: bool,
}

fn summarize_prompt(prompt: &str, max_chars: usize) -> String {
    let trimmed = prompt.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }
    let short = trimmed.chars().take(max_chars).collect::<String>();
    format!("{}...", short)
}

fn recommendation_fingerprint(title: &str, trigger: &str) -> String {
    format!(
        "{}::{}",
        title.trim().to_lowercase(),
        trigger.trim().to_lowercase()
    )
}

fn find_recommendation_id_by_fingerprint(target: &str) -> Result<Option<i64>> {
    let rows = db::get_recommendations_with_filter(Some("all"))?;
    for rec in rows {
        let fp = recommendation_fingerprint(&rec.title, &rec.trigger);
        if fp == target {
            return Ok(Some(rec.id));
        }
    }
    Ok(None)
}

pub fn insert_or_get_recommendation_id(proposal: &AutomationProposal) -> Result<(i64, bool)> {
    let fp = proposal.fingerprint();
    let inserted = db::insert_recommendation(proposal)?;
    let rec_id = find_recommendation_id_by_fingerprint(&fp)?
        .ok_or_else(|| anyhow!("failed to resolve recommendation id after insert"))?;
    Ok((rec_id, inserted))
}

pub fn queue_manual_workflow_recommendation(
    prompt: &str,
    source: &str,
) -> Result<ManualWorkflowQueueOutcome> {
    let prompt_trimmed = prompt.trim();
    if prompt_trimmed.is_empty() {
        return Err(anyhow!("workflow prompt is empty"));
    }
    let short = summarize_prompt(prompt_trimmed, 48);
    let proposal = AutomationProposal {
        title: format!("Manual Workflow: {}", short),
        summary: format!(
            "Manual workflow request captured from {} (approval required before creation).",
            source
        ),
        trigger: "Manual workflow request".to_string(),
        actions: vec!["n8n Workflow".to_string()],
        confidence: 0.6,
        n8n_prompt: prompt_trimmed.to_string(),
        evidence: vec![
            format!("source={}", source),
            format!("prompt={}", summarize_prompt(prompt_trimmed, 160)),
        ],
        pattern_id: None,
    };

    let (recommendation_id, inserted) = insert_or_get_recommendation_id(&proposal)?;
    Ok(ManualWorkflowQueueOutcome {
        recommendation_id,
        inserted,
    })
}

fn config_path(config_override: Option<&str>) -> PathBuf {
    if let Some(path) = config_override.map(str::trim).filter(|s| !s.is_empty()) {
        return PathBuf::from(path);
    }
    std::env::var("STEER_COLLECTOR_CONFIG")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("configs/config.yaml"))
}

fn normalize_abs_path(path: &Path) -> String {
    if let Ok(canon) = std::fs::canonicalize(path) {
        return canon.to_string_lossy().to_string();
    }
    if path.is_absolute() {
        return path.to_string_lossy().to_string();
    }
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(path)
        .to_string_lossy()
        .to_string()
}

fn allow_collector_db_mismatch() -> bool {
    std::env::var("STEER_ALLOW_COLLECTOR_DB_MISMATCH")
        .ok()
        .map(|v| {
            matches!(
                v.trim().to_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

fn handoff_max_attempts() -> i64 {
    std::env::var("STEER_COLLECTOR_HANDOFF_MAX_ATTEMPTS")
        .ok()
        .and_then(|v| v.trim().parse::<i64>().ok())
        .filter(|v| *v >= 1)
        .unwrap_or(5)
}

fn handoff_retry_base_secs() -> u64 {
    std::env::var("STEER_COLLECTOR_HANDOFF_RETRY_BASE_SECS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|v| *v >= 1)
        .unwrap_or(60)
}

fn handoff_lease_secs() -> i64 {
    std::env::var("STEER_COLLECTOR_HANDOFF_LEASE_SECS")
        .ok()
        .and_then(|v| v.trim().parse::<i64>().ok())
        .filter(|v| *v >= 10)
        .unwrap_or(180)
}

fn expected_handoff_schema_major() -> u64 {
    std::env::var("STEER_COLLECTOR_HANDOFF_SCHEMA_MAJOR")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|v| *v >= 1)
        .unwrap_or(1)
}

fn parse_handoff_major(payload: &Value) -> Option<u64> {
    let raw = payload.get("version")?.as_str()?.trim();
    if raw.is_empty() {
        return None;
    }
    let normalized = raw.trim_start_matches(['v', 'V']);
    let major = normalized.split('.').next()?.trim();
    major.parse::<u64>().ok()
}

fn validate_handoff_schema(payload: &Value) -> Result<()> {
    let expected_major = expected_handoff_schema_major();
    let parsed_major = parse_handoff_major(payload).ok_or_else(|| {
        anyhow!(
            "collector handoff missing/invalid version (expected major={})",
            expected_major
        )
    })?;
    if parsed_major != expected_major {
        return Err(anyhow!(
            "collector handoff schema mismatch: got major={} expected major={}",
            parsed_major,
            expected_major
        ));
    }
    Ok(())
}

fn extract_sequence(candidate: &Value) -> String {
    let Some(events) = candidate
        .get("pattern")
        .and_then(|v| v.get("events"))
        .and_then(|v| v.as_array())
    else {
        return String::new();
    };
    let parts = events
        .iter()
        .filter_map(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.trim().to_string())
        .take(5)
        .collect::<Vec<_>>();
    parts.join(" -> ")
}

fn build_proposal_from_handoff(
    row: &collector_pipeline::PendingHandoffRow,
) -> Result<AutomationProposal> {
    let Some(top) = row
        .payload
        .get("routine_candidates")
        .and_then(|v| v.as_array())
        .and_then(|v| v.first())
    else {
        return Err(anyhow!("no routine_candidates in handoff payload"));
    };

    let pattern_id = top
        .get("pattern_id")
        .and_then(|v| v.as_str())
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());
    let support = top.get("support").and_then(|v| v.as_i64()).unwrap_or(0);
    let confidence = top
        .get("confidence")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.6)
        .clamp(0.1, 0.99);
    let sequence = extract_sequence(top);
    let active_app = row
        .payload
        .get("device_context")
        .and_then(|v| v.get("active_app"))
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");

    let sequence_label = if sequence.is_empty() {
        "Routine candidate".to_string()
    } else {
        sequence.clone()
    };

    let title = format!(
        "Collector Routine: {}",
        summarize_prompt(&sequence_label, 64)
    );
    let trigger = pattern_id
        .clone()
        .map(|p| format!("Collector pattern {}", p))
        .unwrap_or_else(|| format!("Collector package {}", row.package_id));

    let summary = format!(
        "Collector handoff 기반 루틴 후보입니다. support={}, confidence={:.2}, active_app={}",
        support, confidence, active_app
    );
    let n8n_prompt = if sequence.is_empty() {
        format!(
            "Create an n8n workflow for repeated activity detected from collector package {}. Include a Telegram summary and optional Notion logging.",
            row.package_id
        )
    } else {
        format!(
            "Create an n8n workflow for this repeated sequence: {}. support={}, confidence={:.2}, active_app={}. Include Telegram summary and human approval checkpoint before side effects.",
            sequence, support, confidence, active_app
        )
    };

    Ok(AutomationProposal {
        title,
        summary,
        trigger,
        actions: vec!["collector_handoff".to_string(), "n8n Workflow".to_string()],
        confidence,
        n8n_prompt,
        evidence: vec![
            format!("package_id={}", row.package_id),
            format!("handoff_created_at={}", row.created_at),
            format!("support={}", support),
            format!("active_app={}", active_app),
            format!("sequence={}", summarize_prompt(&sequence_label, 160)),
        ],
        pattern_id,
    })
}

pub fn ingest_latest_collector_handoff(
    config_override: Option<&str>,
) -> Result<CollectorHandoffIngestOutcome> {
    let cfg_path = config_path(config_override);
    let collector_db = collector_pipeline::resolve_db_path(Some(&cfg_path));
    if !allow_collector_db_mismatch() {
        if let Some(core_db_path) = db::current_db_path() {
            let collector_norm = normalize_abs_path(&collector_db);
            let core_norm = normalize_abs_path(Path::new(&core_db_path));
            if collector_norm != core_norm {
                return Err(anyhow!(
                    "collector/core db path mismatch (collector={}, core={}). \
Set STEER_ALLOW_COLLECTOR_DB_MISMATCH=1 only when intentional.",
                    collector_norm,
                    core_norm
                ));
            }
        }
    }
    let mut conn = collector_pipeline::open_connection(&collector_db)?;
    collector_pipeline::ensure_pipeline_tables(&conn)?;
    let max_attempts = handoff_max_attempts();
    let retry_base_secs = handoff_retry_base_secs();
    let lease_secs = handoff_lease_secs();
    let consumer_id = format!("intake-{}", std::process::id());

    let Some(row) = collector_pipeline::claim_retryable_handoff(
        &mut conn,
        max_attempts,
        &consumer_id,
        lease_secs,
    )?
    else {
        return Ok(CollectorHandoffIngestOutcome {
            status: "noop".to_string(),
            detail: "no retryable collector handoff".to_string(),
            package_id: None,
            recommendation_id: None,
            inserted: false,
        });
    };

    let package_id = row.package_id.clone();
    if let Err(schema_err) = validate_handoff_schema(&row.payload) {
        let detail = format!("handoff schema invalid: {}", schema_err);
        collector_pipeline::update_handoff_status(&conn, row.id, "invalid", Some(&detail))?;
        let _ = db::record_collector_handoff_receipt(
            &package_id,
            Some(row.id),
            "invalid",
            None,
            Some(&detail),
        );
        return Ok(CollectorHandoffIngestOutcome {
            status: "invalid".to_string(),
            detail,
            package_id: Some(package_id),
            recommendation_id: None,
            inserted: false,
        });
    }

    let ingest_result = (|| -> Result<(i64, bool)> {
        let proposal = build_proposal_from_handoff(&row)?;
        insert_or_get_recommendation_id(&proposal)
    })();

    match ingest_result {
        Ok((rec_id, inserted)) => {
            collector_pipeline::mark_handoff_consumed(&conn, row.id)?;
            let _ = db::record_collector_handoff_receipt(
                &package_id,
                Some(row.id),
                "consumed",
                Some(rec_id),
                Some(&format!(
                    "inserted={} attempts={}",
                    inserted,
                    row.attempt_count + 1
                )),
            );
            Ok(CollectorHandoffIngestOutcome {
                status: "consumed".to_string(),
                detail: format!(
                    "collector handoff consumed -> recommendation {} (inserted={}, attempts={})",
                    rec_id,
                    inserted,
                    row.attempt_count + 1
                ),
                package_id: Some(package_id),
                recommendation_id: Some(rec_id),
                inserted,
            })
        }
        Err(e) => {
            let update = collector_pipeline::mark_handoff_failed_with_backoff(
                &conn,
                row.id,
                &e.to_string(),
                max_attempts,
                retry_base_secs,
            )?;
            let status = if update.terminal {
                "failed".to_string()
            } else {
                "retry_scheduled".to_string()
            };
            let detail = if let Some(next_retry_at) = update.next_retry_at {
                format!(
                    "handoff ingest failed (attempt {}/{}): {} | next_retry_at={}",
                    update.attempt_count, update.max_attempts, e, next_retry_at
                )
            } else {
                format!(
                    "handoff ingest failed (attempt {}/{}): {} | no more retries",
                    update.attempt_count, update.max_attempts, e
                )
            };
            let _ = db::record_collector_handoff_receipt(
                &package_id,
                Some(row.id),
                &status,
                None,
                Some(&detail),
            );
            Ok(CollectorHandoffIngestOutcome {
                status,
                detail,
                package_id: Some(package_id),
                recommendation_id: None,
                inserted: false,
            })
        }
    }
}
