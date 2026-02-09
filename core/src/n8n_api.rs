#![allow(dead_code)]
use anyhow::Result;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

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

impl N8nApi {
    pub fn new(base_url: &str, api_key: &str) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: api_key.to_string(),
            client: Client::new(),
        }
    }

    pub fn from_env() -> anyhow::Result<Self> {
        let base_url = std::env::var("N8N_API_URL")
            .unwrap_or_else(|_| "http://localhost:5678/api/v1".to_string());
        // Allow missing key for CLI fallback. Users needing API should set it.
        let api_key = std::env::var("N8N_API_KEY").unwrap_or_default();
        Ok(Self::new(&base_url, &api_key))
    }

    /// Check if n8n is running, and start it if not
    pub async fn ensure_server_running(&self) -> Result<()> {
        // Use 127.0.0.1 to avoid macOS localhost DNS lag
        let health_url = self
            .base_url
            .replace("localhost", "127.0.0.1")
            .replace("/api/v1", "/");

        println!("🔎 Checking n8n health at {}...", health_url);

        // 1. Check if running
        if self
            .client
            .get(&health_url)
            .timeout(std::time::Duration::from_secs(2))
            .send()
            .await
            .is_ok()
        {
            println!("✅ n8n server is running.");
            // self.verify_auth().await?; // Disabled to prevent 503 if user hasn't set up API key yet
            return Ok(());
        }

        println!("⚠️  n8n server NOT found. Starting automatically...");

        // 2. Start n8n
        // Use npx -y n8n start --tunnel
        use std::process::{Command, Stdio};

        // We use spawn to run in background.
        // Note: This child process will be detached or killed when core exits depending on impl.
        // For MVP, we just spawn it.
        let _child = Command::new("npx")
            .args(["-y", "n8n", "start", "--tunnel"])
            .stdout(Stdio::null()) // Mute output or redirect to file
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| anyhow::anyhow!("Failed to auto-start n8n: {}", e))?;

        println!("⏳ Waiting for n8n to initialize (this may take 30s)...");

        // 3. Wait for it to become ready (Polling)
        for i in 0..30 {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            if self.client.get(&health_url).send().await.is_ok() {
                println!("🚀 n8n server started successfully!");
                // [NEW] Verify Auth immediately
                self.verify_auth().await?;
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
            .client
            .get(&url)
            .header("X-N8N-API-KEY", &self.api_key)
            .timeout(std::time::Duration::from_secs(3))
            .send()
            .await?;

        if resp.status().is_success() {
            println!("✅ API Key is valid.");
            Ok(())
        } else if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            Err(anyhow::anyhow!(
                "❌ n8n API Key is INVALID (401). Check core/.env or secrets."
            ))
        } else {
            // Other error, maybe server error, but key might be fine.
            // Be conservative: warn but don't block if it's just empty or 404?
            // 403/401 is the main concern.
            println!("⚠️ Auth check returned status: {}", resp.status());
            Ok(()) // Let it slide if it's not strictly 401, to avoid blocking valid but weird states
        }
    }

    /// List available credentials
    pub async fn list_credentials(&self) -> Result<Vec<Credential>> {
        let url = format!("{}/credentials", self.base_url);
        let resp = self
            .client
            .get(&url)
            .header("X-N8N-API-KEY", &self.api_key)
            .send()
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
        // 1. Validate JSON (repair to minimal workflow if empty)
        let mut normalized = workflow_json.clone();

        // Check if nodes are empty or missing
        let is_empty = normalized
            .get("nodes")
            .and_then(|n| n.as_array())
            .map(|arr| arr.is_empty())
            .unwrap_or(true);

        if is_empty {
            println!("⚠️ Workflow nodes empty. Falling back to minimal workflow template.");
            normalized = Self::build_minimal_workflow(name);
        }

        if crate::env_flag("STEER_N8N_MOCK") {
            let mock_id = format!("mock-wf-{}", chrono::Utc::now().timestamp_millis());
            println!(
                "🧪 STEER_N8N_MOCK=1: skipping n8n network/CLI calls and returning {}",
                mock_id
            );
            return Ok(mock_id);
        }

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

        // 3. Try API
        if !self.api_key.is_empty() && self.api_key != "placeholder" {
            println!("🌐 Attempting to create workflow via API...");
            match self.create_workflow_api(name, &normalized, active).await {
                Ok(id) => return Ok(id),
                Err(e) => println!("⚠️ API creation failed ({}). Falling back to CLI...", e),
            }
        } else {
            println!("ℹ️ No API Key configured. Using CLI mode.");
        }

        // 4. Fallback to CLI (Strict Local Check)
        if !self.base_url.contains("localhost") && !self.base_url.contains("127.0.0.1") {
            return Err(anyhow::anyhow!(
                "❌ CLI Fallback aborted: n8n is remote ({}). CLI only works for local instances.",
                self.base_url
            ));
        }

        // 5. Run CLI Import
        if let Err(e) = self.create_workflow_cli(name, &normalized, active).await {
            return Err(anyhow::anyhow!("❌ CLI Fallback Failed: {}", e));
        }

        // 6. Retrieve ID from DB (Crucial Step for Management)
        // Since CLI doesn't return ID, we query the DB by name
        self.retrieve_workflow_id_by_name(name)
    }

    async fn create_workflow_api(
        &self,
        name: &str,
        workflow_json: &Value,
        _active: bool,
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

        let resp = self
            .client
            .post(&url)
            .header("X-N8N-API-KEY", &self.api_key)
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let error_text = resp.text().await?;
            return Err(anyhow::anyhow!("n8n API Error: {}", error_text));
        }

        let resp_json: Value = resp.json().await?;
        let id = resp_json["id"].as_str().unwrap_or("unknown").to_string();
        Ok(id)
    }

    async fn create_workflow_cli(
        &self,
        name: &str,
        workflow_json: &Value,
        active: bool,
    ) -> Result<String> {
        // Prepare JSON file
        let mut final_json = workflow_json.clone();
        final_json["name"] = json!(name);
        final_json["active"] = json!(active);

        // Ensure nodes exist
        if final_json["nodes"].as_array().is_none_or(|n| n.is_empty()) {
            return Err(anyhow::anyhow!("Refusing to import empty workflow via CLI"));
        }

        let path = format!("/tmp/n8n_import_{}.json", uuid::Uuid::new_v4());
        tokio::fs::write(&path, serde_json::to_string(&final_json)?).await?;

        println!("📥 Importing workflow via CLI from {}...", path);

        let output = tokio::process::Command::new("npx")
            .args(["-y", "n8n", "import:workflow", "--input", &path])
            .output()
            .await?;

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
                "CLI Import failed (exit {}): {}",
                code,
                detail
            ));
        }

        println!("✅ CLI Import successful! Now fixing ownership...");

        if let Err(e) = self.fix_workflow_ownership() {
            println!("⚠️ Ownership fix failed: {}", e);
        }

        Ok("cli-imported".to_string())
    }

    // Helper to find ID after CLI import
    fn retrieve_workflow_id_by_name(&self, name: &str) -> Result<String> {
        use rusqlite::Connection;
        let home = std::env::var("HOME").unwrap_or("/".to_string());
        let db_path = format!("{}/.n8n/database.sqlite", home);

        let conn = Connection::open(db_path)?;
        let id: String = conn
            .query_row(
                "SELECT id FROM workflow_entity WHERE name = ?1 ORDER BY updatedAt DESC LIMIT 1",
                [name],
                |row| row.get(0),
            )
            .map_err(|_| anyhow::anyhow!("Could not find imported workflow ID in DB"))?;

        Ok(id)
    }

    fn build_minimal_workflow(name: &str) -> Value {
        json!({
            "name": name,
            "nodes": [
                {
                    "id": uuid::Uuid::new_v4().to_string(),
                    "name": "Manual Trigger",
                    "type": "n8n-nodes-base.manualTrigger",
                    "typeVersion": 1,
                    "position": [0, 0]
                },
                {
                    "id": uuid::Uuid::new_v4().to_string(),
                    "name": "Set",
                    "type": "n8n-nodes-base.set",
                    "typeVersion": 2,
                    "position": [260, 0],
                    "parameters": {
                        "keepOnlySet": true,
                        "values": {
                            "string": [
                                { "name": "note", "value": "Auto-generated minimal workflow" }
                            ]
                        }
                    }
                }
            ],
            "connections": {
                "Manual Trigger": {
                    "main": [[{ "node": "Set", "type": "main", "index": 0 }]]
                }
            },
            "settings": { "saveManualExecutions": true },
            "active": false
        })
    }

    /// HACK: Directly modify n8n SQLite DB to assign project ownership to imported workflows (n8n v1+)
    /// SECURITY: Requires N8N_ALLOW_DB_MODIFY=true to execute
    fn fix_workflow_ownership(&self) -> Result<()> {
        use rusqlite::Connection;

        // Security: Require explicit opt-in for direct DB modifications
        let allow_db_modify = std::env::var("N8N_ALLOW_DB_MODIFY")
            .map(|v| v == "true" || v == "1")
            .unwrap_or(false);

        if !allow_db_modify {
            println!(
                "⚠️ [n8n] Direct DB modification skipped. Set N8N_ALLOW_DB_MODIFY=true to enable."
            );
            return Ok(());
        }

        let home = std::env::var("HOME").unwrap_or("/".to_string());
        let db_path = format!("{}/.n8n/database.sqlite", home);

        if !std::path::Path::new(&db_path).exists() {
            return Ok(());
        }

        let conn = Connection::open(db_path)?;

        // 1. Get the user's personal Project ID
        // n8n v1: User -> ProjectRelation -> Project
        let project_id: String = conn
            .query_row(
                "SELECT projectId FROM project_relation 
             WHERE userId = (SELECT id FROM user ORDER BY createdAt ASC LIMIT 1) 
             LIMIT 1",
                [],
                |row| row.get(0),
            )
            .map_err(|_| anyhow::anyhow!("No project found for user"))?;

        if !project_id.is_empty() {
            println!("🔧 Linking workflows to Project ID: {}", project_id);

            // 2. Find workflows that have NO entry in shared_workflow
            let mut stmt = conn.prepare(
                "SELECT id FROM workflow_entity 
                 WHERE id NOT IN (SELECT workflowId FROM shared_workflow)",
            )?;

            let orphan_ids: Vec<String> = stmt
                .query_map([], |row| row.get(0))?
                .filter_map(Result::ok)
                .collect();

            let mut count = 0;
            for wid in orphan_ids {
                // 3. Insert into shared_workflow
                let res = conn.execute(
                    "INSERT INTO shared_workflow (workflowId, projectId, role, createdAt, updatedAt) 
                     VALUES (?1, ?2, 'workflow:owner', datetime('now'), datetime('now'))",
                    [&wid, &project_id],
                );
                if res.is_ok() {
                    count += 1;
                }
            }

            if count > 0 {
                println!("✨ Fixed visibility for {} workflows.", count);
            }
        }

        Ok(())
    }

    /// Activate a workflow
    pub async fn activate_workflow(&self, id: &str) -> Result<()> {
        let url = format!("{}/workflows/{}/activate", self.base_url, id);

        let resp = self
            .client
            .post(&url)
            .header("X-N8N-API-KEY", &self.api_key)
            .send()
            .await?;

        if !resp.status().is_success() {
            let error_text = resp.text().await?;
            return Err(anyhow::anyhow!("n8n activate error: {}", error_text));
        }
        Ok(())
    }

    /// Deactivate a workflow
    pub async fn deactivate_workflow(&self, id: &str) -> Result<()> {
        let url = format!("{}/workflows/{}/deactivate", self.base_url, id);

        let resp = self
            .client
            .post(&url)
            .header("X-N8N-API-KEY", &self.api_key)
            .send()
            .await?;

        if !resp.status().is_success() {
            let error_text = resp.text().await?;
            return Err(anyhow::anyhow!("n8n deactivate error: {}", error_text));
        }
        Ok(())
    }

    /// Get workflow status
    pub async fn get_workflow(&self, id: &str) -> Result<WorkflowStatus> {
        let url = format!("{}/workflows/{}", self.base_url, id);

        let resp = self
            .client
            .get(&url)
            .header("X-N8N-API-KEY", &self.api_key)
            .send()
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
            .client
            .get(&url)
            .header("X-N8N-API-KEY", &self.api_key)
            .send()
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

        let resp = self
            .client
            .post(&url)
            .header("X-N8N-API-KEY", &self.api_key)
            .json(&json!({}))
            .send()
            .await?;

        if !resp.status().is_success() {
            let error_text = resp.text().await?;
            return Err(anyhow::anyhow!("n8n execute error: {}", error_text));
        }

        let data: Value = resp.json().await?;
        Ok(ExecutionResult {
            id: data["id"].as_str().unwrap_or("").to_string(),
            finished: data["finished"].as_bool().unwrap_or(false),
            status: data["status"].as_str().unwrap_or("unknown").to_string(),
            started_at: data["startedAt"].as_str().unwrap_or("").to_string(),
            stopped_at: data["stoppedAt"].as_str().map(|s| s.to_string()),
        })
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
            .client
            .get(&url)
            .header("X-N8N-API-KEY", &self.api_key)
            .send()
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
            .client
            .delete(&url)
            .header("X-N8N-API-KEY", &self.api_key)
            .send()
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

    #[tokio::test]
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
