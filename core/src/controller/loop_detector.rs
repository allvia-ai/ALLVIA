use serde_json::Value;

/// Action Loop Detector
/// Identifies repetitive actions to prevent agent from getting stuck.
pub struct LoopDetector;

impl LoopDetector {
    /// Detect if the same normalized action has been repeated 4+ times consecutively.
    pub fn detect_action_loop(history: &[String], current_action: &str) -> bool {
        if history.len() < 3 {
            return false;
        }

        let current_key = Self::extract_action_key(current_action);
        if current_key == "unknown"
            || current_key == "action:report"
            || current_key == "action:done"
        {
            return false;
        }

        history
            .iter()
            .rev()
            .take(3)
            .all(|entry| Self::extract_action_key(entry) == current_key)
    }

    fn normalize_modifiers(v: Option<&Value>) -> String {
        let mut mods: Vec<String> = v
            .and_then(|m| m.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|x| x.as_str())
                    .map(|s| s.trim().to_lowercase())
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        mods.sort();
        mods.dedup();
        mods.join("+")
    }

    /// Extract a normalized key from action JSON for loop comparison.
    fn extract_action_key(action_str: &str) -> String {
        let parsed = serde_json::from_str::<Value>(action_str);
        if let Ok(v) = parsed {
            if let Some(obj) = v.as_object() {
                let action = obj
                    .get("action")
                    .and_then(|x| x.as_str())
                    .unwrap_or("unknown")
                    .trim()
                    .to_lowercase();

                match action.as_str() {
                    "shortcut" | "key" => {
                        let key = obj
                            .get("key")
                            .and_then(|x| x.as_str())
                            .unwrap_or("")
                            .trim()
                            .to_lowercase();
                        let mods = Self::normalize_modifiers(obj.get("modifiers"));
                        if !key.is_empty() {
                            if mods.is_empty() {
                                return format!("action:{}:{}", action, key);
                            }
                            return format!("action:{}:{}+{}", action, mods, key);
                        }
                    }
                    "open_app" => {
                        let name = obj
                            .get("name")
                            .or_else(|| obj.get("app"))
                            .and_then(|x| x.as_str())
                            .unwrap_or("")
                            .trim()
                            .to_lowercase();
                        if !name.is_empty() {
                            return format!("action:open_app:{}", name);
                        }
                    }
                    "switch_app" | "activate_app" => {
                        let app = obj
                            .get("app")
                            .or_else(|| obj.get("name"))
                            .and_then(|x| x.as_str())
                            .unwrap_or("")
                            .trim()
                            .to_lowercase();
                        if !app.is_empty() {
                            return format!("action:switch_app:{}", app);
                        }
                    }
                    "type" => {
                        let text = obj
                            .get("text")
                            .and_then(|x| x.as_str())
                            .unwrap_or("")
                            .trim()
                            .to_lowercase();
                        if !text.is_empty() {
                            let preview: String = text.chars().take(24).collect();
                            return format!("action:type:{}", preview);
                        }
                    }
                    "click_visual" => {
                        let desc = obj
                            .get("description")
                            .or_else(|| obj.get("text"))
                            .and_then(|x| x.as_str())
                            .unwrap_or("")
                            .trim()
                            .to_lowercase();
                        if !desc.is_empty() {
                            return format!("action:click_visual:{}", desc);
                        }
                    }
                    other => {
                        return format!("action:{}", other);
                    }
                }
            }
        }

        let compact = action_str.trim().replace(char::is_whitespace, "");
        if compact.is_empty() {
            return "unknown".to_string();
        }
        compact.chars().take(96).collect()
    }
}
