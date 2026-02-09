use local_os_agent::config_manager::ConfigManager;
use local_os_agent::controller::planner::Planner;
use local_os_agent::db::{self, DashboardStats, LearnedRoutine};
use local_os_agent::llm_gateway::{LLMClient, OpenAILLMClient};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;
use tauri::{Manager, State};

struct AgentState {
    llm: Option<Arc<dyn LLMClient>>,
}

#[derive(Debug, Clone, Serialize)]
struct RecommendationItem {
    id: i64,
    status: String,
    title: String,
    summary: String,
    confidence: f64,
}

#[tauri::command]
async fn run_agent_task(goal: String, state: State<'_, AgentState>) -> Result<String, String> {
    let Some(llm) = &state.llm else {
        return Err("LLM client is not initialized".to_string());
    };

    let planner = Planner::new(llm.clone(), None);
    planner
        .run_goal(&goal, None)
        .await
        .map_err(|e| e.to_string())?;
    Ok("Task Completed.".to_string())
}

#[tauri::command]
async fn start_architect_mode(_rec_id: i64) -> Result<String, String> {
    Err("Architect mode is not available in this build".to_string())
}

#[tauri::command]
async fn get_dashboard_data() -> Result<DashboardStats, String> {
    db::get_dashboard_stats().map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_recommendations() -> Result<Vec<RecommendationItem>, String> {
    let items = db::get_recent_recommendations(10)
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|rec| RecommendationItem {
            id: rec.id,
            status: rec.status,
            title: rec.title,
            summary: rec.summary,
            confidence: rec.confidence,
        })
        .collect();
    Ok(items)
}

#[tauri::command]
async fn get_config() -> Result<HashMap<String, String>, String> {
    let cm = ConfigManager::new();
    Ok(cm.get_all())
}

#[tauri::command]
async fn set_config(key: String, value: String) -> Result<(), String> {
    let cm = ConfigManager::new();
    cm.update(&key, &value)
}

#[tauri::command]
async fn list_routines() -> Result<Vec<LearnedRoutine>, String> {
    db::list_learned_routines().map_err(|e| e.to_string())
}

#[tauri::command]
async fn delete_routine(id: i64) -> Result<(), String> {
    db::delete_learned_routine(id).map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(
            tauri_plugin_log::Builder::default()
                .target(tauri_plugin_log::Target::new(
                    tauri_plugin_log::TargetKind::Stdout,
                ))
                .target(tauri_plugin_log::Target::new(
                    tauri_plugin_log::TargetKind::LogDir { file_name: None },
                ))
                .target(tauri_plugin_log::Target::new(
                    tauri_plugin_log::TargetKind::Webview,
                ))
                .level(log::LevelFilter::Info)
                .build(),
        )
        .setup(|app| {
            let resource_path = app
                .path()
                .resource_dir()
                .map(|path| path.join(".env"))
                .unwrap_or_else(|_| std::path::PathBuf::from(".env"));

            if resource_path.exists() {
                dotenv::from_path(&resource_path).ok();
            } else {
                dotenv::dotenv().ok();
            }

            if let Err(err) = db::init() {
                log::warn!("failed to initialize db: {}", err);
            }

            let llm = OpenAILLMClient::new()
                .ok()
                .map(|client| Arc::new(client) as Arc<dyn LLMClient>);
            app.manage(AgentState { llm });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            run_agent_task,
            get_dashboard_data,
            get_recommendations,
            start_architect_mode,
            get_config,
            set_config,
            list_routines,
            delete_routine
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
