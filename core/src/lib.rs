pub mod action_schema;
pub mod analyzer;
pub mod applescript;
pub mod approval_gate;
pub mod bash_executor;
pub mod browser_automation;
pub mod controller;
pub mod db;
pub mod dependency_check;
pub mod external_apis;
pub mod llm_gateway;
pub mod mcp_client;
pub mod monitor;
pub mod n8n_api;
pub mod notifier;
pub mod peekaboo_cli;
pub mod permission_manager;
pub mod policy;
pub mod prompts;
pub mod retry_logic;
pub mod scheduler;
pub mod schema;
pub mod session;
pub mod session_store;
pub mod subagent;
pub mod tool_chaining;

pub mod api_server;
pub mod chat_sanitize;
pub mod feedback_collector;
pub mod integrations;
pub mod memory;
pub mod orchestrator;
pub mod pattern_detector;
pub mod privacy;
pub mod recommendation;
pub mod recommendation_executor;
pub mod security;
pub mod send_policy;
pub mod shell_actions;
pub mod shell_analysis;
pub mod visual_driver;
pub mod workflow_intake;
pub mod workflow_schema;

pub mod collector_pipeline;
pub mod command_queue;
pub mod context_pruning;
pub mod project_scanner;
pub mod runtime_verification;
pub mod tool_policy;

pub mod chat_gate;
pub mod consistency_check;
pub mod judgment;
pub mod performance_verification;
pub mod quality_scorer;
pub mod release_gate;
pub mod semantic_contract;
pub mod semantic_verification;
pub mod singleton_lock;
pub mod static_checks;
pub mod tool_result_guard;
pub mod visual_verification;

pub mod execution_controller;
pub mod intent_router;
pub mod nl_automation;
pub mod nl_store;
pub mod plan_builder;
pub mod slot_filler;
pub mod verification_engine;

pub mod error;

pub mod cli_llm;
pub mod config_manager;
pub mod content_extractor;
pub mod macos;
pub mod reality_check;
pub mod screen_recorder;
pub mod telegram;

pub fn load_env_with_fallback() {
    use std::sync::Once;
    static INIT: Once = Once::new();

    INIT.call_once(|| {
        let _ = dotenv::dotenv();

        let mut candidates: Vec<std::path::PathBuf> = vec![
            std::path::PathBuf::from("core/.env"),
            std::path::PathBuf::from(".env"),
        ];

        if let Ok(exe) = std::env::current_exe() {
            for ancestor in exe.ancestors().take(8) {
                candidates.push(ancestor.join(".env"));
                candidates.push(ancestor.join("core").join(".env"));
            }
        }

        for path in candidates {
            if path.exists() {
                let _ = dotenv::from_path(path);
            }
        }
    });
}

pub fn env_flag(key: &str) -> bool {
    std::env::var(key)
        .ok()
        .map(|v| {
            matches!(
                v.trim().to_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}
