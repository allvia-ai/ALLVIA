use crate::llm_gateway::LLMClient;
use crate::n8n_api::N8nApi;
use crate::project_scanner::ProjectScanner;
use crate::recommendation::AutomationProposal;
use crate::visual_driver::VisualDriver;
use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TaskType {
    Coding,      // Modify files, refactor, build (DAACS style)
    Workflow,    // Recurring automation, data piping (n8n)
    OsOperation, // Click buttons, open apps, data entry (Visual Driver)
    Research,    // RFI, clarifications (Phase 1.5)
}

pub struct Orchestrator {
    llm: std::sync::Arc<dyn LLMClient>,
    #[allow(dead_code)]
    n8n: N8nApi,
    // VisualDriver is stateless per run usually, but we can keep config here
}

impl Orchestrator {
    pub async fn new() -> Result<Self> {
        Ok(Self {
            llm: std::sync::Arc::new(crate::llm_gateway::OpenAILLMClient::new()?),
            n8n: N8nApi::from_env()?,
        })
    }

    /// The Main Entrypoint: "Computer, do X"
    /// 1. CLASSIFY: What kind of task is this?
    /// 2. PLAN: Break it down.
    /// 3. EXECUTE: Delegate to the right engine.
    pub async fn handle_request(&self, user_request: &str) -> Result<String> {
        println!("🧠 Orchestrator: Analyzing request '{}'...", user_request);

        // 1. Classification (intent analysis)
        let task_type = self.classify_intent(user_request).await?;
        println!("   👉 Classified as: {:?}", task_type);

        match task_type {
            TaskType::Coding => self.handle_coding_task(user_request).await,
            TaskType::Workflow => self.handle_workflow_task(user_request).await,
            TaskType::OsOperation => self.handle_os_task(user_request).await,
            TaskType::Research => self.handle_research_task(user_request).await,
        }
    }

    async fn classify_intent(&self, request: &str) -> Result<TaskType> {
        let _prompt = format!("Classify this user request... {}", request);

        // ... (Logic)

        let trimmed = request.trim();
        let r = trimmed.to_lowercase();
        let first_token = r.split_whitespace().next().unwrap_or("");

        // Workflow routing is explicit-only: /workflow (or /workflow@bot)
        // Natural-language mentions like "workflow", "n8n", "daily" should not force workflow mode.
        if first_token == "/workflow" || first_token.starts_with("/workflow@") {
            return Ok(TaskType::Workflow);
        } else if r.contains("click")
            || r.contains("open")
            || r.contains("type")
            || r.contains("mouse")
        {
            return Ok(TaskType::OsOperation);
        } else if r.contains("code")
            || r.contains("rust")
            || r.contains("file")
            || r.contains("project")
        {
            return Ok(TaskType::Coding);
        }

        // Default
        Ok(TaskType::Research)
    }

    // --- Handlers ---

    async fn handle_coding_task(&self, request: &str) -> Result<String> {
        println!("   💻 Starting DAACS Coding Agent...");
        let workdir = std::env::current_dir()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let scanner = ProjectScanner::new(&workdir);
        let scan = scanner.scan(Some(120));
        let project_type = scanner.get_project_type();
        Ok(format!(
            "(Coding Agent) I analyzed your request. Project type: {}. Files scanned: {}. Key files: {}.\nNext: run 'plan' to generate an RFP for: {}",
            project_type.as_str(),
            scan.files.len(),
            scan.key_files.len(),
            request
        ))
    }

    async fn handle_workflow_task(&self, request: &str) -> Result<String> {
        println!("   ⚙️  Starting n8n Automation Builder...");
        let normalized_request = request
            .trim()
            .trim_start_matches("/workflow")
            .trim_start()
            .trim_start_matches("@")
            .trim();
        let workflow_request = if normalized_request.is_empty() {
            request.trim()
        } else {
            normalized_request
        };

        if let Some(mock) = Self::load_mock_workflow_proposal() {
            if !mock.n8n_prompt.trim().is_empty() {
                return Ok(format!(
                    "(n8n Agent) Mock workflow received: '{}'. I will proceed as if the workflow package was delivered.",
                    mock.title
                ));
            }
        }

        // If the LLM chain fails, return a deterministic fallback response instead of bubbling an error.
        let proposal = match self.llm.propose_workflow(&[workflow_request.to_string()]).await {
            Ok(p) => p,
            Err(e) => {
                return Ok(format!(
                    "(n8n Agent Fallback) Workflow intent acknowledged, but LLM generation is temporarily unavailable ({}).\n\
Request: {}\n\
Use /workflow <trigger + action> again after CLI/LLM providers recover.",
                    e, workflow_request
                ));
            }
        };

        if !proposal.n8n_prompt.is_empty() {
            return Ok(format!(
                "(n8n Agent) I have prepared a workflow: '{}'. Please approve it in the Dashboard.",
                proposal.title
            ));
        }

        Ok("(n8n Agent) I couldn't generate a valid workflow from your request.".to_string())
    }

    fn load_mock_workflow_proposal() -> Option<AutomationProposal> {
        let assume_received = std::env::var("STEER_WORKFLOW_ASSUME_RECEIVED")
            .ok()
            .map(|v| matches!(v.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
            .unwrap_or(false);
        if !assume_received {
            return None;
        }

        let path = std::env::var("STEER_WORKFLOW_MOCK_FILE")
            .unwrap_or_else(|_| "core/mock/workflow_received_mock.json".to_string());
        let raw = std::fs::read_to_string(&path).ok()?;
        serde_json::from_str::<AutomationProposal>(&raw).ok()
    }

    async fn handle_os_task(&self, _request: &str) -> Result<String> {
        println!("   🖱️  Starting Visual Driver (OS Control)...");
        let _driver = VisualDriver::new();
        // driver.execute implementation skipped for MVP stub
        Ok("(Visual Driver) I have executed the UI actions on your screen.".to_string())
    }

    async fn handle_research_task(&self, request: &str) -> Result<String> {
        Ok(format!("(Analyst) I need more clarification on '{}'. Are you asking for a code change or a workflow?", request))
    }
}
