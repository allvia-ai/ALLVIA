use anyhow::{Context, Result};
use serde_json::Value;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct PeekabooElement {
    pub id: String,
    pub role: String,
    pub name: String,
    pub bounds: Option<(i32, i32, i32, i32)>,
}

#[derive(Debug, Clone)]
pub struct PeekabooSnapshot {
    pub snapshot_id: Option<String>,
    pub elements: Vec<PeekabooElement>,
}

#[derive(Debug, Clone)]
pub struct PeekabooPermissions {
    pub screen_recording: Option<bool>,
    pub accessibility: Option<bool>,
}

pub fn is_available() -> bool {
    let timeout_ms = std::env::var("STEER_PEEKABOO_PROBE_TIMEOUT_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(1200);

    let mut child = match Command::new("peekaboo")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(_) => return false,
    };

    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return false;
                }
                std::thread::sleep(Duration::from_millis(60));
            }
            Err(_) => return false,
        }
    }
}

pub fn check_permissions() -> Result<PeekabooPermissions> {
    let output = Command::new("peekaboo")
        .arg("permissions")
        .arg("--json")
        .output()
        .context("Failed to execute peekaboo permissions")?;

    if !output.status.success() {
        return Err(anyhow::anyhow!(
            "peekaboo permissions failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: Value =
        serde_json::from_str(&stdout).context("peekaboo permissions returned non-JSON output")?;

    let screen_recording = find_bool_any(
        &json,
        &["screenRecording", "screen_recording", "screen-recording"],
    );
    let accessibility = find_bool_any(&json, &["accessibility"]);

    Ok(PeekabooPermissions {
        screen_recording,
        accessibility,
    })
}

pub fn take_snapshot(app: Option<&str>) -> Result<PeekabooSnapshot> {
    let mut cmd = Command::new("peekaboo");
    cmd.arg("see").arg("--json");

    if let Some(app_name) = app {
        cmd.arg("--app").arg(app_name);
    }

    let output = cmd.output().context("Failed to execute peekaboo see")?;
    if !output.status.success() {
        return Err(anyhow::anyhow!(
            "peekaboo see failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: Value =
        serde_json::from_str(&stdout).context("peekaboo see returned non-JSON output")?;

    let snapshot_id = find_string_any(&json, &["snapshotId", "snapshot_id", "snapshot", "id"]);
    let elements = parse_elements(&json);

    Ok(PeekabooSnapshot {
        snapshot_id,
        elements,
    })
}

pub fn click(ref_id: &str, snapshot_id: Option<&str>, app: Option<&str>) -> Result<()> {
    let mut cmd = Command::new("peekaboo");
    cmd.arg("click").arg("--on").arg(ref_id);
    cmd.arg("--focus-retry-count").arg("2");
    cmd.arg("--focus-timeout-seconds").arg("2");

    if let Some(snapshot) = snapshot_id {
        cmd.arg("--snapshot").arg(snapshot);
    }

    if let Some(app_name) = app {
        cmd.arg("--app").arg(app_name);
    }

    let output = cmd.output().context("Failed to execute peekaboo click")?;
    if !output.status.success() {
        return Err(anyhow::anyhow!(
            "peekaboo click failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    Ok(())
}

fn find_bool_any(value: &Value, keys: &[&str]) -> Option<bool> {
    if let Some(obj) = value.as_object() {
        for key in keys {
            if let Some(val) = obj.get(*key) {
                if let Some(b) = val.as_bool() {
                    return Some(b);
                }
                if let Some(s) = val.as_str() {
                    let s = s.trim().to_lowercase();
                    if s == "true" || s == "granted" || s == "yes" {
                        return Some(true);
                    }
                    if s == "false" || s == "denied" || s == "no" {
                        return Some(false);
                    }
                }
            }
        }
        for val in obj.values() {
            if let Some(found) = find_bool_any(val, keys) {
                return Some(found);
            }
        }
    } else if let Some(arr) = value.as_array() {
        for val in arr {
            if let Some(found) = find_bool_any(val, keys) {
                return Some(found);
            }
        }
    }
    None
}

fn find_string_any(value: &Value, keys: &[&str]) -> Option<String> {
    if let Some(obj) = value.as_object() {
        for key in keys {
            if let Some(val) = obj.get(*key) {
                if let Some(s) = val.as_str() {
                    let trimmed = s.trim();
                    if !trimmed.is_empty() {
                        return Some(trimmed.to_string());
                    }
                }
            }
        }
        for val in obj.values() {
            if let Some(found) = find_string_any(val, keys) {
                return Some(found);
            }
        }
    } else if let Some(arr) = value.as_array() {
        for val in arr {
            if let Some(found) = find_string_any(val, keys) {
                return Some(found);
            }
        }
    }
    None
}

fn parse_elements(root: &Value) -> Vec<PeekabooElement> {
    let mut elements: Vec<PeekabooElement> = Vec::new();
    if let Some(arr) = find_elements_array(root) {
        for elem in arr {
            if let Some(parsed) = parse_element(elem) {
                elements.push(parsed);
            }
        }
    }
    elements
}

fn find_elements_array(value: &Value) -> Option<&Vec<Value>> {
    if let Some(obj) = value.as_object() {
        if let Some(arr) = obj.get("elements").and_then(|v| v.as_array()) {
            return Some(arr);
        }
        if let Some(arr) = obj.get("items").and_then(|v| v.as_array()) {
            return Some(arr);
        }
        if let Some(arr) = obj.get("nodes").and_then(|v| v.as_array()) {
            return Some(arr);
        }
        if let Some(arr) = obj.get("refs").and_then(|v| v.as_array()) {
            return Some(arr);
        }
        for val in obj.values() {
            if let Some(arr) = find_elements_array(val) {
                return Some(arr);
            }
        }
    } else if let Some(arr) = value.as_array() {
        for val in arr {
            if let Some(arr) = find_elements_array(val) {
                return Some(arr);
            }
        }
    }
    None
}

fn parse_element(value: &Value) -> Option<PeekabooElement> {
    let obj = value.as_object()?;
    let id = find_string_in_obj(obj, &["id", "ref", "elementId", "element_id"])?;
    let role =
        find_string_in_obj(obj, &["role", "type", "kind"]).unwrap_or_else(|| "unknown".to_string());
    let name =
        find_string_in_obj(obj, &["name", "title", "label", "text", "value"]).unwrap_or_default();
    let bounds = parse_bounds(obj);

    Some(PeekabooElement {
        id,
        role,
        name,
        bounds,
    })
}

fn find_string_in_obj(obj: &serde_json::Map<String, Value>, keys: &[&str]) -> Option<String> {
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

fn parse_bounds(obj: &serde_json::Map<String, Value>) -> Option<(i32, i32, i32, i32)> {
    let candidate = obj
        .get("bounds")
        .or_else(|| obj.get("frame"))
        .or_else(|| obj.get("rect"));

    let bounds = candidate?.as_object()?;

    let x = bounds
        .get("x")
        .or_else(|| bounds.get("left"))
        .and_then(|v| v.as_f64())? as i32;
    let y = bounds
        .get("y")
        .or_else(|| bounds.get("top"))
        .and_then(|v| v.as_f64())? as i32;
    let width = bounds
        .get("width")
        .or_else(|| bounds.get("w"))
        .and_then(|v| v.as_f64())? as i32;
    let height = bounds
        .get("height")
        .or_else(|| bounds.get("h"))
        .and_then(|v| v.as_f64())? as i32;

    Some((x, y, width, height))
}
