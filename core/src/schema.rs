use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "action", content = "payload")]
#[allow(dead_code)]
pub enum AgentAction {
    // Observe
    #[serde(rename = "ui.snapshot")]
    UiSnapshot { scope: Option<String> },
    #[serde(rename = "ui.find")]
    UiFind { query: String },

    // Act
    #[serde(rename = "ui.click")]
    UiClick {
        element_id: String,
        double_click: bool,
    },
    #[serde(rename = "ui.click_text")]
    UiClickText { text: String },
    #[serde(rename = "ui.type")]
    UiType { text: String },
    #[serde(rename = "keyboard.type")]
    KeyboardType { text: String, submit: bool },

    // System
    #[serde(rename = "system.open")]
    SystemOpen { app: String },
    #[serde(rename = "system.search")]
    SystemSearch { query: String },
    #[serde(rename = "system.terminate")]
    Terminate,
    #[serde(rename = "debug.fake_log")]
    DebugFakeLog,
    #[serde(rename = "shell.exec")]
    ShellExecution { command: String },
}

// --- Data Collection Schema (Matches Python models.py) ---

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PrivacyContext {
    #[serde(default)]
    pub pii_types: Vec<String>,
    #[serde(default)]
    pub hash_method: String,
    #[serde(default)]
    pub is_masked: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ResourceContext {
    #[serde(rename = "type")]
    pub resource_type: String,
    pub id: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct EventEnvelope {
    pub schema_version: String,
    pub event_id: String,
    pub ts: String,     // ISO 8601
    pub source: String, // e.g., "macos_monitor"
    pub app: String,
    pub event_type: String,
    pub priority: String, // P0, P1, P2

    #[serde(default)]
    pub resource: Option<ResourceContext>,

    pub payload: serde_json::Value,

    #[serde(default)]
    pub privacy: Option<PrivacyContext>,

    pub pid: Option<u32>,
    pub window_id: Option<String>,

    // [Context Enrichment]
    pub window_title: Option<String>,
    pub browser_url: Option<String>,

    #[serde(default)]
    pub raw: Option<serde_json::Value>,
}
