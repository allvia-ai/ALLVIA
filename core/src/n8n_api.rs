#![allow(dead_code)]
use anyhow::Result;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum N8nRuntime {
    Docker,
    Npx,
    Manual,
}

impl N8nRuntime {
    fn from_env() -> Self {
        let raw = std::env::var("STEER_N8N_RUNTIME")
            .unwrap_or_else(|_| "docker".to_string())
            .trim()
            .to_lowercase();
        match raw.as_str() {
            "npx" => {
                if parse_bool_env_with_default("STEER_N8N_ENABLE_NPX_RUNTIME", false) {
                    Self::Npx
                } else {
                    eprintln!(
                        "⚠️ STEER_N8N_RUNTIME=npx ignored: set STEER_N8N_ENABLE_NPX_RUNTIME=1 to opt in."
                    );
                    Self::Docker
                }
            }
            "manual" | "none" => Self::Manual,
            _ => Self::Docker,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Docker => "docker",
            Self::Npx => "npx",
            Self::Manual => "manual",
        }
    }
}

fn parse_bool_env_with_default(key: &str, default: bool) -> bool {
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

impl N8nApi {
    fn build_http_client() -> Client {
        let prefer_no_proxy =
            cfg!(test) || parse_bool_env_with_default("STEER_HTTP_NO_SYSTEM_PROXY", false);
        if prefer_no_proxy {
            if let Ok(client) = Client::builder().no_proxy().build() {
                return client;
            }
        }

        if let Ok(client) = std::panic::catch_unwind(Client::new) {
            return client;
        }

        eprintln!("⚠️ reqwest default client init panicked; falling back to no-proxy client");
        Client::builder()
            .no_proxy()
            .build()
            .unwrap_or_else(|_| Client::new())
    }

    pub fn new(base_url: &str, api_key: &str) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: api_key.to_string(),
            client: Self::build_http_client(),
        }
    }

    pub fn from_env() -> anyhow::Result<Self> {
        let base_url = std::env::var("N8N_API_URL")
            .unwrap_or_else(|_| "http://localhost:5678/api/v1".to_string());
        // Allow missing key only when CLI fallback is explicitly enabled.
        let api_key = std::env::var("N8N_API_KEY").unwrap_or_default();
        Ok(Self::new(&base_url, &api_key))
    }

    fn runtime_mode(&self) -> N8nRuntime {
        N8nRuntime::from_env()
    }

    fn auto_start_enabled(&self, runtime: N8nRuntime) -> bool {
        let default = matches!(runtime, N8nRuntime::Docker);
        parse_bool_env_with_default("STEER_N8N_AUTO_START", default)
    }

    fn cli_fallback_enabled(&self, runtime: N8nRuntime) -> bool {
        let default = matches!(runtime, N8nRuntime::Npx);
        parse_bool_env_with_default("STEER_N8N_ALLOW_CLI_FALLBACK", default)
    }

    fn local_target(&self) -> bool {
        self.base_url.contains("localhost") || self.base_url.contains("127.0.0.1")
    }

    fn health_urls(&self) -> (String, String) {
        let root_url = self
            .base_url
            .replace("localhost", "127.0.0.1")
            .replace("/api/v1", "/");
        let root_trimmed = root_url.trim_end_matches('/');
        let healthz = format!("{}/healthz", root_trimmed);
        (healthz, format!("{}/", root_trimmed))
    }

    async fn is_server_reachable(&self, healthz: &str, root: &str) -> bool {
        let timeout = std::time::Duration::from_secs(2);
        let ok_health = self
            .client
            .get(healthz)
            .timeout(timeout)
            .send()
            .await
            .map(|resp| resp.status().is_success())
            .unwrap_or(false);
        if ok_health {
            return true;
        }

        self.client
            .get(root)
            .timeout(timeout)
            .send()
            .await
            .map(|resp| resp.status().is_success())
            .unwrap_or(false)
    }

    fn resolve_compose_file() -> Option<PathBuf> {
        if let Ok(raw) = std::env::var("STEER_N8N_COMPOSE_FILE") {
            let trimmed = raw.trim();
            if !trimmed.is_empty() {
                return Some(PathBuf::from(trimmed));
            }
        }

        let cwd = std::env::current_dir().ok()?;
        let candidates = [
            cwd.join("docker-compose.yml"),
            cwd.join("../docker-compose.yml"),
        ];
        candidates.into_iter().find(|p| p.is_file())
    }

    fn run_docker_compose(compose_file: &Path, args: &[&str]) -> Result<()> {
        let compose_file_str = compose_file.display().to_string();
        let run_primary = std::process::Command::new("docker")
            .arg("compose")
            .arg("-f")
            .arg(&compose_file_str)
            .args(args)
            .output();

        match run_primary {
            Ok(out) if out.status.success() => return Ok(()),
            Ok(out) => {
                let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
                let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
                let detail = if !stderr.is_empty() { stderr } else { stdout };
                eprintln!(
                    "⚠️ docker compose failed, trying docker-compose fallback: {}",
                    detail
                );
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(anyhow::anyhow!("failed to run docker compose: {}", e)),
        }

        let legacy = std::process::Command::new("docker-compose")
            .arg("-f")
            .arg(&compose_file_str)
            .args(args)
            .output();
        match legacy {
            Ok(out) if out.status.success() => Ok(()),
            Ok(out) => {
                let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
                let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
                let detail = if !stderr.is_empty() { stderr } else { stdout };
                Err(anyhow::anyhow!("docker-compose failed: {}", detail))
            }
            Err(e) => Err(anyhow::anyhow!(
                "docker compose unavailable (tried docker compose + docker-compose): {}",
                e
            )),
        }
    }

    fn start_with_docker(&self) -> Result<()> {
        let compose_file = Self::resolve_compose_file().ok_or_else(|| {
            anyhow::anyhow!(
                "docker-compose.yml not found. Place it at repo root or set STEER_N8N_COMPOSE_FILE"
            )
        })?;
        println!(
            "🐳 Starting n8n via Docker Compose (runtime=docker, file={})...",
            compose_file.display()
        );
        Self::run_docker_compose(&compose_file, &["up", "-d", "n8n"])
    }

    fn start_with_npx(&self) -> Result<()> {
        if !parse_bool_env_with_default("STEER_N8N_ENABLE_NPX_RUNTIME", false) {
            return Err(anyhow::anyhow!(
                "npx runtime is disabled by default. Set STEER_N8N_ENABLE_NPX_RUNTIME=1 to enable."
            ));
        }
        println!("⚠️  Starting n8n with npx fallback runtime...");
        use std::process::{Command, Stdio};

        let mut args = vec!["-y", "n8n", "start"];
        if crate::env_flag("STEER_N8N_USE_TUNNEL") {
            args.push("--tunnel");
        }

        Command::new("npx")
            .args(args)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| anyhow::anyhow!("Failed to auto-start n8n with npx: {}", e))?;
        Ok(())
    }

    fn start_runtime(&self, runtime: N8nRuntime) -> Result<()> {
        match runtime {
            N8nRuntime::Docker => self.start_with_docker(),
            N8nRuntime::Npx => self.start_with_npx(),
            N8nRuntime::Manual => Err(anyhow::anyhow!(
                "runtime=manual: start n8n yourself and set N8N_API_URL/N8N_API_KEY"
            )),
        }
    }

    pub async fn restart_server(&self) -> Result<()> {
        if crate::env_flag("STEER_N8N_MOCK") {
            println!("🧪 STEER_N8N_MOCK=1: skipping n8n restart");
            return Ok(());
        }

        let runtime = self.runtime_mode();
        if !self.local_target() && !matches!(runtime, N8nRuntime::Manual) {
            return Err(anyhow::anyhow!(
                "runtime={} cannot restart remote n8n target ({})",
                runtime.as_str(),
                self.base_url
            ));
        }

        match runtime {
            N8nRuntime::Docker => {
                let compose_file = Self::resolve_compose_file().ok_or_else(|| {
                    anyhow::anyhow!(
                        "docker-compose.yml not found. Place it at repo root or set STEER_N8N_COMPOSE_FILE"
                    )
                })?;
                println!(
                    "🐳 Restarting n8n via Docker Compose (file={})...",
                    compose_file.display()
                );
                if let Err(restart_err) =
                    Self::run_docker_compose(&compose_file, &["restart", "n8n"])
                {
                    eprintln!(
                        "⚠️ Docker restart failed ({}). Trying `up -d n8n`...",
                        restart_err
                    );
                    Self::run_docker_compose(&compose_file, &["up", "-d", "n8n"])?;
                }
            }
            N8nRuntime::Npx => {
                let _ = std::process::Command::new("pkill")
                    .arg("-f")
                    .arg("n8n")
                    .output();
                self.start_with_npx()?;
            }
            N8nRuntime::Manual => {
                return Err(anyhow::anyhow!(
                    "runtime=manual: cannot restart automatically"
                ));
            }
        }

        let (healthz, root) = self.health_urls();
        for _ in 0..30 {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            if self.is_server_reachable(&healthz, &root).await {
                println!("✅ n8n restart completed.");
                return Ok(());
            }
        }
        Err(anyhow::anyhow!("Timed out waiting for n8n after restart"))
    }

    /// Check if n8n is running, and start it if not
    pub async fn ensure_server_running(&self) -> Result<()> {
        if crate::env_flag("STEER_N8N_MOCK") {
            println!("🧪 STEER_N8N_MOCK=1: skipping n8n health/start checks");
            return Ok(());
        }

        let runtime = self.runtime_mode();
        let (healthz, root) = self.health_urls();
        println!(
            "🔎 Checking n8n health (runtime={}, healthz={})...",
            runtime.as_str(),
            healthz
        );

        if self.is_server_reachable(&healthz, &root).await {
            println!("✅ n8n server is running.");
            return Ok(());
        }

        if !self.auto_start_enabled(runtime) {
            return Err(anyhow::anyhow!(
                "n8n server is not reachable at {}. Enable auto-start with STEER_N8N_AUTO_START=1 or run n8n manually.",
                healthz
            ));
        }

        if !self.local_target() && !matches!(runtime, N8nRuntime::Manual) {
            return Err(anyhow::anyhow!(
                "runtime={} cannot auto-start remote n8n target ({})",
                runtime.as_str(),
                self.base_url
            ));
        }

        println!("⚠️  n8n server NOT found. Starting automatically...");
        self.start_runtime(runtime)?;

        println!("⏳ Waiting for n8n to initialize (this may take 60s)...");
        for i in 0..30 {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            if self.is_server_reachable(&healthz, &root).await {
                println!("🚀 n8n server started successfully!");
                if !self.api_key.is_empty() && self.api_key != "placeholder" {
                    self.verify_auth().await?;
                }
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
        } else if resp.status() == reqwest::StatusCode::UNAUTHORIZED
            || resp.status() == reqwest::StatusCode::FORBIDDEN
        {
            Err(anyhow::anyhow!(
                "❌ n8n API Key is INVALID ({}). Check core/.env or secrets.",
                resp.status()
            ))
        } else {
            Err(anyhow::anyhow!(
                "❌ n8n auth verification failed with status {}",
                resp.status()
            ))
        }
    }

    /// List available credentials
    pub async fn list_credentials(&self) -> Result<Vec<Credential>> {
        if crate::env_flag("STEER_N8N_MOCK") {
            return Ok(Vec::new());
        }

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

        let runtime = self.runtime_mode();
        let allow_cli_fallback = self.cli_fallback_enabled(runtime);

        // 3. Try API
        if !self.api_key.is_empty() && self.api_key != "placeholder" {
            println!("🌐 Attempting to create workflow via API...");
            match self.create_workflow_api(name, &normalized, active).await {
                Ok(id) => return Ok(id),
                Err(e) => {
                    if !allow_cli_fallback {
                        return Err(anyhow::anyhow!(
                            "n8n API creation failed and CLI fallback is disabled (runtime={}): {}",
                            runtime.as_str(),
                            e
                        ));
                    }
                    println!("⚠️ API creation failed ({}). Falling back to CLI...", e);
                }
            }
        } else {
            if !allow_cli_fallback {
                return Err(anyhow::anyhow!(
                    "N8N_API_KEY is not set and CLI fallback is disabled (runtime={}). Set N8N_API_KEY or enable STEER_N8N_ALLOW_CLI_FALLBACK=1",
                    runtime.as_str()
                ));
            }
            println!("ℹ️ No API Key configured. Using CLI fallback mode.");
        }

        // 4. Fallback to CLI (Strict Local Check)
        if !self.base_url.contains("localhost") && !self.base_url.contains("127.0.0.1") {
            return Err(anyhow::anyhow!(
                "❌ CLI Fallback aborted: n8n is remote ({}). CLI only works for local instances.",
                self.base_url
            ));
        }

        let import_marker = format!("steer-import-{}", uuid::Uuid::new_v4());

        // 5. Run CLI Import
        if let Err(e) = self
            .create_workflow_cli(name, &normalized, active, &import_marker)
            .await
        {
            return Err(anyhow::anyhow!("❌ CLI Fallback Failed: {}", e));
        }

        // 6. Retrieve ID via CLI export (no direct SQLite coupling).
        self.retrieve_workflow_id_via_cli_export(name, &import_marker)
            .await
    }

    async fn create_workflow_api(
        &self,
        name: &str,
        workflow_json: &Value,
        active: bool,
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
        if active {
            if let Err(activate_err) = self.activate_workflow(&id).await {
                // Some n8n versions expose activation differently; attempt generic update fallback.
                self.update_workflow_active(&id, true).await.map_err(|patch_err| {
                    anyhow::anyhow!(
                        "workflow created (id={}) but activation failed: {} | fallback update failed: {}",
                        id,
                        activate_err,
                        patch_err
                    )
                })?;
            }
        }
        Ok(id)
    }

    async fn update_workflow_active(&self, id: &str, active: bool) -> Result<()> {
        let url = format!("{}/workflows/{}", self.base_url, id);
        let resp = self
            .client
            .patch(&url)
            .header("X-N8N-API-KEY", &self.api_key)
            .json(&json!({ "active": active }))
            .send()
            .await?;
        if !resp.status().is_success() {
            let error_text = resp.text().await?;
            return Err(anyhow::anyhow!("n8n workflow update error: {}", error_text));
        }
        Ok(())
    }

    async fn create_workflow_cli(
        &self,
        name: &str,
        workflow_json: &Value,
        active: bool,
        import_marker: &str,
    ) -> Result<String> {
        // Prepare JSON file
        let mut final_json = workflow_json.clone();
        final_json["name"] = json!(name);
        final_json["active"] = json!(active);
        let mut settings = final_json
            .get("settings")
            .cloned()
            .unwrap_or_else(|| json!({}));
        if !settings.is_object() {
            settings = json!({});
        }
        settings["steerImportId"] = json!(import_marker);
        final_json["settings"] = settings;

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

        println!("✅ CLI Import successful!");

        Ok("cli-imported".to_string())
    }

    fn workflow_id_from_value(value: Option<&Value>) -> Option<String> {
        match value {
            Some(Value::String(s)) if !s.trim().is_empty() => Some(s.trim().to_string()),
            Some(Value::Number(n)) => Some(n.to_string()),
            _ => None,
        }
    }

    fn export_items_from_value(value: &Value) -> Vec<Value> {
        match value {
            Value::Array(items) => items.clone(),
            Value::Object(map) => {
                if let Some(Value::Array(items)) = map.get("data") {
                    items.clone()
                } else if map.contains_key("nodes") || map.contains_key("connections") {
                    vec![value.clone()]
                } else {
                    Vec::new()
                }
            }
            _ => Vec::new(),
        }
    }

    async fn retrieve_workflow_id_via_cli_export(
        &self,
        name: &str,
        import_marker: &str,
    ) -> Result<String> {
        let path = format!("/tmp/n8n_export_{}.json", uuid::Uuid::new_v4());
        let output = tokio::process::Command::new("npx")
            .args(["-y", "n8n", "export:workflow", "--all", "--output", &path])
            .output()
            .await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let detail = if !stderr.is_empty() { stderr } else { stdout };
            return Err(anyhow::anyhow!(
                "CLI workflow id lookup failed after import: {}",
                detail
            ));
        }

        let raw = tokio::fs::read_to_string(&path).await?;
        if let Err(e) = tokio::fs::remove_file(&path).await {
            eprintln!("⚠️ Failed to clean up export file {}: {}", path, e);
        }

        let parsed: Value = serde_json::from_str(&raw).map_err(|e| {
            anyhow::anyhow!(
                "Failed to parse exported workflows while resolving imported workflow id: {}",
                e
            )
        })?;
        let items = Self::export_items_from_value(&parsed);
        if items.is_empty() {
            return Err(anyhow::anyhow!(
                "No workflows found in n8n CLI export while resolving imported workflow id"
            ));
        }

        let mut marker_match: Option<String> = None;
        let mut name_matches: Vec<String> = Vec::new();
        for item in items {
            let Some(id) = Self::workflow_id_from_value(item.get("id")) else {
                continue;
            };
            let settings_text = item
                .get("settings")
                .map(|v| v.to_string())
                .unwrap_or_default();
            if !import_marker.trim().is_empty() && settings_text.contains(import_marker) {
                marker_match = Some(id);
                break;
            }

            if item
                .get("name")
                .and_then(|v| v.as_str())
                .map(|wf_name| wf_name == name)
                .unwrap_or(false)
            {
                name_matches.push(id);
            }
        }

        if let Some(id) = marker_match {
            return Ok(id);
        }
        let allow_name_fallback =
            parse_bool_env_with_default("STEER_N8N_ALLOW_NAME_ID_FALLBACK", false);
        if allow_name_fallback {
            if let Some(id) = name_matches.first() {
                if name_matches.len() > 1
                    && !parse_bool_env_with_default(
                        "STEER_N8N_ALLOW_AMBIGUOUS_NAME_ID_FALLBACK",
                        false,
                    )
                {
                    return Err(anyhow::anyhow!(
                        "Ambiguous name-based fallback for '{}': {} matches. \
Set STEER_N8N_ALLOW_AMBIGUOUS_NAME_ID_FALLBACK=1 only for controlled test environments.",
                        name,
                        name_matches.len()
                    ));
                }
                if name_matches.len() > 1 {
                    eprintln!(
                        "⚠️ Ambiguous name fallback explicitly allowed for '{}'; using first exported id={}",
                        name, id
                    );
                }
                return Ok(id.clone());
            }
        }

        if !name_matches.is_empty() {
            return Err(anyhow::anyhow!(
                "Import marker not found in exported workflows; {} name match(es) exist for '{}'. \
Set STEER_N8N_ALLOW_NAME_ID_FALLBACK=1 to allow name-based fallback.",
                name_matches.len(),
                name
            ));
        }
        Err(anyhow::anyhow!(
            "Could not resolve imported workflow id via n8n CLI export"
        ))
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
    use serial_test::serial;

    #[tokio::test]
    #[serial]
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
