use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

pub struct ConfigManager {
    env_path: PathBuf,
}

impl Default for ConfigManager {
    fn default() -> Self {
        Self::new()
    }
}

impl ConfigManager {
    pub fn new() -> Self {
        // Safe location strategy
        // 1. Current Dir (Dev)
        // 2. Resource Dir (Prod - tricky in Tauri, usually read-only)
        // For MVP, we target the .env in CWD or a known config location.
        // If wrapped in typical Tauri bundle, writing to .env inside .app might fail.
        // Better to use a config file in ~/.steer_config?
        // But the legacy code uses .env.
        // We will try CWD .env first.

        let env_path = PathBuf::from(".env");
        ConfigManager { env_path }
    }

    pub fn get_all(&self) -> HashMap<String, String> {
        let mut map = HashMap::new();
        if let Ok(content) = fs::read_to_string(&self.env_path) {
            for line in content.lines() {
                if let Some((key, val)) = line.split_once('=') {
                    map.insert(key.trim().to_string(), val.trim().to_string());
                }
            }
        }
        // Also overlay real env vars?
        // No, we want the file content specifically for editing.
        map
    }

    pub fn update(&self, key: &str, value: &str) -> Result<(), String> {
        let mut lines = Vec::new();
        let mut found = false;

        let content = fs::read_to_string(&self.env_path).unwrap_or_default();

        for line in content.lines() {
            if line.starts_with(key) && line.contains('=') {
                lines.push(format!("{}={}", key, value));
                found = true;
            } else {
                lines.push(line.to_string());
            }
        }

        if !found {
            lines.push(format!("{}={}", key, value));
        }

        fs::write(&self.env_path, lines.join("\n")).map_err(|e| e.to_string())
    }
}
