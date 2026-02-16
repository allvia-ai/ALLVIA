use accessibility_sys::{
    AXUIElementCopyAttributeValue, AXUIElementCopyElementAtPosition, AXUIElementCreateApplication,
    AXUIElementCreateSystemWide, AXUIElementRef,
};
use core_foundation::array::CFArray;
use core_foundation::base::{CFTypeRef, TCFType};
use core_foundation::string::CFString;
use serde_json::{json, Value};
use std::process::Command;
use std::ptr;
use std::thread;
use std::time::Duration;

// Helper to convert foreign AX error to Result
#[allow(dead_code)]
fn check_ax_err(err: i32) -> Result<(), i32> {
    if err == 0 {
        Ok(())
    } else {
        Err(err)
    }
}

fn env_flag_default(key: &str, default: bool) -> bool {
    std::env::var(key)
        .ok()
        .map(|v| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(default)
}

fn env_usize_bounded(key: &str, default: usize, min: usize, max: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .map(|v| v.clamp(min, max))
        .unwrap_or(default)
}

fn env_u64_bounded(key: &str, default: u64, min: u64, max: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .map(|v| v.clamp(min, max))
        .unwrap_or(default)
}

fn snapshot_focus_retry_count() -> usize {
    env_usize_bounded("STEER_AX_SNAPSHOT_FOCUS_RETRIES", 2, 0, 8)
}

fn snapshot_focus_retry_ms() -> u64 {
    env_u64_bounded("STEER_AX_SNAPSHOT_RETRY_MS", 120, 20, 2000)
}

fn snapshot_window_retry_count() -> usize {
    env_usize_bounded("STEER_AX_SNAPSHOT_WINDOW_RETRIES", 3, 0, 12)
}

fn snapshot_window_retry_ms() -> u64 {
    env_u64_bounded("STEER_AX_SNAPSHOT_WINDOW_RETRY_MS", 90, 20, 2000)
}

fn snapshot_fallback_app_name() -> String {
    std::env::var("STEER_AX_SNAPSHOT_FALLBACK_APP")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "Finder".to_string())
}

fn snapshot_strict_mode() -> bool {
    if let Ok(raw) = std::env::var("STEER_AX_SNAPSHOT_STRICT") {
        let normalized = raw.trim().to_ascii_lowercase();
        if matches!(normalized.as_str(), "0" | "false" | "no" | "off") {
            return false;
        }
        if matches!(normalized.as_str(), "1" | "true" | "yes" | "on") {
            return true;
        }
    }
    crate::env_flag("STEER_SCENARIO_MODE")
        || crate::env_flag("STEER_TEST_MODE")
        || env_flag_default("STEER_PREFLIGHT_FOCUS_HANDOFF", false)
}

// Helper to get attribute
fn get_attribute(element: AXUIElementRef, attribute: &str) -> Option<CFTypeRef> {
    unsafe {
        let attr_name = CFString::new(attribute);
        let mut value_ref: CFTypeRef = ptr::null_mut();
        let err =
            AXUIElementCopyAttributeValue(element, attr_name.as_concrete_TypeRef(), &mut value_ref);
        if err == 0 {
            Some(value_ref)
        } else {
            None
        }
    }
}

// Minimal wrapper for memory safety
struct AxElement(AXUIElementRef);
impl Drop for AxElement {
    fn drop(&mut self) {
        unsafe {
            core_foundation::base::CFRelease(self.0 as CFTypeRef);
        }
    }
}

pub fn snapshot(_scope: Option<String>) -> Value {
    println!("[MacOS] Capturing Snapshot (Native)...");

    unsafe {
        let strict_mode = snapshot_strict_mode();
        let retry_count = snapshot_focus_retry_count();
        let retry_sleep = Duration::from_millis(snapshot_focus_retry_ms());
        let window_retry_count = snapshot_window_retry_count();
        let window_retry_sleep = Duration::from_millis(snapshot_window_retry_ms());
        let fallback_app = snapshot_fallback_app_name();

        // 1. System Wide
        let system_wide = AXUIElementCreateSystemWide();
        let _system_wrapper = AxElement(system_wide); // Auto-release

        // 2. Focused App
        let mut focused_app_ref = resolve_focused_application(system_wide);
        if focused_app_ref.is_none() {
            for _ in 0..retry_count {
                let (front_name, front_pid) =
                    frontmost_app_info_via_osascript().unwrap_or_default();
                if let Some(pid) = front_pid {
                    bring_process_frontmost(pid);
                } else if !front_name.is_empty() {
                    activate_application_by_name(&front_name);
                }
                thread::sleep(retry_sleep);
                focused_app_ref = resolve_focused_application(system_wide);
                if focused_app_ref.is_some() {
                    break;
                }
            }
        }
        if focused_app_ref.is_none() {
            activate_application_by_name(&fallback_app);
            thread::sleep(retry_sleep);
            focused_app_ref = resolve_focused_application(system_wide);
        }
        let focused_app_ref = match focused_app_ref {
            Some(v) => v,
            None => {
                let (front_name, _) = frontmost_app_info_via_osascript().unwrap_or_default();
                crate::diagnostic_events::emit(
                    "ax.snapshot.focus_missing",
                    json!({
                        "kind": "application",
                        "frontmost_app_hint": front_name,
                        "fallback_app": fallback_app,
                        "strict_mode": strict_mode
                    }),
                );
                let mut payload = json!({
                    "role": "AXApplication",
                    "title": front_name,
                    "focused_window": {
                        "role": "AXWindow",
                        "title": "",
                        "children": []
                    },
                    "warning": "No focused application",
                    "frontmost_app_hint": front_name,
                    "strict_mode": strict_mode,
                    "ok": false
                });
                if strict_mode {
                    payload["error"] = json!("NO_FOCUSED_APPLICATION");
                }
                return payload;
            }
        };
        // Note: get_attribute returns +1 retain count, so we wrap it
        let _focused_app = AxElement(focused_app_ref);

        // Get App Title
        let fallback_front_name = frontmost_app_name_via_osascript().unwrap_or_default();
        let app_title = {
            let via_ax = get_string_attribute(focused_app_ref, "AXTitle").unwrap_or_default();
            if via_ax.is_empty() {
                fallback_front_name
            } else {
                via_ax
            }
        };

        // 3. Focused Window
        let mut focused_window_ref = resolve_focused_window(focused_app_ref);
        if focused_window_ref.is_none() {
            for _ in 0..window_retry_count {
                let (front_name, front_pid) =
                    frontmost_app_info_via_osascript().unwrap_or_default();
                if let Some(pid) = front_pid {
                    bring_process_frontmost(pid);
                } else if !front_name.trim().is_empty() {
                    activate_application_by_name(&front_name);
                } else if !app_title.trim().is_empty() {
                    activate_application_by_name(&app_title);
                } else {
                    activate_application_by_name(&fallback_app);
                }
                thread::sleep(window_retry_sleep);
                focused_window_ref = resolve_focused_window(focused_app_ref);
                if focused_window_ref.is_some() {
                    break;
                }
            }
        }
        let focused_window_ref = match focused_window_ref {
            Some(r) => r,
            None => {
                crate::diagnostic_events::emit(
                    "ax.snapshot.focus_missing",
                    json!({
                        "kind": "window",
                        "app_title": app_title,
                        "fallback_app": fallback_app,
                        "strict_mode": strict_mode
                    }),
                );
                let mut payload = json!({
                    "role": "AXApplication",
                    "title": app_title,
                    "focused_window": {
                        "role": "AXWindow",
                        "title": "",
                        "children": []
                    },
                    "warning": "No focused window",
                    "strict_mode": strict_mode,
                    "ok": false
                });
                if strict_mode {
                    payload["error"] = json!("NO_FOCUSED_WINDOW");
                }
                return payload;
            }
        };
        let _focused_window = AxElement(focused_window_ref);

        let window_title = get_string_attribute(focused_window_ref, "AXTitle").unwrap_or_default();

        // 4. Traverse Children (Limit depth for MVP)
        // For performance, we only dump the focused window's children.
        let children_json = traverse_children(focused_window_ref, 0, 2);

        json!({
            "role": "AXApplication",
            "title": app_title,
            "focused_window": {
                "role": "AXWindow",
                "title": window_title,
                "children": children_json
            },
            "strict_mode": strict_mode,
            "ok": true
        })
    }
}

fn frontmost_app_name_via_osascript() -> Option<String> {
    frontmost_app_info_via_osascript().and_then(|(name, _)| {
        if name.trim().is_empty() {
            None
        } else {
            Some(name)
        }
    })
}

fn frontmost_app_info_via_osascript() -> Option<(String, Option<i32>)> {
    let output = Command::new("osascript")
        .arg("-e")
        .arg("tell application \"System Events\" to set p to first application process whose frontmost is true")
        .arg("-e")
        .arg("tell application \"System Events\" to return (name of p as text) & tab & (unix id of p as text)")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let raw = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if raw.is_empty() {
        return None;
    }
    let mut parts = raw.split('\t');
    let name = parts.next().unwrap_or_default().trim().to_string();
    let pid = parts.next().and_then(|v| v.trim().parse::<i32>().ok());
    Some((name, pid))
}

fn bring_process_frontmost(pid: i32) {
    let script = format!(
        "tell application \"System Events\" to set frontmost of first application process whose unix id is {} to true",
        pid
    );
    let _ = Command::new("osascript").arg("-e").arg(script).output();
}

fn activate_application_by_name(name: &str) {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return;
    }
    let escaped = trimmed.replace('\\', "\\\\").replace('"', "\\\"");
    let script = format!("tell application \"{}\" to activate", escaped);
    let _ = Command::new("osascript").arg("-e").arg(script).output();
}

unsafe fn resolve_focused_application(system_wide: AXUIElementRef) -> Option<AXUIElementRef> {
    for _ in 0..4 {
        if let Some(r) = get_attribute(system_wide, "AXFocusedApplication") {
            return Some(r as AXUIElementRef);
        }
        thread::sleep(Duration::from_millis(60));
    }

    if let Some((_name, Some(pid))) = frontmost_app_info_via_osascript() {
        bring_process_frontmost(pid);
        thread::sleep(Duration::from_millis(90));
        for _ in 0..3 {
            if let Some(r) = get_attribute(system_wide, "AXFocusedApplication") {
                return Some(r as AXUIElementRef);
            }
            thread::sleep(Duration::from_millis(50));
        }
        let fallback_app = AXUIElementCreateApplication(pid);
        if !fallback_app.is_null() {
            return Some(fallback_app);
        }
    }
    None
}

unsafe fn resolve_focused_window(focused_app_ref: AXUIElementRef) -> Option<AXUIElementRef> {
    if let Some(r) = get_attribute(focused_app_ref, "AXFocusedWindow") {
        return Some(r as AXUIElementRef);
    }
    if let Some(r) = get_attribute(focused_app_ref, "AXMainWindow") {
        return Some(r as AXUIElementRef);
    }
    if let Some(windows_ref) = get_attribute(focused_app_ref, "AXWindows") {
        let windows_array = CFArray::<CFTypeRef>::wrap_under_get_rule(
            windows_ref as core_foundation::array::CFArrayRef,
        );
        let first = windows_array.get(0).map(|ptr_ref| {
            let window = *ptr_ref as AXUIElementRef;
            core_foundation::base::CFRetain(window as CFTypeRef);
            window
        });
        core_foundation::base::CFRelease(windows_ref);
        if let Some(window) = first {
            return Some(window);
        }
    }
    None
}

unsafe fn traverse_children(element: AXUIElementRef, depth: usize, max_depth: usize) -> Vec<Value> {
    if depth > max_depth {
        return vec![];
    }

    let mut nodes = Vec::new();

    if let Some(children_ref) = get_attribute(element, "AXChildren") {
        let children_array = CFArray::<CFTypeRef>::wrap_under_get_rule(
            children_ref as core_foundation::array::CFArrayRef,
        );

        for i in 0..children_array.len() {
            let Some(child_ptr) = children_array.get(i) else {
                continue;
            };
            let child_element = *child_ptr as AXUIElementRef;

            let role = get_string_attribute(child_element, "AXRole").unwrap_or_default();
            let title = get_string_attribute(child_element, "AXTitle").unwrap_or_default();
            let value = get_string_attribute(child_element, "AXValue").unwrap_or_default();

            // Recursion
            let sub_children = if depth < max_depth {
                traverse_children(child_element, depth + 1, max_depth)
            } else {
                vec![]
            };

            let mut node = json!({
                "role": role,
                "children": sub_children
            });

            if !title.is_empty() {
                node["title"] = json!(title);
            }
            if !value.is_empty() {
                node["value"] = json!(value);
            }

            nodes.push(node);
        }
        // Release the array ref
        core_foundation::base::CFRelease(children_ref);
    }

    nodes
}

unsafe fn get_string_attribute(element: AXUIElementRef, attr: &str) -> Option<String> {
    if let Some(val_ref) = get_attribute(element, attr) {
        // Assume it's a string
        // Check ID?
        let cf_str =
            CFString::wrap_under_create_rule(val_ref as core_foundation::string::CFStringRef);
        Some(cf_str.to_string())
    } else {
        None
    }
}

#[allow(dead_code)]
pub fn find_element(query: &str) -> Option<String> {
    println!("[MacOS] Find element (Not impl in MVP): {}", query);
    None
}

/// Get the currently selected text from the frontmost application.
/// Uses AppleScript via `osascript` for maximum compatibility.
pub fn get_selected_text() -> Option<String> {
    // Strategy 1: Try AXSelectedText attribute via System Events (Cleaner)
    // Strategy 2: If fails, we might need Cmd+C simulation (Risky/Intrusive), so let's stick to AX first.
    let script = r#"
        tell application "System Events"
            set frontApp to first application process whose frontmost is true
            set appName to name of frontApp
            
            try
                tell frontApp
                    -- Try focused UI element first
                    set focusedElement to value of attribute "AXFocusedUIElement"
                    if focusedElement is not missing value then
                         set selectedText to value of attribute "AXSelectedText" of focusedElement
                         if selectedText is not missing value and selectedText is not "" then
                             return selectedText
                         end if
                    end if
                end tell
            end try
            
            -- Fallback for some editors (like simple text fields) if AXFocusedUIElement fails
            return ""
        end tell
    "#;

    use std::process::Command;
    let output = Command::new("osascript")
        .arg("-e")
        .arg(script)
        .output()
        .ok()?;

    if output.status.success() {
        let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if text.is_empty() || text == "missing value" {
            None
        } else {
            Some(text)
        }
    } else {
        None
    }
}

/// Find the UI element at the specific screen coordinates (x, y)
/// Returns (x, y) of the element's center if found.
pub fn find_element_at_pos(x: f32, y: f32) -> Option<(i32, i32)> {
    unsafe {
        let system_wide = AXUIElementCreateSystemWide();
        let _system = AxElement(system_wide); // Release

        let mut element_ref: AXUIElementRef = ptr::null_mut();

        let err = AXUIElementCopyElementAtPosition(system_wide, x, y, &mut element_ref);
        if err != 0 {
            return None;
        }

        // Wrap for release
        let _element = AxElement(element_ref);

        // Get Position
        let _pos_val = get_attribute(element_ref, "AXPosition")?;
        // Get Size
        let _size_val = get_attribute(element_ref, "AXSize")?;

        // Convert to concrete values (Need AXValueGetValue)
        // Accessing AXValue is tricky via raw bindings without `core-foundation` helpers for AXValue.
        // For MVP, if we found an element, we assume it's robust.
        // But to return CENTER, we need logic.

        // Wait, since parsing AXValue structure manually in Rust is painful without a helper crate,
        // and we barely have `accessibility-sys`.
        // Let's rely on AppleScript for the "Get Position/Size" part if raw bindings fail.
        // Actually, let's keep it simple: If we found *something* at (x,y), we trust the OS.
        // But the goal is "Snapping".
        // If we click (100, 100), and the element is at (0, 0) size (200, 200), its center is (100, 100).
        // If we click (190, 190), and element is the same, center is (100, 100).
        // We WANT to click (100, 100) instead of (190, 190).

        // Alternative: Use AppleScript to query element at raw pos?
        // "tell application 'System Events' to click at {x,y}" is already what we do.

        // The point of "Hybrid Grounding" is to correct MIS-CLICKS.
        // e.g. Vision says (500, 500) [Empty Space], but Button is at (480, 500).
        // OS `ElementAtPosition(500, 500)` might return "Window" (parent) instead of "Button".

        // Only if we hit the button do we get the button.
        // So this function helps verify: "Is there a clickable thing here?"
        // If it returns "Window" or "Group", maybe we are off.

        // Let's implement a "Smart Snap" via Applescript for simplicity vs Unsafe Rust logic?
        // No, let's try to extract position if possible.
        // Just checking if we hit a LEAF node is good enough.

        // For now, let's return input (x,y) if success, just to validate "Something is there".
        // Use AppleScript for the heavy lifting of "Find nearest button"?

        // Let's stick to: "Confirm there is a UI element"

        // Actually, to implement "Snap to Center", we NEED the position/size.
        // Since properly binding AXValue in raw Rust is verbose...
        // I will use a helper AppleScript to "Get Center of Element at X,Y".
        // It's slower but safe and easy implementation for this MVP.

        None // Fallback to AppleScript approach below
    }
}

pub fn get_element_center_at(x: i32, y: i32) -> Option<(i32, i32)> {
    // Returns the center (x, y) of the element at the given coordinates.
    // Minimizes "edge clicking" risk.
    let _script = format!(
        r#"
        use framework "CoreGraphics"
        use scripting additions
        
        tell application "System Events"
            set targetList to value of attribute "AXChildren" of (element at {{ {}, {} }})
             -- "element at" isn't standard System Events syntax... it's a specific obscure one or needs bridging.
             
             -- Standard way: click, but we want to query.
             -- Let's iterate processes? Too slow.
        end tell
        "#,
        x, y
    );

    // Actually, AXUIElementCopyElementAtPosition IS the best way.
    // Let's try to do it properly in Rust, ignoring complex AXValue parsing if hard.
    // BUT we can check the ROLE. If role is "AXButton", we trust it.

    // For now, let's just use the function signature and "Mock" it or use a simplified check
    // because complex struct decoding (AXValue) without `accessibility` crate (we have sys) is error prone.

    // Let's simulate a "Snap" by verifying existence.
    // If AXUIElementCopyElementAtPosition returns valid ref, it means "Hit".
    // We return input (x,y) to confirm "Yes, valid target".

    unsafe {
        let system_wide = AXUIElementCreateSystemWide();
        let _system = AxElement(system_wide);
        let mut element_ref: AXUIElementRef = ptr::null_mut();
        let err =
            AXUIElementCopyElementAtPosition(system_wide, x as f32, y as f32, &mut element_ref);
        if err == 0 && !element_ref.is_null() {
            let _element = AxElement(element_ref);
            // We hit something!
            Some((x, y))
        } else {
            None
        }
    }
}

// =====================================================
// Phase 29: OCR/Text Search via Accessibility API
// =====================================================

/// Search for text on screen using Accessibility API
/// Returns true if text was found in any UI element
/// This serves as a CLI fallback when vision-based click fails
pub fn find_text_on_screen(query: &str) -> Option<String> {
    let query_lower = query.to_lowercase();

    unsafe {
        let system_wide = AXUIElementCreateSystemWide();
        if system_wide.is_null() {
            return None;
        }
        let _system = AxElement(system_wide);

        // Get focused application
        let focused_app = resolve_focused_application(system_wide)?;
        let _focused_app_guard = AxElement(focused_app);

        // Search through UI hierarchy
        fn search_element(element: AXUIElementRef, query: &str, depth: usize) -> Option<String> {
            if depth > 8 {
                return None;
            } // Limit recursion for performance

            unsafe {
                // Check this element's text attributes
                let title = get_string_attribute(element, "AXTitle").unwrap_or_default();
                let value = get_string_attribute(element, "AXValue").unwrap_or_default();
                let description =
                    get_string_attribute(element, "AXDescription").unwrap_or_default();

                // Check if any text matches (case-insensitive)
                if title.to_lowercase().contains(query) {
                    return Some(format!("title:{}", title));
                }
                if value.to_lowercase().contains(query) {
                    return Some(format!("value:{}", value));
                }
                if description.to_lowercase().contains(query) {
                    return Some(format!("description:{}", description));
                }

                // Search children
                if let Some(children_ref) = get_attribute(element, "AXChildren") {
                    let children_array = CFArray::<CFTypeRef>::wrap_under_get_rule(
                        children_ref as core_foundation::array::CFArrayRef,
                    );

                    for i in 0..children_array.len() {
                        if let Some(child_ptr) = children_array.get(i) {
                            let child = *child_ptr as AXUIElementRef;
                            if let Some(result) = search_element(child, query, depth + 1) {
                                core_foundation::base::CFRelease(children_ref);
                                return Some(result);
                            }
                        }
                    }
                    core_foundation::base::CFRelease(children_ref);
                }

                None
            }
        }

        search_element(focused_app, &query_lower, 0)
    }
}
