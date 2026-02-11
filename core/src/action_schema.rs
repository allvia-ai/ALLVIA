use serde_json::{json, Map, Value};

#[derive(Debug, Clone)]
pub struct ActionValidation {
    pub normalized: Value,
    pub error: Option<String>,
}

fn get_string_any(obj: &Map<String, Value>, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(val) = obj.get(*key).and_then(|v| v.as_str()) {
            let trimmed = val.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

fn parse_keys_array(arr: &[Value]) -> Option<(String, Vec<String>)> {
    let mut key: Option<String> = None;
    let mut modifiers: Vec<String> = Vec::new();

    for val in arr.iter() {
        let raw = val.as_str().unwrap_or("").trim().to_lowercase();
        if raw.is_empty() {
            continue;
        }
        if let Some(normalized) = normalize_modifier(&raw) {
            if !modifiers.contains(&normalized) {
                modifiers.push(normalized);
            }
            continue;
        }
        if key.is_none() {
            key = Some(raw.to_string());
        } else if !modifiers.contains(&raw) {
            modifiers.push(raw.to_string());
        }
    }

    key.map(|k| (k, modifiers))
}

fn normalize_modifier(raw: &str) -> Option<String> {
    match raw.trim().to_lowercase().as_str() {
        "cmd" | "command" => Some("command".to_string()),
        "shift" => Some("shift".to_string()),
        "option" | "alt" => Some("option".to_string()),
        "control" | "ctrl" => Some("control".to_string()),
        _ => None,
    }
}

fn parse_shortcut_combo(raw: &str) -> Option<(String, Vec<String>)> {
    let compact = raw.trim().to_lowercase();
    if !compact.contains('+') {
        return None;
    }

    let mut key: Option<String> = None;
    let mut modifiers: Vec<String> = Vec::new();
    for token in compact.split('+') {
        let token = token.trim();
        if token.is_empty() {
            continue;
        }
        if let Some(modifier) = normalize_modifier(token) {
            if !modifiers.contains(&modifier) {
                modifiers.push(modifier);
            }
            continue;
        }
        key = Some(token.to_string());
    }

    key.map(|k| (k, modifiers))
}

fn canonicalize_shortcut_alias(key: &str, modifiers: &mut Vec<String>) -> String {
    let mut key_lower = key.trim().to_lowercase();
    match key_lower.as_str() {
        "paste" | "붙여넣기" => {
            key_lower = "v".to_string();
            if !modifiers.iter().any(|m| m == "command") {
                modifiers.push("command".to_string());
            }
        }
        "copy" | "복사" => {
            key_lower = "c".to_string();
            if !modifiers.iter().any(|m| m == "command") {
                modifiers.push("command".to_string());
            }
        }
        "select_all" | "selectall" | "전체선택" => {
            key_lower = "a".to_string();
            if !modifiers.iter().any(|m| m == "command") {
                modifiers.push("command".to_string());
            }
        }
        "new" | "새로만들기" => {
            key_lower = "n".to_string();
            if !modifiers.iter().any(|m| m == "command") {
                modifiers.push("command".to_string());
            }
        }
        _ => {}
    }
    key_lower
}

fn normalize_action_name(raw: &str) -> String {
    let lower = raw.trim().to_lowercase();
    match lower.as_str() {
        "open_browser" => "open_url".to_string(),
        "open" => "open_url".to_string(),
        "click" | "ui.click" | "click_text" => "click_visual".to_string(),
        "take_snapshot" => "snapshot".to_string(),
        "mcp_call" | "external_tool" => "mcp".to_string(),
        "copy_to_clipboard" => "copy".to_string(),
        "paste_clipboard" => "paste".to_string(),
        "get_clipboard" => "read_clipboard".to_string(),
        "copy_between_apps" => "transfer".to_string(),
        "mail_send" | "send_mail" | "email_send" | "send_email" => "mail_send".to_string(),
        other => other.to_string(),
    }
}

pub fn normalize_action(plan: &Value) -> ActionValidation {
    let mut normalized = plan.clone();

    if normalized
        .get("action")
        .map(|v| v.is_object())
        .unwrap_or(false)
    {
        normalized = normalized["action"].clone();
    }

    let obj = match normalized.as_object_mut() {
        Some(obj) => obj,
        None => {
            return ActionValidation {
                normalized: json!({"action": "report", "message": "Invalid action: not an object"}),
                error: Some("Action must be a JSON object".to_string()),
            }
        }
    };

    let mut action = obj
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    if action.is_empty() {
        if let Some(tool) = obj.get("tool").and_then(|v| v.as_str()) {
            action = tool.to_string();
        } else if let Some(kind) = obj.get("type").and_then(|v| v.as_str()) {
            action = kind.to_string();
        }
    }

    if action.is_empty() {
        return ActionValidation {
            normalized: json!({"action": "report", "message": "Invalid action: missing action field"}),
            error: Some("Missing action field".to_string()),
        };
    }

    let action = normalize_action_name(&action);
    obj.insert("action".to_string(), Value::String(action.clone()));

    let mut error: Option<String> = None;

    match action.as_str() {
        "open_app" => {
            if let Some(name) = get_string_any(obj, &["name", "app"]) {
                obj.insert("name".to_string(), Value::String(name));
            } else {
                error = Some("open_app requires 'name'".to_string());
            }
        }
        "open_url" => {
            if let Some(url) = get_string_any(obj, &["url", "link"]) {
                obj.insert("url".to_string(), Value::String(url));
            } else {
                error = Some("open_url requires 'url'".to_string());
            }
        }
        "type" => {
            if let Some(text) = get_string_any(obj, &["text"]) {
                obj.insert("text".to_string(), Value::String(text));
            } else {
                error = Some("type requires 'text'".to_string());
            }
        }
        "key" => {
            if let Some(key) = get_string_any(obj, &["key"]) {
                if let Some((combo_key, combo_modifiers)) = parse_shortcut_combo(&key) {
                    obj.insert("action".to_string(), Value::String("shortcut".to_string()));
                    obj.insert("key".to_string(), Value::String(combo_key));
                    obj.insert(
                        "modifiers".to_string(),
                        Value::Array(combo_modifiers.into_iter().map(Value::String).collect()),
                    );
                } else {
                    obj.insert("key".to_string(), Value::String(key));
                }
            } else {
                error = Some("key requires 'key'".to_string());
            }
        }
        "shortcut" => {
            let mut key = get_string_any(obj, &["key"]);
            let mut modifiers: Vec<String> = Vec::new();

            if let Some(existing_modifiers) = obj.get("modifiers") {
                match existing_modifiers {
                    Value::Array(arr) => {
                        for item in arr {
                            if let Some(raw) = item.as_str() {
                                if let Some(normalized) = normalize_modifier(raw) {
                                    if !modifiers.contains(&normalized) {
                                        modifiers.push(normalized);
                                    }
                                }
                            }
                        }
                    }
                    Value::String(raw) => {
                        if let Some(normalized) = normalize_modifier(raw) {
                            modifiers.push(normalized);
                        }
                    }
                    _ => {}
                }
            }

            if let Some(raw_key) = key.clone() {
                if let Some((parsed_key, parsed_mods)) = parse_shortcut_combo(&raw_key) {
                    key = Some(parsed_key);
                    for modifier in parsed_mods {
                        if !modifiers.contains(&modifier) {
                            modifiers.push(modifier);
                        }
                    }
                }
            }

            if key.is_none() {
                if let Some(arr) = obj.get("keys").and_then(|v| v.as_array()) {
                    if let Some((parsed_key, parsed_modifiers)) = parse_keys_array(arr) {
                        key = Some(parsed_key);
                        for modifier in parsed_modifiers {
                            if let Some(normalized) = normalize_modifier(&modifier) {
                                if !modifiers.contains(&normalized) {
                                    modifiers.push(normalized);
                                }
                            }
                        }
                    }
                }
            }

            if key.is_none() {
                error = Some("shortcut requires 'key'".to_string());
            } else {
                let canonical_key = canonicalize_shortcut_alias(
                    key.as_deref().unwrap_or_default(),
                    &mut modifiers,
                );
                obj.insert("key".to_string(), Value::String(canonical_key.clone()));
                obj.insert(
                    "modifiers".to_string(),
                    Value::Array(modifiers.into_iter().map(Value::String).collect()),
                );

                let has_command = obj
                    .get("modifiers")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .any(|m| m.as_str().unwrap_or("").eq_ignore_ascii_case("command"))
                    })
                    .unwrap_or(false);
                if has_command {
                    match canonical_key.as_str() {
                        "v" => {
                            obj.insert("action".to_string(), Value::String("paste".to_string()));
                        }
                        "c" => {
                            obj.insert("action".to_string(), Value::String("copy".to_string()));
                        }
                        "a" => {
                            obj.insert(
                                "action".to_string(),
                                Value::String("select_all".to_string()),
                            );
                        }
                        _ => {}
                    }
                }
            }
        }
        "click_visual" => {
            if let Some(desc) = get_string_any(obj, &["description", "text"]) {
                obj.insert("description".to_string(), Value::String(desc));
            } else {
                error = Some("click_visual requires 'description'".to_string());
            }
        }
        "read" => {
            if let Some(query) = get_string_any(obj, &["query"]) {
                obj.insert("query".to_string(), Value::String(query));
            } else {
                obj.insert(
                    "query".to_string(),
                    Value::String("Describe the screen".to_string()),
                );
            }
        }
        "scroll" => {
            if let Some(dir) = get_string_any(obj, &["direction"]) {
                obj.insert("direction".to_string(), Value::String(dir));
            } else {
                obj.insert("direction".to_string(), Value::String("down".to_string()));
            }
        }
        "select_text" => {
            if let Some(text) = get_string_any(obj, &["text"]) {
                obj.insert("text".to_string(), Value::String(text));
            } else {
                error = Some("select_text requires 'text'".to_string());
            }
        }
        "snapshot" => {}
        "click_ref" => {
            if let Some(r) = get_string_any(obj, &["ref", "id"]) {
                obj.insert("ref".to_string(), Value::String(r));
            } else {
                error = Some("click_ref requires 'ref'".to_string());
            }
        }
        "switch_app" | "activate_app" => {
            if let Some(name) = get_string_any(obj, &["app", "name"]) {
                obj.insert("app".to_string(), Value::String(name));
            } else {
                error = Some("switch_app requires 'app'".to_string());
            }
        }
        "copy" => {
            if let Some(text) = get_string_any(obj, &["text", "content"]) {
                obj.insert("text".to_string(), Value::String(text));
            }
        }
        "select_all" => {}
        "paste" => {}
        "read_clipboard" => {}
        "mail_send" => {}
        "transfer" => {
            let from = get_string_any(obj, &["from"]);
            let to = get_string_any(obj, &["to"]);
            if let (Some(f), Some(t)) = (from, to) {
                obj.insert("from".to_string(), Value::String(f));
                obj.insert("to".to_string(), Value::String(t));
            } else {
                error = Some("transfer requires 'from' and 'to'".to_string());
            }
        }
        "mcp" => {
            let server = get_string_any(obj, &["server"]);
            let tool = get_string_any(obj, &["tool"]);
            if let (Some(s), Some(t)) = (server, tool) {
                obj.insert("server".to_string(), Value::String(s));
                obj.insert("tool".to_string(), Value::String(t));
            } else {
                error = Some("mcp requires 'server' and 'tool'".to_string());
            }
        }
        "mcp_list" => {}
        "shell" | "run_shell" => {
            if let Some(cmd) = get_string_any(obj, &["command", "cmd"]) {
                obj.insert("command".to_string(), Value::String(cmd));
            } else {
                error = Some("shell requires 'command'".to_string());
            }
        }
        "spawn_agent" => {
            if let Some(task) = get_string_any(obj, &["task"]) {
                obj.insert("task".to_string(), Value::String(task));
                if let Some(name) = get_string_any(obj, &["name"]) {
                    obj.insert("name".to_string(), Value::String(name));
                }
            } else {
                error = Some("spawn_agent requires 'task'".to_string());
            }
        }
        "save_routine" | "replay_routine" => {
            if let Some(name) = get_string_any(obj, &["name"]) {
                obj.insert("name".to_string(), Value::String(name));
            } else {
                error = Some("routine action requires 'name'".to_string());
            }
        }
        "read_file" => {
            if let Some(path) = get_string_any(obj, &["path"]) {
                obj.insert("path".to_string(), Value::String(path));
            } else {
                error = Some("read_file requires 'path'".to_string());
            }
        }
        "open_desktop_image" => {}
        "wait" => {
            if obj.get("seconds").is_none() {
                obj.insert("seconds".to_string(), Value::Number(2.into()));
            }
        }
        "report" => {
            if let Some(msg) = get_string_any(obj, &["message", "reason"]) {
                obj.insert("message".to_string(), Value::String(msg));
            } else {
                obj.insert(
                    "message".to_string(),
                    Value::String("Progress update".to_string()),
                );
            }
        }
        "reply" => {
            if let Some(text) = get_string_any(obj, &["text", "message"]) {
                obj.insert("text".to_string(), Value::String(text));
            } else {
                error = Some("reply requires 'text'".to_string());
            }
        }
        "done" | "fail" => {}
        other => {
            error = Some(format!("Unknown action: {}", other));
        }
    }

    ActionValidation { normalized, error }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_open_app() {
        let plan = json!({"action": "open_app", "app": "Notes"});
        let result = normalize_action(&plan);
        assert!(result.error.is_none());
        assert_eq!(result.normalized["name"].as_str().unwrap(), "Notes");
    }

    #[test]
    fn normalize_shortcut_keys() {
        let plan = json!({"action": "shortcut", "keys": ["command", "n"]});
        let result = normalize_action(&plan);
        assert!(result.error.is_none());
        assert_eq!(result.normalized["key"].as_str().unwrap(), "n");
        assert_eq!(result.normalized["modifiers"][0].as_str().unwrap(), "command");
    }

    #[test]
    fn normalize_shortcut_cmd_plus_v_to_paste() {
        let plan = json!({"action": "shortcut", "key": "cmd+v"});
        let result = normalize_action(&plan);
        assert!(result.error.is_none());
        assert_eq!(result.normalized["action"].as_str().unwrap(), "paste");
    }

    #[test]
    fn normalize_key_combo_to_shortcut() {
        let plan = json!({"action": "key", "key": "command+n"});
        let result = normalize_action(&plan);
        assert!(result.error.is_none());
        assert_eq!(result.normalized["action"].as_str().unwrap(), "shortcut");
        assert_eq!(result.normalized["key"].as_str().unwrap(), "n");
        assert_eq!(result.normalized["modifiers"][0].as_str().unwrap(), "command");
    }

    #[test]
    fn unknown_action_errors() {
        let plan = json!({"action": "unknown"});
        let result = normalize_action(&plan);
        assert!(result.error.is_some());
    }
}
