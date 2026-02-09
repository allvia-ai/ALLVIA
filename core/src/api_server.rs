use axum::{
    extract::{Path, Query, State},
    http::{HeaderValue, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tower_http::cors::{Any, CorsLayer};

use crate::{
    approval_gate, chat_sanitize, consistency_check, context_pruning, db, execution_controller,
    feedback_collector, integrations, intent_router, judgment, llm_gateway, monitor, n8n_api,
    nl_store, pattern_detector, performance_verification, plan_builder, project_scanner,
    quality_scorer, release_gate, runtime_verification, semantic_verification, slot_filler,
    tool_result_guard, verification_engine, visual_verification,
};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use sysinfo::System;

#[derive(Clone)]
pub struct AppState {
    pub llm_client: Option<std::sync::Arc<dyn llm_gateway::LLMClient>>,
    pub current_goal: Arc<Mutex<Option<String>>>,
}

// Request/Response types
#[derive(Deserialize)]
pub struct ChatRequest {
    pub message: String,
    pub channel: Option<String>,
    pub chat_type: Option<String>,
    pub sender: Option<String>,
    pub mentioned: Option<bool>,
}

#[derive(Deserialize)]
pub struct FeedbackRequest {
    pub goal: String,
    pub feedback: String,
    pub history_summary: Option<String>,
}

#[derive(Serialize)]
pub struct FeedbackResponse {
    pub action: String,
    pub new_goal: Option<String>,
    pub message: String,
}

#[derive(Deserialize)]
pub struct AgentIntentRequest {
    pub text: String,
}

#[derive(Serialize)]
pub struct AgentIntentResponse {
    pub session_id: String,
    pub intent: String,
    pub confidence: f32,
    pub slots: HashMap<String, String>,
    pub missing_slots: Vec<String>,
    pub follow_up: Option<String>,
}

#[derive(Deserialize)]
pub struct AgentPlanRequest {
    pub session_id: String,
    pub slots: Option<HashMap<String, String>>,
}

#[derive(Serialize)]
pub struct AgentPlanResponse {
    pub plan_id: String,
    pub intent: String,
    pub steps: Vec<crate::nl_automation::PlanStep>,
    pub missing_slots: Vec<String>,
}

#[derive(Deserialize)]
pub struct AgentExecuteRequest {
    pub plan_id: String,
}

#[derive(Serialize)]
pub struct AgentExecuteResponse {
    pub status: String,
    pub logs: Vec<String>,
    pub approval: Option<crate::nl_automation::ApprovalContext>,
    #[serde(default)]
    pub manual_steps: Vec<String>,
    pub resume_from: Option<usize>,
}

#[derive(Deserialize)]
pub struct AgentVerifyRequest {
    pub plan_id: String,
}

#[derive(Serialize)]
pub struct AgentVerifyResponse {
    pub ok: bool,
    pub issues: Vec<String>,
}

#[derive(Deserialize)]
pub struct AgentApproveRequest {
    pub plan_id: String,
    pub action: String,
    pub decision: Option<String>,
}

#[derive(Serialize)]
pub struct AgentApproveResponse {
    pub status: String,
    pub requires_approval: bool,
    pub message: String,
    pub risk_level: String,
    pub policy: String,
}

#[derive(Deserialize)]
pub struct ExecApprovalQuery {
    pub status: Option<String>,
    pub limit: Option<i64>,
}

#[derive(Deserialize)]
pub struct ApprovalPolicyQuery {
    pub limit: Option<i64>,
}

#[derive(Deserialize)]
pub struct NLRunMetricsQuery {
    pub limit: Option<i64>,
}

#[derive(Deserialize)]
pub struct ApprovalPolicyRequest {
    pub policy_key: String,
    pub decision: String,
}

#[derive(Serialize)]
pub struct ApprovalPolicyResponse {
    pub policy_key: String,
    pub decision: String,
    pub updated_at: String,
}

#[derive(Deserialize, Default)]
pub struct ExecApprovalResolve {
    pub resolved_by: Option<String>,
    pub decision: Option<String>,
}

#[derive(Deserialize)]
pub struct RoutineRunsQuery {
    pub limit: Option<i64>,
}

#[derive(Deserialize)]
pub struct ExecAllowlistRequest {
    pub pattern: String,
    pub cwd: Option<String>,
}

#[derive(Deserialize)]
pub struct ExecAllowlistQuery {
    pub limit: Option<i64>,
}

#[derive(Deserialize)]
pub struct ExecResultsQuery {
    pub status: Option<String>,
    pub limit: Option<i64>,
}

#[derive(Deserialize)]
pub struct VerificationRunsQuery {
    pub limit: Option<i64>,
}

#[derive(Deserialize)]
pub struct NLRunQuery {
    pub limit: Option<i64>,
}

#[derive(Deserialize)]
pub struct ProjectScanQuery {
    pub max_files: Option<usize>,
    pub workdir: Option<String>,
}

#[derive(Serialize)]
pub struct ProjectScanResponse {
    pub project_type: String,
    pub files: Vec<String>,
    pub key_files: std::collections::HashMap<String, String>,
}

#[derive(Deserialize)]
pub struct RuntimeVerifyRequest {
    pub workdir: Option<String>,
    pub run_backend: Option<bool>,
    pub run_frontend: Option<bool>,
    pub run_e2e: Option<bool>,
    pub run_build_checks: Option<bool>,
    pub backend_port: Option<u16>,
    pub frontend_port: Option<u16>,
    pub backend_health_path: Option<String>,
}

#[derive(Deserialize)]
pub struct QualityScoreRequest {
    pub runtime: Option<runtime_verification::RuntimeVerifyResult>,
    pub runtime_options: Option<RuntimeVerifyRequest>,
    pub code_review: Option<quality_scorer::CodeReviewInput>,
    pub goal: Option<String>,
    pub use_llm: Option<bool>,
}

#[derive(Serialize)]
pub struct QualityScoreResponse {
    pub created_at: String,
    pub score: quality_scorer::QualityScore,
}

#[derive(Deserialize)]
pub struct SemanticVerifyRequest {
    pub workdir: Option<String>,
    pub max_files: Option<usize>,
}

#[derive(Deserialize)]
pub struct PerformanceVerifyRequest {
    pub workdir: Option<String>,
    pub max_files: Option<usize>,
}

#[derive(Serialize)]
pub struct ChatResponse {
    pub response: String,
    pub command: Option<String>,
}

#[derive(Serialize)]
pub struct SystemStatus {
    pub cpu_usage: f32,
    pub memory_used: u64,
    pub memory_total: u64,
}

#[derive(Serialize)]
pub struct LogEntry {
    pub timestamp: String,
    pub level: String,
    pub message: String,
}

#[derive(Serialize)]
pub struct RecommendationItem {
    pub id: i64,
    pub status: String, // [NEW] Status field
    pub title: String,
    pub summary: String,
    pub confidence: f64,
    pub evidence: Vec<String>, // [NEW] Explainability field
    pub last_error: Option<String>,
}

#[derive(Serialize)]
pub struct QualityMetrics {
    pub total: u32,
    pub success: u32,
    pub rate: f64,
}

/// Start the HTTP API server for desktop GUI
// Middleware for API Key Authentication
async fn auth_middleware(
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Result<axum::response::Response, StatusCode> {
    let api_key = std::env::var("STEER_API_KEY").unwrap_or_default();

    // If no key configured, allow all (Localhost Dev Mode)
    if api_key.is_empty() {
        return Ok(next.run(req).await);
    }

    // Check Header
    let auth_header = req
        .headers()
        .get("Authorization")
        .and_then(|h| h.to_str().ok())
        .map(|s| s.replace("Bearer ", ""));

    match auth_header {
        Some(key) if key == api_key => Ok(next.run(req).await),
        _ => Err(StatusCode::UNAUTHORIZED),
    }
}

/// Start the HTTP API server for desktop GUI
pub async fn start_api_server(
    llm_client: Option<std::sync::Arc<dyn llm_gateway::LLMClient>>,
) -> anyhow::Result<()> {
    let state = AppState {
        llm_client,
        current_goal: Arc::new(Mutex::new(None)),
    };

    // SECURITY: Restrict CORS to localhost only (Tauri/Dev Server)
    let allowed_origins = [
        "http://localhost:5173"
            .parse::<HeaderValue>()
            .expect("Invalid CORS origin"),
        "http://localhost:5174"
            .parse::<HeaderValue>()
            .expect("Invalid CORS origin"),
        "http://localhost:5680"
            .parse::<HeaderValue>()
            .expect("Invalid CORS origin"),
        "tauri://localhost"
            .parse::<HeaderValue>()
            .expect("Invalid CORS origin"),
        "http://127.0.0.1:5173"
            .parse::<HeaderValue>()
            .expect("Invalid CORS origin"),
        "http://127.0.0.1:5174"
            .parse::<HeaderValue>()
            .expect("Invalid CORS origin"),
    ];

    let cors = CorsLayer::new()
        .allow_origin(allowed_origins)
        .allow_methods(Any)
        .allow_headers(Any);

    let app = Router::new()
        .route("/", get(root_handler))
        .route("/api/health", get(health_check))
        // Open endpoints (Status, Logs)
        .route("/api/status", get(get_system_status))
        .route("/api/logs", get(get_recent_logs))
        .route("/api/system/health", get(get_system_health))
        // Protected Endpoints (Chat, Execute, Plan, Verify)
        .route("/events", post(ingest_events))
        .route("/api/chat", post(handle_chat))
        .route("/api/recommendations", get(list_recommendations))
        .route(
            "/api/recommendations/:id/approve",
            post(approve_recommendation),
        )
        .route(
            "/api/recommendations/:id/reject",
            post(reject_recommendation),
        )
        .route("/api/recommendations/:id/later", post(later_recommendation))
        .route(
            "/api/recommendations/:id/restore",
            post(restore_recommendation),
        )
        .route("/api/exec-approvals", get(list_exec_approvals))
        .route(
            "/api/exec-approvals/:id/approve",
            post(approve_exec_approval),
        )
        .route("/api/exec-approvals/:id/reject", post(reject_exec_approval))
        .route(
            "/api/exec-allowlist",
            get(list_exec_allowlist).post(add_exec_allowlist),
        )
        .route(
            "/api/exec-allowlist/:id",
            axum::routing::delete(remove_exec_allowlist),
        )
        .route("/api/exec-results", get(list_exec_results))
        .route("/api/project/scan", get(scan_project_handler))
        .route(
            "/api/verify/runtime",
            post(run_runtime_verification_handler),
        )
        .route("/api/verify/visual", post(run_visual_verification_handler))
        .route(
            "/api/verify/semantic",
            post(run_semantic_verification_handler),
        )
        .route(
            "/api/verify/performance",
            post(run_performance_verification_handler),
        )
        .route(
            "/api/verify/consistency",
            post(run_consistency_verification_handler),
        )
        .route("/api/verify/runs", get(list_verification_runs))
        .route("/api/judgment", post(run_judgment_handler))
        .route("/api/release/baseline", post(set_release_baseline_handler))
        .route("/api/release/gate", post(run_release_gate_handler))
        .route(
            "/api/exec-results/guard",
            post(run_exec_results_guard_handler),
        )
        .route("/api/quality/score", post(score_quality_handler))
        .route("/api/quality/latest", get(latest_quality_handler))
        .route("/api/patterns/analyze", post(analyze_patterns))
        .route("/api/quality", get(get_quality_metrics))
        .route(
            "/api/recommendations/metrics",
            get(get_recommendation_metrics),
        )
        .route(
            "/api/routines",
            get(list_routines).post(create_routine_handler),
        )
        .route(
            "/api/routines/:id",
            axum::routing::patch(toggle_routine_handler),
        )
        .route("/api/routine-runs", get(list_routine_runs))
        .route("/api/agent/intent", post(agent_intent_handler))
        .route("/api/agent/plan", post(agent_plan_handler))
        .route("/api/agent/execute", post(agent_execute_handler))
        .route("/api/agent/verify", post(agent_verify_handler))
        .route("/api/agent/approve", post(agent_approve_handler))
        .route("/api/agent/nl-runs", get(list_nl_runs_handler))
        .route("/api/agent/nl-metrics", get(nl_run_metrics_handler))
        .route(
            "/api/agent/approval-policies",
            get(list_nl_approval_policies).post(set_nl_approval_policy),
        )
        .route(
            "/api/agent/approval-policies/:key",
            axum::routing::delete(remove_nl_approval_policy),
        )
        .route("/api/agent/goal", post(execute_goal_handler))
        .route("/api/agent/goal/current", get(get_current_goal))
        .route("/api/agent/feedback", post(handle_feedback))
        .route("/api/context/selection", get(get_selection_context))
        // Session Management (Clawdbot-ported)
        .route("/api/sessions", get(list_sessions_handler))
        .route(
            "/api/sessions/:id",
            get(get_session_handler).delete(delete_session_handler),
        )
        .route("/api/sessions/:id/resume", post(resume_session_handler))
        .layer(axum::middleware::from_fn(auth_middleware)) // Apply Auth Middleware
        .layer(cors)
        .with_state(state);

    let port = std::env::var("STEER_API_PORT")
        .ok()
        .and_then(|v| v.parse::<u16>().ok())
        .unwrap_or(5680);
    println!("🌐 Desktop API server running on http://localhost:{}", port);

    let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{}", port))
        .await
        .map_err(|e| anyhow::anyhow!("Failed to bind port {}: {}", port, e))?;

    axum::serve(listener, app)
        .await
        .map_err(|e| anyhow::anyhow!("Server error: {}", e))?;
    Ok(())
}

async fn root_handler() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "online",
        "service": "Steer OS Core API",
        "version": "v0.1.0",
        "ui_url": "http://localhost:5174",
        "docs": "/api/health"
    }))
}

async fn health_check() -> &'static str {
    "ok"
}

async fn get_system_status() -> Json<SystemStatus> {
    let mut sys = System::new_all();
    sys.refresh_cpu(); // First refresh just gathers data
    std::thread::sleep(std::time::Duration::from_millis(200)); // Sleep minimal amount for CPU calculation
    sys.refresh_cpu(); // Second refresh calculates usage
    sys.refresh_memory();

    let cpu_usage = sys.global_cpu_info().cpu_usage();
    let memory_used = sys.used_memory() as f32 / 1024.0 / 1024.0; // MB
    let memory_total = sys.total_memory() as f32 / 1024.0 / 1024.0; // MB

    Json(SystemStatus {
        cpu_usage,
        memory_used: memory_used as u64,
        memory_total: memory_total as u64,
    })
}

async fn get_recent_logs() -> Json<Vec<LogEntry>> {
    // Fetch routines from DB and convert last_run to LogEntry
    match crate::db::get_all_routines() {
        Ok(routines) => {
            let mut logs = Vec::new();
            for r in routines {
                if let Some(last) = r.last_run {
                    logs.push(LogEntry {
                        timestamp: last,
                        level: "INFO".to_string(),
                        message: format!("Routine Executed: {}", r.name),
                    });
                }
            }
            // Sort by timestamp desc
            logs.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
            Json(logs)
        }
        Err(_) => Json(vec![]),
    }
}

async fn get_system_health() -> Json<crate::dependency_check::SystemHealth> {
    let health = crate::dependency_check::SystemHealth::check_all();
    Json(health)
}

async fn scan_project_handler(Query(query): Query<ProjectScanQuery>) -> Json<ProjectScanResponse> {
    let workdir = query.workdir.unwrap_or_else(|| {
        std::env::current_dir()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string()
    });
    let scanner = project_scanner::ProjectScanner::new(&workdir);
    let result = scanner.scan(query.max_files);
    let project_type = scanner.get_project_type();

    Json(ProjectScanResponse {
        project_type: project_type.as_str().to_string(),
        files: result.files,
        key_files: result.key_files,
    })
}

async fn run_runtime_verification_handler(
    Json(payload): Json<RuntimeVerifyRequest>,
) -> Json<runtime_verification::RuntimeVerifyResult> {
    let options = runtime_verification::RuntimeVerifyOptions {
        workdir: payload.workdir,
        run_backend: payload.run_backend,
        run_frontend: payload.run_frontend,
        run_e2e: payload.run_e2e,
        run_build_checks: payload.run_build_checks,
        backend_port: payload.backend_port,
        frontend_port: payload.frontend_port,
        backend_health_path: payload.backend_health_path,
    };
    let result = runtime_verification::run_runtime_verification(options).await;
    let summary = if result.issues.is_empty() {
        "Runtime verification passed".to_string()
    } else {
        format!("Runtime verification issues: {}", result.issues.len())
    };
    log_verification_run(
        "runtime",
        result.issues.is_empty(),
        &summary,
        Some(
            json!({ "issues": result.issues, "backend_health": result.backend_health, "frontend_health": result.frontend_health }),
        ),
    );
    Json(result)
}

async fn run_visual_verification_handler(
    State(state): State<AppState>,
    Json(payload): Json<visual_verification::VisualVerifyRequest>,
) -> Json<visual_verification::VisualVerifyResult> {
    let Some(llm) = &state.llm_client else {
        return Json(visual_verification::VisualVerifyResult {
            ok: false,
            verdicts: vec![],
        });
    };
    match visual_verification::verify_screen(llm.as_ref(), payload).await {
        Ok(result) => {
            let summary = if result.ok {
                "Visual verification passed"
            } else {
                "Visual verification failed"
            };
            let details = json!({
                "verdicts": result.verdicts.iter().map(|v| json!({ "prompt": v.prompt, "ok": v.ok })).collect::<Vec<_>>()
            });
            log_verification_run("visual", result.ok, summary, Some(details));
            Json(result)
        }
        Err(_) => Json(visual_verification::VisualVerifyResult {
            ok: false,
            verdicts: vec![],
        }),
    }
}

async fn run_semantic_verification_handler(
    Json(payload): Json<SemanticVerifyRequest>,
) -> Json<semantic_verification::SemanticVerificationResult> {
    let workdir = payload.workdir.unwrap_or_else(|| {
        std::env::current_dir()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string()
    });
    let max_files = payload.max_files.unwrap_or(200);
    let result =
        semantic_verification::semantic_consistency(std::path::Path::new(&workdir), max_files);
    let details = json!({
        "issues": result.issues.iter().take(10).map(|i| json!({"file": i.file, "severity": i.severity, "reason": i.reason})).collect::<Vec<_>>()
    });
    log_verification_run("semantic", result.ok, &result.reason, Some(details));
    Json(result)
}

async fn run_performance_verification_handler(
    Json(payload): Json<PerformanceVerifyRequest>,
) -> Json<performance_verification::PerformanceVerificationResult> {
    let workdir = payload.workdir.unwrap_or_else(|| {
        std::env::current_dir()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string()
    });
    let max_files = payload.max_files.unwrap_or(300);
    let result =
        performance_verification::performance_baseline(std::path::Path::new(&workdir), max_files);
    let details = json!({
        "metrics": result.metrics.iter().map(|m| json!({"name": m.name, "value": m.value, "threshold": m.threshold, "ok": m.ok})).collect::<Vec<_>>()
    });
    log_verification_run("performance", result.ok, &result.reason, Some(details));
    Json(result)
}

async fn run_consistency_verification_handler(
    Json(payload): Json<consistency_check::ConsistencyCheckRequest>,
) -> Json<consistency_check::ConsistencyCheckResult> {
    let result = consistency_check::run_consistency_check(payload);
    let details = json!({
        "summary": result.summary,
        "issues": result.issues.iter().take(10).map(|i| json!({"path": i.path, "source": i.source})).collect::<Vec<_>>()
    });
    log_verification_run("consistency", result.ok, &result.summary, Some(details));
    Json(result)
}

async fn run_judgment_handler(
    Json(payload): Json<judgment::JudgmentRequest>,
) -> Json<judgment::JudgmentResponse> {
    let result = judgment::evaluate_judgment(payload);
    Json(result)
}

async fn set_release_baseline_handler(
    Json(payload): Json<release_gate::ReleaseBaselineRequest>,
) -> Json<release_gate::ReleaseBaseline> {
    let baseline = release_gate::build_baseline(payload);
    release_gate::save_baseline(&baseline);
    Json(baseline)
}

async fn run_release_gate_handler(
    Json(payload): Json<release_gate::ReleaseGateRequest>,
) -> Json<release_gate::ReleaseGateResult> {
    let result = release_gate::run_release_gate(payload);
    let summary = if result.ok {
        "Release gate passed"
    } else {
        "Release gate failed"
    };
    let details = json!({
        "regressions": result.regressions.iter().take(10).cloned().collect::<Vec<_>>(),
        "warnings": result.warnings.iter().take(10).cloned().collect::<Vec<_>>()
    });
    log_verification_run("release_gate", result.ok, summary, Some(details));
    Json(result)
}

async fn run_exec_results_guard_handler(
    Json(payload): Json<tool_result_guard::ToolResultGuardRequest>,
) -> Json<tool_result_guard::ToolResultGuardResult> {
    let result = tool_result_guard::guard_exec_results(payload);
    Json(result)
}

async fn score_quality_handler(
    State(state): State<AppState>,
    Json(payload): Json<QualityScoreRequest>,
) -> Json<QualityScoreResponse> {
    let runtime = if let Some(rt) = payload.runtime {
        rt
    } else if let Some(opts) = payload.runtime_options {
        let options = runtime_verification::RuntimeVerifyOptions {
            workdir: opts.workdir,
            run_backend: opts.run_backend,
            run_frontend: opts.run_frontend,
            run_e2e: opts.run_e2e,
            run_build_checks: opts.run_build_checks,
            backend_port: opts.backend_port,
            frontend_port: opts.frontend_port,
            backend_health_path: opts.backend_health_path,
        };
        runtime_verification::run_runtime_verification(options).await
    } else {
        runtime_verification::RuntimeVerifyResult {
            backend_started: false,
            backend_health: false,
            backend_build_ok: None,
            frontend_started: false,
            frontend_health: false,
            frontend_build_ok: None,
            e2e_passed: None,
            issues: vec!["No runtime verification provided".to_string()],
            logs: Vec::new(),
        }
    };

    let use_llm = payload.use_llm.unwrap_or(false);
    let score = if use_llm {
        if let Some(llm) = &state.llm_client {
            match quality_scorer::score_quality_with_llm(
                llm.as_ref(),
                payload.goal.as_deref(),
                Some(&runtime),
                payload.code_review.as_ref(),
            )
            .await
            {
                Ok(score) => score,
                Err(_) => {
                    quality_scorer::score_quality(Some(&runtime), payload.code_review.as_ref())
                }
            }
        } else {
            quality_scorer::score_quality(Some(&runtime), payload.code_review.as_ref())
        }
    } else {
        quality_scorer::score_quality(Some(&runtime), payload.code_review.as_ref())
    };
    let _ = db::insert_quality_score(&score);
    let created_at = chrono::Utc::now().to_rfc3339();
    Json(QualityScoreResponse { created_at, score })
}

async fn latest_quality_handler() -> Json<Option<QualityScoreResponse>> {
    match db::get_latest_quality_score() {
        Ok(Some(record)) => {
            let score = quality_scorer::QualityScore {
                overall: record.overall,
                breakdown: record
                    .breakdown
                    .as_object()
                    .map(|map| {
                        map.iter()
                            .filter_map(|(k, v)| v.as_f64().map(|val| (k.clone(), val)))
                            .collect()
                    })
                    .unwrap_or_default(),
                issues: record.issues,
                strengths: record.strengths,
                recommendation: record.recommendation,
                summary: record.summary,
            };
            Json(Some(QualityScoreResponse {
                created_at: record.created_at,
                score,
            }))
        }
        _ => Json(None),
    }
}

// Ingest Handler (Replaces Python main.py)
async fn ingest_events(Json(payload): Json<serde_json::Value>) -> Json<serde_json::Value> {
    // 1. Normalize
    let events: Vec<crate::schema::EventEnvelope> = if let Some(arr) = payload.as_array() {
        arr.iter()
            .filter_map(|v| serde_json::from_value(v.clone()).ok())
            .collect()
    } else if let Ok(single) = serde_json::from_value(payload.clone()) {
        vec![single]
    } else {
        return Json(serde_json::json!({ "error": "Invalid Event Format", "count": 0 }));
    };

    let count = events.len();

    // 2. Process & Insert
    // In a real high-perf scenario, we would push to a channel (EventBus).
    // For now, direct DB insert is fast enough for direct migration.

    // Initialize Privacy Guard (Salt should come from env in prod)
    let salt = std::env::var("PRIVACY_SALT").unwrap_or_else(|_| "default_salt".to_string());
    let guard = crate::privacy::PrivacyGuard::new(salt);

    let mut success = 0;
    for event in events {
        // [Privacy] Apply masking
        if let Some(masked_event) = guard.apply(event) {
            if let Err(e) = db::insert_event_v2(&masked_event) {
                eprintln!("Ingest Error: {}", e);
            } else {
                success += 1;
            }
        } else {
            // Dropped by privacy rules (e.g. deny list)
            println!("Event dropped by PrivacyGuard");
        }
    }

    Json(serde_json::json!({
        "status": "queued",
        "received": count,
        "processed": success
    }))
}

async fn handle_chat(
    State(state): State<AppState>,
    Json(req): Json<ChatRequest>,
) -> Json<ChatResponse> {
    let gate = crate::chat_gate::ChatGateConfig::from_env();
    let gate_ctx = crate::chat_gate::ChatGateContext {
        channel: req.channel.clone(),
        chat_type: req.chat_type.clone(),
        sender: req.sender.clone(),
        mentioned: req.mentioned,
    };
    if !gate.is_allowed(&gate_ctx) {
        return Json(ChatResponse {
            response: "⛔️ 이 채널에서는 현재 요청을 처리할 수 없습니다.".to_string(),
            command: None,
        });
    }

    let sanitized = chat_sanitize::sanitize_chat_input(&req.message);
    if !sanitized.flags.is_empty() {
        eprintln!("⚠️ Chat sanitize flags: {:?}", sanitized.flags);
    }
    let message = sanitized.text.trim().to_string();
    if message.is_empty() {
        return Json(ChatResponse {
            response: "❓ 메시지가 비어있어요. 다시 입력해주세요.".to_string(),
            command: None,
        });
    }

    // [Memory] Save User Message
    if let Err(e) = db::insert_chat_message("user", &message) {
        eprintln!("Failed to save user chat: {}", e);
    }

    // 1. Intercept explicit system commands (Bypass LLM)
    if message == "analyze_patterns" || message == "패턴 분석" {
        let results = run_analysis_internal();
        let response_text = if results.is_empty() {
            "🔍 분석 완료! 새로운 패턴을 찾지 못했습니다.\n(하지만 시연을 위해 데모 항목을 생성했습니다. 오른쪽을 확인하세요!)".to_string()
        } else {
            format!(
                "🔍 분석 완료! {}개의 패턴을 찾았습니다:\n{}",
                results.len(),
                results.join("\n")
            )
        };

        return Json(ChatResponse {
            response: response_text,
            command: Some("analyze_patterns".to_string()),
        });
    }

    // Issue #4 Fix: n8n restart command
    if message == "n8n restart" || message.contains("n8n 재시작") {
        match std::process::Command::new("pkill")
            .arg("-f")
            .arg("n8n")
            .output()
        {
            Ok(_) => {
                // Try to start n8n again
                let _ = std::process::Command::new("npx")
                    .args(["n8n", "start"])
                    .spawn();
                return Json(ChatResponse {
                    response: "🔄 n8n 서버를 재시작했습니다.".to_string(),
                    command: Some("n8n_restart".to_string()),
                });
            }
            Err(e) => {
                return Json(ChatResponse {
                    response: format!("❌ n8n 재시작 실패: {}", e),
                    command: None,
                });
            }
        }
    }

    // 2. Vision Command (Explicit Only)
    // Prevent accidental capture on "screen" mention. Require explicit command.
    if message.trim() == "/capture"
        || message.contains("화면 분석해줘")
        || message.contains("analyze screen")
    {
        if let Some(llm) = &state.llm_client {
            match crate::visual_driver::VisualDriver::capture_screen() {
                Ok((b64, _scale)) => {
                    let prompt = "Describe what is on the user's screen briefly. Identify active applications and context.";
                    match llm.analyze_screen(prompt, &b64).await {
                        Ok(desc) => {
                            return Json(ChatResponse {
                                response: format!("👁️ 화면 분석 결과:\n{}", desc),
                                command: None,
                            })
                        }
                        Err(e) => {
                            return Json(ChatResponse {
                                response: format!("❌ Vision API 오류: {}", e),
                                command: None,
                            })
                        }
                    }
                }
                Err(e) => {
                    return Json(ChatResponse {
                        response: format!("❌ 화면 캡처 실패: {}", e),
                        command: None,
                    })
                }
            }
        } else {
            return Json(ChatResponse {
                response: "❌ LLM 클라이언트가 초기화되지 않았습니다.".to_string(),
                command: None,
            });
        }
    }

    // 3. Demo Vision Workflow Trigger
    if message == "demo_vision" {
        if let Some(llm) = &state.llm_client {
            let llm_clone = llm.clone(); // Clone for async task

            tokio::spawn(async move {
                let mut driver = crate::visual_driver::VisualDriver::new();
                use crate::visual_driver::{SmartStep, UiAction};

                // Step 1: Open Google
                driver.add_step(
                    SmartStep::new(
                        UiAction::OpenUrl("https://www.google.com".to_string()),
                        "Open Google",
                    )
                    .with_post_check("Is the Google search homepage visible?"),
                );

                // Step 2: Wait for load
                driver.add_step(SmartStep::new(UiAction::Wait(3), "Wait for Load"));

                // Step 3: Type 'Hello World'
                // Pre-check: Ensure search bar exists
                // Post-check: Ensure text appears
                driver.add_step(
                    SmartStep::new(
                        UiAction::Type("Hello World".to_string()),
                        "Type Search Query",
                    )
                    .with_pre_check("Is there a search input field visible?")
                    .with_post_check("Is the text 'Hello World' visible in the search bar?"),
                );

                if let Err(e) = driver.execute(Some(llm_clone.as_ref())).await {
                    eprintln!("❌ Vision Demo Failed: {}", e);
                } else {
                    println!("✅ Vision Demo Completed Successfully.");
                }
            });

            return Json(ChatResponse { 
                response: "🚀 Vision 검증 데모(Smart Mode)를 시작합니다.\n(Google 접속 -> [검증] -> 키 입력 -> [검증])".to_string(), 
                command: None 
            });
        }
    }

    if let Some(brain) = &state.llm_client {
        // [Context] Fetch recent history
        let history =
            db::get_recent_chat_history(context_pruning::history_fetch_limit()).unwrap_or_default();

        match brain.parse_intent_with_history(&message, &history).await {
            Ok(intent) => {
                let command = intent["command"].as_str().unwrap_or("unknown").to_string();
                let confidence = intent["confidence"].as_f64().unwrap_or(0.0);

                if confidence < 0.5 {
                    return Json(ChatResponse {
                        response: "❓ 무슨 말인지 잘 모르겠어요. 다시 말씀해주세요.".to_string(),
                        command: None,
                    });
                }

                let response = match command.as_str() {
                    "analyze_patterns" => {
                         let results = run_analysis_internal();
                         if results.is_empty() {
                             "🔍 분석 완료! 새로운 패턴을 찾지 못했습니다.".to_string()
                         } else {
                             format!("🔍 분석 완료!:\n{}", results.join("\n"))
                         }
                    },
                    "gmail_list" => {
                        match integrations::gmail::GmailClient::new().await {
                            Ok(client) => match client.list_messages(5).await {
                                Ok(msgs) => {
                                    if msgs.is_empty() {
                                        "📭 새 메일이 없습니다.".to_string()
                                    } else {
                                        let mut s = String::from("📧 최근 이메일 5건:\n");
                                        for (_, subj, from) in msgs {
                                            s.push_str(&format!("• {} ({})\n", subj, from));
                                        }
                                        s
                                    }
                                },
                                Err(e) => format!("❌ 이메일 가져오기 실패: {}", e),
                            },
                            Err(e) => format!("⚠️ Gmail 인증 실패: {}", e),
                        }
                    },
                    "calendar_today" => {
                        match integrations::calendar::CalendarClient::new().await {
                            Ok(client) => match client.list_today().await {
                                Ok(events) => {
                                    if events.is_empty() {
                                        "📅 오늘 일정이 없습니다.".to_string()
                                    } else {
                                        let mut s = String::from("📅 오늘 일정:\n");
                                        for (_, summary, start_time) in events {
                                            s.push_str(&format!("• {} ({})\n", summary, start_time));
                                        }
                                        s
                                    }
                                },
                                Err(e) => format!("❌ 일정 확인 실패: {}", e),
                            },
                            Err(e) => format!("⚠️ Calendar 인증 실패: {}", e),
                        }
                    },
                    "calendar_week" => {
                        match integrations::calendar::CalendarClient::new().await {
                            Ok(client) => match client.list_week().await {
                                Ok(events) => {
                                    if events.is_empty() {
                                        "📅 이번 주 일정이 없습니다.".to_string()
                                    } else {
                                        let mut s = String::from("📅 이번 주 일정:\n");
                                        for (_, summary, start_time) in events {
                                            s.push_str(&format!("• {} ({})\n", summary, start_time));
                                        }
                                        s
                                    }
                                },
                                Err(e) => format!("❌ 일정 확인 실패: {}", e),
                            },
                            Err(e) => format!("⚠️ Calendar 인증 실패: {}", e),
                        }
                    },
                    "system_status" => {
                        let mut rm = monitor::ResourceMonitor::new();
                        format!("📊 {}", rm.get_status())
                    }
                    "build_workflow" => {
                        let prompt_str = intent["params"]["prompt"].as_str()
                            .or_else(|| intent["params"]["description"].as_str())
                            .unwrap_or(&message);
                        let prompt = prompt_str.to_string(); // Clone to owned String for async block
                        let response_msg = format!("🏗️ 자동화 워크플로우 생성을 시작합니다: '{}'\n(잠시만 기다려주세요...)", prompt);
                        
                        // Spawn async task to handle creation (avoid blocking response)
                        let llm_clone = brain.clone();
                        tokio::spawn(async move {
                            match n8n_api::N8nApi::from_env() {
                                Ok(n8n) => {
                                    // 1. Get Credentials Context
                                    let creds = n8n.list_credentials().await.unwrap_or_default();
                                    let cred_context = creds.iter().map(|c| format!("{}:{}", c.name, c.id)).collect::<Vec<_>>().join(", ");
                                    
                                    // Ensure server is running
                                    if let Err(e) = n8n.ensure_server_running().await {
                                        println!("⚠️ n8n Server Check Failed: {}", e);
                                        // Try proceeding anyway? Or return? 
                                        // CLI fallback handles start, so we might be fine, but explicit check is good.
                                    }

                                    // 2. Build via LLM
                                    let cred_str = if cred_context.is_empty() {
                                        "NO CREDENTIALS AVAILABLE. Do NOT use any nodes requiring authentication (like Gmail, Slack, Drive). Use ONLY core nodes (Schedule, Webhook, HTTP Request, etc).".to_string()
                                    } else {
                                        format!("Available Credentials: {}", cred_context)
                                    };
                                    
                                    let full_prompt = format!("Create a n8n workflow for: {}. {}", prompt, cred_str);
                                    match llm_clone.build_n8n_workflow(&full_prompt).await {
                                        Ok(json_str) => {
                                            // Parse JSON string to Value
                                            match serde_json::from_str::<serde_json::Value>(&json_str) {
                                                Ok(json_val) => {
                                                    // 3. Create in n8n (Inactive)
                                                    match n8n.create_workflow("Chat Generated Workflow", &json_val, false).await {
                                                        Ok(id) => println!("✅ Chat-triggered Workflow Created: {}", id),
                                                        Err(e) => println!("❌ Workflow Creation Failed: {}", e),
                                                    }
                                                },
                                                Err(e) => println!("❌ Invalid JSON from LLM: {}", e),
                                            }
                                        },
                                        Err(e) => println!("❌ LLM Generation Failed: {}", e),
                                    }
                                },
                                Err(e) => println!("❌ n8n Client Error: {}", e),
                            }
                        });
                        
                        response_msg
                    },
                    "create_routine" => {
                        let params = intent["params"].as_object();
                        if let Some(p) = params {
                            let name = p.get("name").and_then(|v| v.as_str()).unwrap_or("New Routine");
                            let cron = p.get("cron").and_then(|v| v.as_str()).unwrap_or("* * * * *");
                            
                            // Validate Cron
                            if std::str::FromStr::from_str(&cron as &str).map(|_: cron::Schedule| ()).is_err() {
                                format!("❌ 잘못된 Cron 표현식입니다: {}", cron)
                            } else {
                                let prompt = p.get("prompt").and_then(|v| v.as_str()).unwrap_or("Check status");
                            
                            match crate::db::create_routine(name, cron, prompt) {
                                Ok(_id) => format!("✅ 루틴이 등록되었습니다!\n• 이름: {}\n• 주기: {}\n• 명령: {}", name, cron, prompt),
                                Err(e) => format!("❌ 루틴 등록 실패: {}", e),
                            }
                            }
                        } else {
                             "❌ 루틴 정보를 파악할 수 없습니다.".to_string()
                        }
                    },
                    "help" => "💡 사용 가능한 명령:\n• '이메일 보여줘'\n• '오늘 일정 뭐야?'\n• '매일 아침 9시 뉴스 요약해줘' (New!)".to_string(),
                    _ => format!("✅ '{}' 명령을 실행합니다.", command),
                };

                let final_response = ChatResponse {
                    response: response.clone(),
                    command: Some(command),
                };

                // [Memory] Save Assistant Response
                if let Err(e) = db::insert_chat_message("assistant", &response) {
                    eprintln!("Failed to save AI chat: {}", e);
                }

                Json(final_response)
            }
            Err(e) => Json(ChatResponse {
                response: format!("❌ 오류: {}", e),
                command: None,
            }),
        }
    } else {
        Json(ChatResponse {
            response: "⚠️ LLM 클라이언트가 없습니다.".to_string(),
            command: None,
        })
    }
}

// --- Routine Handlers ---

async fn list_routines() -> Json<Vec<crate::db::Routine>> {
    match crate::db::get_all_routines() {
        Ok(routines) => Json(routines),
        Err(e) => {
            eprintln!("Failed to list routines: {}", e);
            Json(Vec::new())
        }
    }
}

#[derive(serde::Deserialize)]
struct CreateRoutineRequest {
    name: String,
    #[serde(alias = "cron_expression")] // Accept both "cron" and "cron_expression"
    cron: String,
    prompt: String,
}

async fn create_routine_handler(
    Json(payload): Json<CreateRoutineRequest>,
) -> Json<serde_json::Value> {
    match crate::db::create_routine(&payload.name, &payload.cron, &payload.prompt) {
        Ok(id) => Json(serde_json::json!({ "status": "ok", "id": id })),
        Err(e) => Json(serde_json::json!({ "status": "error", "message": e.to_string() })),
    }
}

// --- Issue #2 Fix: Toggle Routine ---
#[derive(serde::Deserialize)]
struct ToggleRoutineRequest {
    enabled: bool,
}

async fn toggle_routine_handler(
    axum::extract::Path(id): axum::extract::Path<i64>,
    Json(payload): Json<ToggleRoutineRequest>,
) -> Json<serde_json::Value> {
    match crate::db::toggle_routine(id, payload.enabled) {
        Ok(_) => Json(serde_json::json!({ "status": "ok" })),
        Err(e) => Json(serde_json::json!({ "status": "error", "message": e.to_string() })),
    }
}

#[derive(serde::Deserialize)]
struct RecQueryParams {
    status: Option<String>,
}

async fn list_recommendations(
    Query(params): Query<RecQueryParams>,
) -> Json<Vec<RecommendationItem>> {
    // Treat empty string as None; default to "all" for history view.
    let filter = params
        .status
        .as_deref()
        .filter(|s| !s.is_empty())
        .or(Some("all"));

    match db::get_recommendations_with_filter(filter) {
        Ok(recs) => Json(
            recs.into_iter()
                .map(|r| RecommendationItem {
                    id: r.id,
                    status: r.status,
                    title: r.title,
                    summary: r.summary,
                    confidence: r.confidence,
                    evidence: r.evidence, // [NEW] Pass evidence
                    last_error: r.last_error,
                })
                .collect(),
        ),
        Err(_) => Json(vec![]),
    }
}

async fn approve_recommendation(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<i64>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    println!("🔔 Received approval request for Recommendation ID: {}", id);

    // 1. Get recommendation from DB
    let rec = match db::get_recommendation(id) {
        Ok(Some(r)) => r,
        _ => {
            eprintln!("❌ Recommendation #{} not found in DB", id);
            return Err((
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Recommendation not found"})),
            ));
        }
    };

    let n8n_client = match n8n_api::N8nApi::from_env() {
        Ok(c) => c,
        Err(e) => {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(
                    serde_json::json!({ "error": "n8n Client Init Failed", "details": e.to_string() }),
                ),
            ));
        }
    };

    // Ensure n8n is running first
    if let Err(e) = n8n_client.ensure_server_running().await {
        eprintln!("❌ Failed to start n8n: {}", e);
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(
                serde_json::json!({ "error": "n8n Server Unavailable", "details": e.to_string() }),
            ),
        ));
    }

    // [NEW] Fetch Credentials to inform LLM
    let credentials = n8n_client.list_credentials().await.unwrap_or_default();
    let cred_context = if credentials.is_empty() {
        "NOTE: No credentials found in n8n. Do NOT use nodes requiring authentication (like Gmail, Slack) unless you are sure.".to_string()
    } else {
        let list = credentials
            .iter()
            .map(|c| {
                format!(
                    "- Name: '{}', ID: '{}', Type: '{}'",
                    c.name, c.id, c.type_name
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        format!("IMPORTANT: You MUST use these exact Credential IDs for authentication:\n{}\nIf a required credential is missing, do not hallucinate an ID. Use a placeholder and add a comment.", list)
    };

    let mut current_json = if let Some(json) = &rec.workflow_json {
        json.clone()
    } else {
        // Initial Generation with Credential Context
        if let Some(llm) = &state.llm_client {
            let full_prompt = format!("{}\n\n{}", rec.n8n_prompt, cred_context);
            println!(
                "🧠 Generating workflow with context: {} credentials",
                credentials.len()
            );

            match llm.build_n8n_workflow(&full_prompt).await {
                Ok(json) => json,
                Err(e) => {
                    return Err((
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(
                            serde_json::json!({ "error": "LLM Generation Failed", "details": e.to_string() }),
                        ),
                    ))
                }
            }
        } else {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({ "error": "LLM Client Unavailable" })),
            ));
        }
    };

    let mut attempts = 0;
    let max_attempts = 3;
    let mut last_error = String::new();

    while attempts < max_attempts {
        attempts += 1;

        // Repair JSON if this is a retry
        if attempts > 1 {
            if let Some(llm) = &state.llm_client {
                println!(
                    "🔧 Attempting to fix workflow JSON (Try {}/{})",
                    attempts, max_attempts
                );
                // Also pass credential context during fix
                let fix_prompt = format!("{}\n\n{}", rec.n8n_prompt, cred_context);
                match llm
                    .fix_n8n_workflow(&fix_prompt, &current_json, &last_error)
                    .await
                {
                    Ok(fixed) => current_json = fixed,
                    Err(e) => println!("Failed to fix JSON: {}", e),
                }
            }
        }

        // Parse JSON to Value
        let workflow_data: serde_json::Value =
            match serde_json::from_str(&current_json).or_else(|_| {
                let cleaned = extract_json_object(&current_json);
                serde_json::from_str(&cleaned)
            }) {
                Ok(v) => v,
                Err(e) => {
                    last_error = format!("Invalid JSON Syntax: {}", e);
                    continue;
                }
            };

        // Extract name
        let name = workflow_data["name"]
            .as_str()
            .unwrap_or(&rec.title)
            .to_string();

        // Try create
        // SAFETY: Created as inactive (false) to prevent broken loops. User must enable manually.
        match n8n_client
            .create_workflow(&name, &workflow_data, false)
            .await
        {
            Ok(workflow_id) => {
                // Success!
                println!("✅ Workflow created successfully on attempt {}", attempts);
                if let Err(e) = db::mark_recommendation_approved(id, &workflow_id, &current_json) {
                    eprintln!("Failed to update DB: {}", e);
                }
                return Ok(Json(serde_json::json!({
                    "status": "success",
                    "id": workflow_id,
                    "message": "Workflow created successfully"
                })));
            }
            Err(e) => {
                last_error = e.to_string();
                println!("❌ Creation failed: {}", last_error);
            }
        }
    }

    // If we get here, all attempts failed
    let error_msg = format!(
        "❌ All {} attempts to create workflow failed. Last Error: {}",
        max_attempts, last_error
    );
    eprintln!("{}", error_msg);
    if let Err(db_err) = db::mark_recommendation_failed(id, &last_error) {
        eprintln!("Failed to mark recommendation as failed: {}", db_err);
    }

    Err((
        StatusCode::BAD_GATEWAY,
        Json(serde_json::json!({ "error": error_msg, "details": last_error })),
    ))
}

fn extract_json_object(input: &str) -> String {
    let start = input.find('{');
    let end = input.rfind('}');
    match (start, end) {
        (Some(s), Some(e)) if e > s => input[s..=e].to_string(),
        _ => input.to_string(),
    }
}

async fn reject_recommendation(axum::extract::Path(id): axum::extract::Path<i64>) -> StatusCode {
    match db::update_recommendation_status(id, "rejected") {
        Ok(_) => StatusCode::OK,
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

async fn later_recommendation(axum::extract::Path(id): axum::extract::Path<i64>) -> StatusCode {
    match db::update_recommendation_status(id, "later") {
        Ok(_) => StatusCode::OK,
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

async fn restore_recommendation(axum::extract::Path(id): axum::extract::Path<i64>) -> StatusCode {
    match db::update_recommendation_status(id, "pending") {
        Ok(_) => StatusCode::OK,
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

async fn list_exec_approvals(
    Query(query): Query<ExecApprovalQuery>,
) -> Json<Vec<db::ExecApproval>> {
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    let status = match query.status.as_deref() {
        Some("all") => None,
        other => other,
    };
    let approvals = db::list_exec_approvals(status, limit).unwrap_or_default();
    Json(approvals)
}

async fn approve_exec_approval(
    Path(id): Path<String>,
    payload: Option<Json<ExecApprovalResolve>>,
) -> StatusCode {
    let resolved_by = payload.as_ref().and_then(|p| p.resolved_by.as_deref());
    let decision = payload
        .as_ref()
        .and_then(|p| p.decision.as_deref())
        .unwrap_or("allow-once");

    if decision == "allow-always" {
        if let Ok(Some(approval)) = db::get_exec_approval(&id) {
            let _ = db::add_exec_allowlist(&approval.command, approval.cwd.as_deref());
        }
    }

    match db::resolve_exec_approval(&id, "approved", resolved_by, Some(decision)) {
        Ok(_) => StatusCode::OK,
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

async fn reject_exec_approval(
    Path(id): Path<String>,
    payload: Option<Json<ExecApprovalResolve>>,
) -> StatusCode {
    let resolved_by = payload.as_ref().and_then(|p| p.resolved_by.as_deref());
    match db::resolve_exec_approval(&id, "rejected", resolved_by, Some("deny")) {
        Ok(_) => StatusCode::OK,
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

async fn list_routine_runs(Query(query): Query<RoutineRunsQuery>) -> Json<Vec<db::RoutineRun>> {
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    let runs = db::list_routine_runs(limit).unwrap_or_default();
    Json(runs)
}

async fn list_exec_allowlist(
    Query(query): Query<ExecAllowlistQuery>,
) -> Json<Vec<db::ExecAllowlistEntry>> {
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    let entries = db::list_exec_allowlist(limit).unwrap_or_default();
    Json(entries)
}

async fn list_exec_results(Query(query): Query<ExecResultsQuery>) -> Json<Vec<db::ExecResult>> {
    let limit = query.limit.unwrap_or(100).clamp(1, 500);
    let status = query.status.as_deref();
    let results = db::list_exec_results(status, limit).unwrap_or_default();
    Json(results)
}

async fn list_verification_runs(
    Query(query): Query<VerificationRunsQuery>,
) -> Json<Vec<db::VerificationRun>> {
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    let runs = db::list_verification_runs(limit).unwrap_or_default();
    Json(runs)
}

async fn list_nl_runs_handler(Query(query): Query<NLRunQuery>) -> Json<Vec<db::NLRun>> {
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    let runs = db::list_nl_runs(limit).unwrap_or_default();
    Json(runs)
}

async fn nl_run_metrics_handler(Query(query): Query<NLRunMetricsQuery>) -> Json<db::NLRunMetrics> {
    let limit = query.limit.unwrap_or(50).clamp(1, 500);
    let metrics = db::get_nl_run_metrics(limit).unwrap_or(db::NLRunMetrics {
        total: 0,
        completed: 0,
        manual_required: 0,
        approval_required: 0,
        blocked: 0,
        error: 0,
        success_rate: 0.0,
    });
    Json(metrics)
}

async fn list_nl_approval_policies(
    Query(query): Query<ApprovalPolicyQuery>,
) -> Json<Vec<ApprovalPolicyResponse>> {
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    let policies = db::list_approval_policies(limit).unwrap_or_default();
    let mapped = policies
        .into_iter()
        .map(|policy| ApprovalPolicyResponse {
            policy_key: policy.policy_key,
            decision: policy.decision,
            updated_at: policy.updated_at,
        })
        .collect();
    Json(mapped)
}

async fn set_nl_approval_policy(Json(payload): Json<ApprovalPolicyRequest>) -> StatusCode {
    if payload.policy_key.trim().is_empty() || payload.decision.trim().is_empty() {
        return StatusCode::BAD_REQUEST;
    }
    match db::upsert_approval_policy(&payload.policy_key, &payload.decision) {
        Ok(_) => StatusCode::OK,
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

async fn remove_nl_approval_policy(Path(key): Path<String>) -> StatusCode {
    if key.trim().is_empty() {
        return StatusCode::BAD_REQUEST;
    }
    match db::delete_approval_policy(&key) {
        Ok(_) => StatusCode::OK,
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

fn log_verification_run(kind: &str, ok: bool, summary: &str, details: Option<serde_json::Value>) {
    let details_str = details.map(|v| v.to_string());
    let _ = db::insert_verification_run(kind, ok, summary, details_str.as_deref());
}

async fn add_exec_allowlist(Json(payload): Json<ExecAllowlistRequest>) -> StatusCode {
    if payload.pattern.trim().is_empty() {
        return StatusCode::BAD_REQUEST;
    }
    match db::add_exec_allowlist(&payload.pattern, payload.cwd.as_deref()) {
        Ok(_) => StatusCode::CREATED,
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

async fn remove_exec_allowlist(Path(id): Path<i64>) -> StatusCode {
    match db::remove_exec_allowlist(id) {
        Ok(_) => StatusCode::OK,
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

#[derive(Serialize)]
struct RecommendationMetricsResponse {
    total: i64,
    approved: i64,
    rejected: i64,
    failed: i64,
    pending: i64,
    later: i64,
    approval_rate: f64,
    last_created_at: Option<String>,
}

async fn get_recommendation_metrics() -> Json<RecommendationMetricsResponse> {
    let metrics = db::get_recommendation_metrics().unwrap_or(crate::db::RecommendationMetrics {
        total: 0,
        approved: 0,
        rejected: 0,
        failed: 0,
        pending: 0,
        later: 0,
        last_created_at: None,
    });

    let approval_rate = if metrics.total > 0 {
        (metrics.approved as f64 / metrics.total as f64) * 100.0
    } else {
        0.0
    };

    Json(RecommendationMetricsResponse {
        total: metrics.total,
        approved: metrics.approved,
        rejected: metrics.rejected,
        failed: metrics.failed,
        pending: metrics.pending,
        later: metrics.later,
        approval_rate,
        last_created_at: metrics.last_created_at,
    })
}

// Add at top: use crate::recommendation::AutomationProposal;

async fn analyze_patterns() -> Json<Vec<String>> {
    Json(run_analysis_internal())
}

fn run_analysis_internal() -> Vec<String> {
    let detector = pattern_detector::PatternDetector::new();
    let patterns = detector.analyze();

    // 1. Save detected patterns to DB
    for p in &patterns {
        let proposal = crate::recommendation::AutomationProposal {
            title: format!("New Pattern: {}", p.description),
            summary: format!(
                "Detected {} repeats. AI suggests automating this.",
                p.occurrences
            ),
            trigger: format!("Pattern Type: {:?}", p.pattern_type),
            actions: vec!["Analyze".to_string(), "Automate".to_string()],
            n8n_prompt: format!("Create an automation for: {}", p.description),
            confidence: p.similarity_score,
            evidence: vec![format!("Pattern: {}", p.description)],
            pattern_id: Some(p.pattern_id.clone()),
        };
        if let Err(e) = db::insert_recommendation(&proposal) {
            eprintln!("Failed to save pattern: {}", e);
        }
    }

    // 2. Fallback: If empty, create a random demo recommendation (For User Experience)
    // DISABLED: Random spam fix
    /*
    if patterns.is_empty() {
        let timestamp = chrono::Utc::now().timestamp() % 1000;
        let proposal = crate::recommendation::AutomationProposal {
            title: format!("Smart Recommendation #{}", timestamp),
            summary: "AI has identified a potential workflow improvement based on recent activity.".to_string(),
            trigger: "System Activity Analysis".to_string(),
            actions: vec!["Log Activity".to_string(), "Send Notification".to_string()],
            n8n_prompt: "Create a workflow that logs system activity and sends a summary notification.".to_string(),
            confidence: 0.85,
        };
        if let Err(e) = db::insert_recommendation(&proposal) {
            eprintln!("⚠️ Failed to save recommendation analysis: {}", e);
        }
        return vec![];
    }
    */

    patterns
        .into_iter()
        .map(|p| format!("{} ({} occurrences)", p.description, p.occurrences))
        .collect()
}

async fn get_quality_metrics() -> Json<QualityMetrics> {
    let collector = feedback_collector::FeedbackCollector::new();
    let metrics = collector.get_quality_metrics();

    Json(QualityMetrics {
        total: metrics.total_executions,
        success: metrics.successful_executions,
        rate: metrics.success_rate,
    })
}

#[derive(serde::Deserialize)]
struct GoalRequest {
    goal: String,
}

async fn execute_goal_handler(
    State(state): State<AppState>,
    Json(payload): Json<GoalRequest>,
) -> Json<serde_json::Value> {
    if let Ok(mut guard) = state.current_goal.lock() {
        *guard = Some(payload.goal.clone());
    }
    if let Some(llm) = state.llm_client {
        // Spawn background task for OODA loop
        tokio::spawn(async move {
            let planner = crate::controller::planner::Planner::new(llm, None);
            match planner.run_goal(&payload.goal, None).await {
                Ok(_) => println!("✅ Goal Execution Success"),
                Err(e) => println!("❌ Goal Execution Failed: {}", e),
            }
        });

        Json(serde_json::json!({
            "status": "started",
            "message": "Autonmous Agent started. Monitor logs for progress."
        }))
    } else {
        Json(serde_json::json!({
            "status": "error",
            "message": "LLM Client not available"
        }))
    }
}

async fn get_current_goal(State(state): State<AppState>) -> Json<serde_json::Value> {
    let goal = state
        .current_goal
        .lock()
        .ok()
        .and_then(|g| g.clone())
        .unwrap_or_default();
    Json(serde_json::json!({ "goal": goal }))
}

async fn agent_intent_handler(Json(payload): Json<AgentIntentRequest>) -> impl IntoResponse {
    let intent_result = intent_router::classify_intent(&payload.text);
    let fill = slot_filler::fill_slots(&intent_result.intent, intent_result.slots.clone());
    let session = nl_store::create_session(
        intent_result.clone(),
        fill.slots.clone(),
        payload.text.clone(),
    );

    let response = AgentIntentResponse {
        session_id: session.session_id,
        intent: intent_result.intent.as_str().to_string(),
        confidence: intent_result.confidence,
        slots: fill.slots,
        missing_slots: fill.missing,
        follow_up: fill.follow_up,
    };

    (StatusCode::OK, Json(response)).into_response()
}

async fn agent_plan_handler(Json(payload): Json<AgentPlanRequest>) -> impl IntoResponse {
    let Some(mut session) = nl_store::get_session(&payload.session_id) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "session_not_found" })),
        )
            .into_response();
    };

    if let Some(updates) = payload.slots.as_ref() {
        if let Some(updated) = nl_store::update_session_slots(&payload.session_id, updates) {
            session = updated;
        }
    }

    let fill = slot_filler::fill_slots(&session.intent.intent, session.slots.clone());
    let plan = plan_builder::build_plan(&session.intent.intent, &fill.slots);
    let _ = nl_store::set_session_plan(&payload.session_id, plan.clone());

    let response = AgentPlanResponse {
        plan_id: plan.plan_id,
        intent: session.intent.intent.as_str().to_string(),
        steps: plan.steps,
        missing_slots: fill.missing,
    };

    (StatusCode::OK, Json(response)).into_response()
}

async fn agent_execute_handler(Json(payload): Json<AgentExecuteRequest>) -> impl IntoResponse {
    let Some(plan) = nl_store::get_plan(&payload.plan_id) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "plan_not_found" })),
        )
            .into_response();
    };

    let mut resume_from = nl_store::get_plan_progress(&plan.plan_id).unwrap_or(0);
    if resume_from >= plan.steps.len() {
        nl_store::clear_plan_progress(&plan.plan_id);
        resume_from = 0;
    }
    let mut result = execution_controller::execute_plan(&plan, resume_from).await;
    if resume_from > 0 {
        result
            .logs
            .insert(0, format!("Resuming from step {}", resume_from + 1));
    }
    let mut verify = verification_engine::verify_execution(&plan, &result.logs);
    if !verify.ok {
        result
            .logs
            .push(format!("Verification failed: {}", verify.issues.join("; ")));
    } else {
        result.logs.push("Verification passed".to_string());
    }

    // Simple auto-replan: one retry on failure or verification issues
    let auto_replan = std::env::var("STEER_AUTO_REPLAN")
        .ok()
        .map(|v| {
            matches!(
                v.trim().to_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(true);
    let allow_replan = !matches!(
        result.status.as_str(),
        "manual_required" | "approval_required"
    );
    if auto_replan && allow_replan && (result.status == "error" || !verify.ok) {
        result
            .logs
            .push("Auto-replan: retrying once after short wait".to_string());
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        let retry = execution_controller::execute_plan(&plan, 0).await;
        result.logs = retry.logs;
        result.status = retry.status;
        result.approval = retry.approval;
        result.manual_steps = retry.manual_steps;
        result.resume_from = retry.resume_from;
        verify = verification_engine::verify_execution(&plan, &result.logs);
        if !verify.ok {
            result
                .logs
                .push(format!("Verification failed: {}", verify.issues.join("; ")));
        } else {
            result.logs.push("Verification passed".to_string());
        }
    }

    if matches!(
        result.status.as_str(),
        "manual_required" | "approval_required"
    ) {
        if let Some(next_step) = result.resume_from {
            nl_store::set_plan_progress(&plan.plan_id, next_step);
        }
    } else {
        nl_store::clear_plan_progress(&plan.plan_id);
    }
    let session = nl_store::find_session_by_plan(&payload.plan_id);
    let summary = extract_summary(&result.logs);
    if let Some(state) = session {
        let _ = db::insert_nl_run(
            state.intent.intent.as_str(),
            &state.prompt,
            &result.status,
            summary.as_deref(),
            Some(&serde_json::to_string(&result.logs).unwrap_or_default()),
        );
    }
    let response = AgentExecuteResponse {
        status: result.status,
        logs: result.logs,
        approval: result.approval,
        manual_steps: result.manual_steps,
        resume_from: result.resume_from,
    };

    (StatusCode::OK, Json(response)).into_response()
}

async fn agent_verify_handler(Json(payload): Json<AgentVerifyRequest>) -> impl IntoResponse {
    let Some(plan) = nl_store::get_plan(&payload.plan_id) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "plan_not_found" })),
        )
            .into_response();
    };

    let result = verification_engine::verify_plan(&plan);
    let response = AgentVerifyResponse {
        ok: result.ok,
        issues: result.issues,
    };

    (StatusCode::OK, Json(response)).into_response()
}

async fn agent_approve_handler(Json(payload): Json<AgentApproveRequest>) -> impl IntoResponse {
    let Some(plan) = nl_store::get_plan(&payload.plan_id) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "plan_not_found" })),
        )
            .into_response();
    };

    if let Some(decision) = payload.decision.as_deref() {
        approval_gate::register_decision(decision, &payload.action, &plan);
    }
    let decision = approval_gate::preview_approval(&payload.action, &plan);
    let response = AgentApproveResponse {
        status: decision.status,
        requires_approval: decision.requires_approval,
        message: decision.message,
        risk_level: decision.risk_level,
        policy: decision.policy,
    };

    (StatusCode::OK, Json(response)).into_response()
}

fn extract_summary(logs: &[String]) -> Option<String> {
    logs.iter()
        .find_map(|line| line.strip_prefix("Summary: ").map(|s| s.to_string()))
}

async fn handle_feedback(
    State(state): State<AppState>,
    Json(req): Json<FeedbackRequest>,
) -> Json<FeedbackResponse> {
    let goal = if req.goal.trim().is_empty() {
        state
            .current_goal
            .lock()
            .ok()
            .and_then(|g| g.clone())
            .unwrap_or_default()
    } else {
        req.goal.clone()
    };
    let history = req
        .history_summary
        .unwrap_or_else(|| format!("Goal: {}", goal));
    if let Some(llm) = &state.llm_client {
        match llm.analyze_user_feedback(&req.feedback, &history).await {
            Ok(analysis) => {
                let action = analysis.action.clone();
                let new_goal = if action == "refine" {
                    analysis.new_goal.clone().or(Some(goal))
                } else {
                    None
                };
                let message = if action == "refine" {
                    "피드백을 반영해 목표를 업데이트했어요. 다시 실행할 수 있어요.".to_string()
                } else {
                    "피드백을 확인했어요. 작업을 완료로 표시합니다.".to_string()
                };
                return Json(FeedbackResponse {
                    action,
                    new_goal,
                    message,
                });
            }
            Err(e) => {
                eprintln!("Feedback analysis failed: {}", e);
            }
        }
    }

    Json(FeedbackResponse {
        action: "complete".to_string(),
        new_goal: None,
        message: "피드백을 확인했어요. 작업을 완료로 표시합니다.".to_string(),
    })
}

// [Context] Selection Handler
async fn get_selection_context() -> Json<serde_json::Value> {
    #[cfg(target_os = "macos")]
    {
        match crate::macos::accessibility::get_selected_text() {
            Some(text) => Json(serde_json::json!({ "found": true, "text": text })),
            None => Json(serde_json::json!({ "found": false, "text": "" })),
        }
    }
    #[cfg(not(target_os = "macos"))]
    Json(serde_json::json!({ "found": false, "text": "", "error": "Not supported on this OS" }))
}

// =====================================================
// SESSION MANAGEMENT HANDLERS (Clawdbot-ported)
// =====================================================

/// List all sessions
async fn list_sessions_handler() -> Json<serde_json::Value> {
    let _ = crate::session_store::init_session_store();

    match crate::session_store::get_session_store() {
        Ok(guard) => {
            if let Some(store) = guard.as_ref() {
                let sessions: Vec<serde_json::Value> = store
                    .list_active()
                    .iter()
                    .map(|s| {
                        serde_json::json!({
                            "id": s.id,
                            "key": s.key,
                            "goal": s.goal,
                            "status": format!("{:?}", s.status),
                            "created_at": s.created_at.to_rfc3339(),
                            "updated_at": s.updated_at.to_rfc3339(),
                            "steps_count": s.steps.len(),
                            "can_resume": s.can_resume(),
                        })
                    })
                    .collect();

                Json(serde_json::json!({
                    "success": true,
                    "sessions": sessions,
                    "total": sessions.len()
                }))
            } else {
                Json(serde_json::json!({ "success": false, "error": "Store not initialized" }))
            }
        }
        Err(e) => Json(serde_json::json!({ "success": false, "error": e.to_string() })),
    }
}

/// Get specific session by ID
async fn get_session_handler(Path(id): Path<String>) -> Json<serde_json::Value> {
    let _ = crate::session_store::init_session_store();

    match crate::session_store::get_session_store() {
        Ok(guard) => {
            if let Some(store) = guard.as_ref() {
                if let Some(session) = store.get(&id) {
                    Json(serde_json::json!({
                        "success": true,
                        "session": {
                            "id": session.id,
                            "key": session.key,
                            "goal": session.goal,
                            "status": format!("{:?}", session.status),
                            "created_at": session.created_at.to_rfc3339(),
                            "updated_at": session.updated_at.to_rfc3339(),
                            "messages": session.messages,
                            "steps": session.steps,
                            "can_resume": session.can_resume(),
                            "resume_point": session.get_resume_point()
                        }
                    }))
                } else {
                    Json(serde_json::json!({ "success": false, "error": "Session not found" }))
                }
            } else {
                Json(serde_json::json!({ "success": false, "error": "Store not initialized" }))
            }
        }
        Err(e) => Json(serde_json::json!({ "success": false, "error": e.to_string() })),
    }
}

/// Delete a session
async fn delete_session_handler(Path(id): Path<String>) -> Json<serde_json::Value> {
    let _ = crate::session_store::init_session_store();

    match crate::session_store::get_session_store() {
        Ok(mut guard) => {
            if let Some(store) = guard.as_mut() {
                match store.delete(&id) {
                    Ok(true) => Json(serde_json::json!({ "success": true, "deleted": id })),
                    Ok(false) => {
                        Json(serde_json::json!({ "success": false, "error": "Session not found" }))
                    }
                    Err(e) => Json(serde_json::json!({ "success": false, "error": e.to_string() })),
                }
            } else {
                Json(serde_json::json!({ "success": false, "error": "Store not initialized" }))
            }
        }
        Err(e) => Json(serde_json::json!({ "success": false, "error": e.to_string() })),
    }
}

/// Resume a paused/failed session
async fn resume_session_handler(Path(id): Path<String>) -> Json<serde_json::Value> {
    let _ = crate::session_store::init_session_store();

    match crate::session_store::get_session_store() {
        Ok(mut guard) => {
            if let Some(store) = guard.as_mut() {
                if let Some(session) = store.get_mut(&id) {
                    if session.can_resume() {
                        let resume_point = session.get_resume_point();
                        let goal = session.goal.clone();
                        session.status = crate::session_store::SessionStatus::Active;
                        session
                            .add_message("system", &format!("Resuming from step {}", resume_point));

                        Json(serde_json::json!({
                            "success": true,
                            "resumed": true,
                            "session_id": id,
                            "goal": goal,
                            "resume_from_step": resume_point,
                            "message": format!("Session resumed from step {}", resume_point)
                        }))
                    } else {
                        Json(serde_json::json!({
                            "success": false,
                            "error": "Session cannot be resumed (status or no steps)"
                        }))
                    }
                } else {
                    Json(serde_json::json!({ "success": false, "error": "Session not found" }))
                }
            } else {
                Json(serde_json::json!({ "success": false, "error": "Store not initialized" }))
            }
        }
        Err(e) => Json(serde_json::json!({ "success": false, "error": e.to_string() })),
    }
}
