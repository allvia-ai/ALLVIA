//! MCP - Model Context Protocol Client
//!
//! Standard protocol for LLM agents to interact with external services.
//! Inspired by Anthropic's MCP specification.
//!
//! Supports:
//! - Service discovery
//! - Tool invocation
//! - Resource access (files, databases)
//! - Prompt templates

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};

// =====================================================
// MCP TYPES
// =====================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServer {
    #[serde(default)]
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpTool {
    pub name: String,
    pub description: String,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpResource {
    pub uri: String,
    pub name: String,
    #[serde(rename = "mimeType")]
    pub mime_type: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpPrompt {
    pub name: String,
    pub description: Option<String>,
    pub arguments: Option<Vec<McpPromptArg>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpPromptArg {
    pub name: String,
    pub description: Option<String>,
    pub required: Option<bool>,
}

// =====================================================
// JSON-RPC MESSAGE TYPES
// =====================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    id: u64,
    method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct JsonRpcResponse {
    jsonrpc: String,
    id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct JsonRpcError {
    code: i32,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
}

// =====================================================
// MCP CLIENT
// =====================================================

pub struct McpClient {
    server: McpServer,
    process: Option<Child>,
    request_id: u64,
    pub tools: Vec<McpTool>,
    pub resources: Vec<McpResource>,
    pub prompts: Vec<McpPrompt>,
}

impl McpClient {
    /// Create a new MCP client for a server
    pub fn new(server: McpServer) -> Self {
        Self {
            server,
            process: None,
            request_id: 0,
            tools: Vec::new(),
            resources: Vec::new(),
            prompts: Vec::new(),
        }
    }

    /// Start the MCP server process
    pub fn connect(&mut self) -> Result<()> {
        println!("🔌 [MCP] Connecting to: {}", self.server.name);

        let mut cmd = Command::new(&self.server.command);
        cmd.args(&self.server.args);

        for (key, value) in &self.server.env {
            cmd.env(key, value);
        }

        cmd.stdin(Stdio::piped());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        let child = cmd.spawn().context(format!(
            "Failed to start MCP server: {}",
            self.server.command
        ))?;

        self.process = Some(child);

        // Initialize the connection
        self.initialize()?;

        // Discover capabilities
        self.discover()?;

        println!(
            "✅ [MCP] Connected: {} tools, {} resources",
            self.tools.len(),
            self.resources.len()
        );

        Ok(())
    }

    /// Send JSON-RPC request and get response
    fn send_request(&mut self, method: &str, params: Option<Value>) -> Result<Value> {
        let child = self
            .process
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("MCP server not connected"))?;

        self.request_id += 1;
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: self.request_id,
            method: method.to_string(),
            params,
        };

        let request_json = serde_json::to_string(&request)?;

        // Write request to stdin
        if let Some(stdin) = child.stdin.as_mut() {
            writeln!(stdin, "{}", request_json)?;
            stdin.flush()?;
        }

        // Read response from stdout
        if let Some(stdout) = child.stdout.as_mut() {
            let mut reader = BufReader::new(stdout);
            let mut line = String::new();
            reader.read_line(&mut line)?;

            let response: JsonRpcResponse = serde_json::from_str(&line)?;

            if let Some(error) = response.error {
                return Err(anyhow::anyhow!(
                    "MCP error {}: {}",
                    error.code,
                    error.message
                ));
            }

            Ok(response.result.unwrap_or(json!(null)))
        } else {
            Err(anyhow::anyhow!("No stdout available"))
        }
    }

    /// Initialize the MCP connection
    fn initialize(&mut self) -> Result<()> {
        let params = json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {
                "tools": {},
                "resources": {},
                "prompts": {}
            },
            "clientInfo": {
                "name": "steer-agent",
                "version": "0.1.0"
            }
        });

        let _result = self.send_request("initialize", Some(params))?;

        // Send initialized notification
        let _ = self.send_request("notifications/initialized", None);

        Ok(())
    }

    /// Discover server capabilities
    fn discover(&mut self) -> Result<()> {
        // List tools
        if let Ok(result) = self.send_request("tools/list", None) {
            if let Some(tools) = result["tools"].as_array() {
                self.tools = tools
                    .iter()
                    .filter_map(|t| serde_json::from_value(t.clone()).ok())
                    .collect();
            }
        }

        // List resources
        if let Ok(result) = self.send_request("resources/list", None) {
            if let Some(resources) = result["resources"].as_array() {
                self.resources = resources
                    .iter()
                    .filter_map(|r| serde_json::from_value(r.clone()).ok())
                    .collect();
            }
        }

        // List prompts
        if let Ok(result) = self.send_request("prompts/list", None) {
            if let Some(prompts) = result["prompts"].as_array() {
                self.prompts = prompts
                    .iter()
                    .filter_map(|p| serde_json::from_value(p.clone()).ok())
                    .collect();
            }
        }

        Ok(())
    }

    /// Call a tool on the MCP server
    pub fn call_tool(&mut self, name: &str, arguments: Value) -> Result<Value> {
        println!("🔧 [MCP] Calling tool: {} with {:?}", name, arguments);

        let params = json!({
            "name": name,
            "arguments": arguments
        });

        let result = self.send_request("tools/call", Some(params))?;

        Ok(result)
    }

    /// Read a resource
    pub fn read_resource(&mut self, uri: &str) -> Result<String> {
        println!("📖 [MCP] Reading resource: {}", uri);

        let params = json!({ "uri": uri });
        let result = self.send_request("resources/read", Some(params))?;

        // Extract content from result
        if let Some(contents) = result["contents"].as_array() {
            if let Some(first) = contents.first() {
                if let Some(text) = first["text"].as_str() {
                    return Ok(text.to_string());
                }
            }
        }

        Ok(result.to_string())
    }

    /// Get a prompt template
    pub fn get_prompt(&mut self, name: &str, arguments: Option<Value>) -> Result<String> {
        let params = json!({
            "name": name,
            "arguments": arguments.unwrap_or(json!({}))
        });

        let result = self.send_request("prompts/get", Some(params))?;

        // Extract messages
        if let Some(messages) = result["messages"].as_array() {
            let texts: Vec<String> = messages
                .iter()
                .filter_map(|m| m["content"]["text"].as_str().map(|s| s.to_string()))
                .collect();
            return Ok(texts.join("\n"));
        }

        Ok(result.to_string())
    }

    /// Disconnect from the server
    pub fn disconnect(&mut self) {
        if let Some(mut child) = self.process.take() {
            let _ = child.kill();
            let _ = child.wait();
            println!("🔌 [MCP] Disconnected: {}", self.server.name);
        }
    }
}

impl Drop for McpClient {
    fn drop(&mut self) {
        self.disconnect();
    }
}

// =====================================================
// MCP REGISTRY - Manages multiple servers
// =====================================================

pub struct McpRegistry {
    clients: HashMap<String, McpClient>,
    config_path: std::path::PathBuf,
}

impl McpRegistry {
    /// Create a new registry
    pub fn new() -> Self {
        let config_path = dirs::home_dir()
            .unwrap_or_default()
            .join(".steer")
            .join("mcp_servers.json");

        Self {
            clients: HashMap::new(),
            config_path,
        }
    }

    /// Load servers from config file
    pub fn load_config(&mut self) -> Result<()> {
        if !self.config_path.exists() {
            // Create default config
            self.create_default_config()?;
        }

        let content = std::fs::read_to_string(&self.config_path)?;
        let servers: HashMap<String, McpServer> = serde_json::from_str(&content)?;

        for (name, mut server) in servers {
            server.name = name.clone();
            if server.enabled {
                let mut client = McpClient::new(server);
                if client.connect().is_ok() {
                    self.clients.insert(name, client);
                }
            }
        }

        Ok(())
    }

    /// Create default config file
    fn create_default_config(&self) -> Result<()> {
        let default = json!({
            "filesystem": {
                "command": "npx",
                "args": ["-y", "@modelcontextprotocol/server-filesystem", "/Users"],
                "env": {},
                "enabled": false
            },
            "memory": {
                "command": "npx",
                "args": ["-y", "@modelcontextprotocol/server-memory"],
                "env": {},
                "enabled": false
            },
            "brave-search": {
                "command": "npx",
                "args": ["-y", "@anthropic-ai/claude-mcp-server-brave"],
                "env": {
                    "BRAVE_API_KEY": ""
                },
                "enabled": false
            },
            "google-calendar": {
                "command": "npx",
                "args": ["-y", "@anthropic-ai/claude-mcp-server-google-calendar"],
                "env": {},
                "enabled": false
            },
            "slack": {
                "command": "npx",
                "args": ["-y", "@anthropic-ai/claude-mcp-server-slack"],
                "env": {
                    "SLACK_BOT_TOKEN": ""
                },
                "enabled": false
            }
        });

        if let Some(parent) = self.config_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        std::fs::write(&self.config_path, serde_json::to_string_pretty(&default)?)?;
        println!(
            "📝 [MCP] Created default config: {}",
            self.config_path.display()
        );

        Ok(())
    }

    /// Get a client by name
    pub fn get(&mut self, name: &str) -> Option<&mut McpClient> {
        self.clients.get_mut(name)
    }

    /// List all available tools across all servers
    pub fn list_all_tools(&self) -> Vec<(String, McpTool)> {
        self.clients
            .iter()
            .flat_map(|(server_name, client)| {
                client
                    .tools
                    .iter()
                    .map(move |tool| (server_name.clone(), tool.clone()))
            })
            .collect()
    }

    /// Call a tool on the appropriate server
    pub fn call_tool(&mut self, server: &str, tool: &str, args: Value) -> Result<Value> {
        let client = self
            .clients
            .get_mut(server)
            .ok_or_else(|| anyhow::anyhow!("MCP server '{}' not found", server))?;

        client.call_tool(tool, args)
    }

    /// Add and connect a server dynamically
    pub fn add_server(&mut self, server: McpServer) -> Result<()> {
        let name = server.name.clone();
        let mut client = McpClient::new(server);
        client.connect()?;
        self.clients.insert(name, client);
        Ok(())
    }
}

// =====================================================
// GLOBAL SINGLETON
// =====================================================

lazy_static::lazy_static! {
    static ref MCP_REGISTRY: std::sync::Mutex<Option<McpRegistry>> = std::sync::Mutex::new(None);
}

/// Initialize the global MCP registry
pub fn init_mcp() -> Result<()> {
    let mut guard = MCP_REGISTRY
        .lock()
        .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

    if guard.is_none() {
        let mut registry = McpRegistry::new();
        if let Err(e) = registry.load_config() {
            eprintln!("❌ [MCP] Failed to load config: {}", e);
            // Verify path
            eprintln!("    Path was: {}", registry.config_path.display());
        }
        *guard = Some(registry);
    }

    Ok(())
}

/// Get the global MCP registry
pub fn get_mcp_registry() -> Result<std::sync::MutexGuard<'static, Option<McpRegistry>>> {
    MCP_REGISTRY
        .lock()
        .map_err(|e| anyhow::anyhow!("Lock error: {}", e))
}

/// Call an MCP tool
pub fn call_mcp_tool(server: &str, tool: &str, args: Value) -> Result<Value> {
    let mut guard = get_mcp_registry()?;
    let registry = guard
        .as_mut()
        .ok_or_else(|| anyhow::anyhow!("MCP not initialized"))?;
    registry.call_tool(server, tool, args)
}

// =====================================================
// TESTS
// =====================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mcp_server_config() {
        let server = McpServer {
            name: "test".to_string(),
            command: "echo".to_string(),
            args: vec!["hello".to_string()],
            env: HashMap::new(),
            enabled: true,
        };

        let json = serde_json::to_string(&server).unwrap();
        assert!(json.contains("test"));
    }

    #[test]
    fn test_registry_default_config() {
        let registry = McpRegistry::new();
        // Just check it doesn't panic
        assert!(registry.clients.is_empty());
    }
}
