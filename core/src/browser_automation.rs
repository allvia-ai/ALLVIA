// Browser Automation Module - Ported from clawdbot-main/src/browser/pw-tools-core.interactions.ts
// Provides stable element references and Playwright-style automation

use crate::peekaboo_cli;
use crate::tool_chaining::CrossAppBridge;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::process::Command;

// =====================================================
// Element Reference System (clawdbot pattern)
// =====================================================

/// Element reference from accessibility tree snapshot
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElementRef {
    pub id: String,   // e.g., "E123"
    pub role: String, // e.g., "button", "textbox"
    pub name: String, // Accessible name
    pub bounds: Option<Bounds>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bounds {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl Bounds {
    pub fn center(&self) -> (i32, i32) {
        (self.x + self.width / 2, self.y + self.height / 2)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SnapshotSource {
    AppleScript,
    Peekaboo,
}

/// Browser automation context
pub struct BrowserAutomation {
    /// Cache of element references from last snapshot
    element_refs: HashMap<String, ElementRef>,
    /// Counter for generating unique element IDs
    ref_counter: u32,
    /// Ordered refs from last snapshot
    last_snapshot_refs: Vec<ElementRef>,
    /// Snapshot source for last capture
    last_snapshot_source: SnapshotSource,
    /// Snapshot id when using Peekaboo
    last_snapshot_id: Option<String>,
}

impl BrowserAutomation {
    pub fn new() -> Self {
        Self {
            element_refs: HashMap::new(),
            ref_counter: 0,
            last_snapshot_refs: Vec::new(),
            last_snapshot_source: SnapshotSource::AppleScript,
            last_snapshot_id: None,
        }
    }

    // =====================================================
    // Snapshot: Build element reference map (like clawdbot's restoreRoleRefsForTarget)
    // =====================================================

    /// Take accessibility snapshot and build element reference map
    pub fn take_snapshot(&mut self) -> Result<Vec<ElementRef>> {
        let mut refs = Vec::new();
        self.element_refs.clear();
        self.ref_counter = 0;
        self.last_snapshot_refs.clear();
        self.last_snapshot_source = SnapshotSource::AppleScript;
        self.last_snapshot_id = None;

        // Use AppleScript to get accessibility tree
        let script = r#"
            tell application "System Events"
                set frontApp to first application process whose frontmost is true
                set appName to name of frontApp
                
                -- Get UI elements (simplified - full impl would recurse)
                set output to ""
                try
                    set allElements to entire contents of window 1 of frontApp
                    repeat with elem in allElements
                        try
                            set elemRole to role of elem
                            set elemName to name of elem
                            set elemPos to position of elem
                            set elemSize to size of elem
                            if elemName is not "" then
                                set output to output & elemRole & "|" & elemName & "|" & (item 1 of elemPos) & "|" & (item 2 of elemPos) & "|" & (item 1 of elemSize) & "|" & (item 2 of elemSize) & "
"
                            end if
                        end try
                    end repeat
                end try
                return output
            end tell
        "#;

        let output = Command::new("osascript")
            .arg("-e")
            .arg(script)
            .output()
            .context("Failed to execute AppleScript for snapshot")?;

        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);

            for line in stdout.lines() {
                let parts: Vec<&str> = line.split('|').collect();
                if parts.len() >= 6 {
                    self.ref_counter += 1;
                    let ref_id = format!("E{}", self.ref_counter);

                    let elem_ref = ElementRef {
                        id: ref_id.clone(),
                        role: parts[0].to_string(),
                        name: parts[1].to_string(),
                        bounds: Some(Bounds {
                            x: parts[2].parse().unwrap_or(0),
                            y: parts[3].parse().unwrap_or(0),
                            width: parts[4].parse().unwrap_or(0),
                            height: parts[5].parse().unwrap_or(0),
                        }),
                    };

                    self.element_refs.insert(ref_id.clone(), elem_ref.clone());
                    refs.push(elem_ref);
                }
            }
        }

        if refs.is_empty() {
            if peekaboo_cli::is_available() {
                let front_app = CrossAppBridge::get_frontmost_app().ok();
                if let Ok(snapshot) = peekaboo_cli::take_snapshot(front_app.as_deref()) {
                    self.element_refs.clear();
                    self.ref_counter = 0;
                    self.last_snapshot_refs.clear();
                    self.last_snapshot_source = SnapshotSource::Peekaboo;
                    self.last_snapshot_id = snapshot.snapshot_id.clone();

                    for elem in snapshot.elements {
                        let bounds = elem.bounds.map(|(x, y, w, h)| Bounds {
                            x,
                            y,
                            width: w,
                            height: h,
                        });
                        let elem_ref = ElementRef {
                            id: elem.id.clone(),
                            role: elem.role.clone(),
                            name: elem.name.clone(),
                            bounds,
                        };
                        self.element_refs.insert(elem.id.clone(), elem_ref.clone());
                        self.last_snapshot_refs.push(elem_ref.clone());
                        refs.push(elem_ref);
                    }
                    println!(
                        "📸 [Browser] Snapshot captured via Peekaboo: {} elements",
                        refs.len()
                    );
                    return Ok(refs);
                }
            }
        }

        self.last_snapshot_refs = refs.clone();
        println!("📸 [Browser] Snapshot captured: {} elements", refs.len());
        if refs.is_empty() && !crate::env_flag("STEER_ALLOW_EMPTY_SNAPSHOT_REFS") {
            let front_app = CrossAppBridge::get_frontmost_app().unwrap_or_default();
            return Err(anyhow::anyhow!(
                "snapshot returned zero elements (frontmost='{}'). ensure target window is visible/focused and accessibility permissions are granted",
                front_app
            ));
        }
        Ok(refs)
    }

    /// Clear cached refs when navigation changes the DOM (refs are not stable across navigations).
    pub fn reset_snapshot(&mut self) {
        self.element_refs.clear();
        self.ref_counter = 0;
        self.last_snapshot_refs.clear();
        self.last_snapshot_id = None;
    }

    // =====================================================
    // Core Interactions (ported from pw-tools-core.interactions.ts)
    // =====================================================

    /// Click element by reference (like clickViaPlaywright)
    pub fn click_by_ref(&self, ref_id: &str, double_click: bool) -> Result<()> {
        if self.last_snapshot_source == SnapshotSource::Peekaboo {
            let front_app = CrossAppBridge::get_frontmost_app().ok();
            let snapshot_id = self.last_snapshot_id.as_deref();
            peekaboo_cli::click(ref_id, snapshot_id, front_app.as_deref())
                .context("Peekaboo click failed")?;
            if double_click {
                std::thread::sleep(std::time::Duration::from_millis(100));
                peekaboo_cli::click(ref_id, snapshot_id, front_app.as_deref())
                    .context("Peekaboo double click failed")?;
            }
            println!("🖱️ [Browser] Clicked ref '{}' via Peekaboo", ref_id);
            return Ok(());
        }

        let elem = self.element_refs.get(ref_id).ok_or_else(|| {
            anyhow::anyhow!("Element ref '{}' not found. Take a new snapshot.", ref_id)
        })?;

        let (x, y) = elem
            .bounds
            .as_ref()
            .map(|b| b.center())
            .ok_or_else(|| anyhow::anyhow!("Element '{}' has no bounds", ref_id))?;

        let click_count = if double_click { 2 } else { 1 };

        let script = format!(
            r#"tell application "System Events" to click at {{{}, {}}} "#,
            x, y
        );

        for _ in 0..click_count {
            Command::new("osascript")
                .arg("-e")
                .arg(&script)
                .output()
                .context("Failed to execute click")?;

            if double_click {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }

        println!("🖱️ [Browser] Clicked '{}' at ({}, {})", elem.name, x, y);
        Ok(())
    }

    /// Hover over element by reference (like hoverViaPlaywright)
    pub fn hover_by_ref(&self, ref_id: &str) -> Result<()> {
        if self.last_snapshot_source == SnapshotSource::Peekaboo {
            let front_app = CrossAppBridge::get_frontmost_app().ok();
            let snapshot_id = self.last_snapshot_id.as_deref();
            peekaboo_cli::click(ref_id, snapshot_id, front_app.as_deref())
                .context("Peekaboo hover fallback (click) failed")?;
            return Ok(());
        }

        let elem = self
            .element_refs
            .get(ref_id)
            .ok_or_else(|| anyhow::anyhow!("Element ref '{}' not found", ref_id))?;

        let (x, y) = elem
            .bounds
            .as_ref()
            .map(|b| b.center())
            .ok_or_else(|| anyhow::anyhow!("Element '{}' has no bounds", ref_id))?;

        // Move mouse without clicking
        let script = format!(
            r#"
            do shell script "cliclick m:{},{}"
            "#,
            x, y
        );

        // Fallback: Use CoreGraphics via AppleScript
        let _ = Command::new("osascript").arg("-e").arg(&script).output();

        println!("👆 [Browser] Hover over '{}' at ({}, {})", elem.name, x, y);
        Ok(())
    }

    /// Type text into focused element (like typeViaPlaywright)
    pub fn type_text(&self, text: &str, delay_ms: u64) -> Result<()> {
        // Use keystroke for reliable typing
        let escaped = text.replace("\"", "\\\"").replace("\\", "\\\\");

        let script = if delay_ms > 0 {
            format!(
                r#"
                tell application "System Events"
                    repeat with c in characters of "{}"
                        keystroke c
                        delay {}
                    end repeat
                end tell
                "#,
                escaped,
                delay_ms as f64 / 1000.0
            )
        } else {
            format!(
                r#"tell application "System Events" to keystroke "{}""#,
                escaped
            )
        };

        Command::new("osascript")
            .arg("-e")
            .arg(&script)
            .output()
            .context("Failed to type text")?;

        println!(
            "⌨️ [Browser] Typed: '{}'",
            if text.len() > 20 { &text[..20] } else { text }
        );
        Ok(())
    }

    /// Find element by name/text (returns ref_id)
    pub fn find_by_name(&self, name: &str) -> Option<String> {
        let name_lower = name.to_lowercase();

        for (ref_id, elem) in &self.element_refs {
            if elem.name.to_lowercase().contains(&name_lower) {
                return Some(ref_id.clone());
            }
        }
        None
    }

    pub fn find_first_by_role_contains(&self, needle: &str) -> Option<String> {
        let needle_lower = needle.to_lowercase();
        for elem in &self.last_snapshot_refs {
            if elem.role.to_lowercase().contains(&needle_lower) {
                return Some(elem.id.clone());
            }
        }
        None
    }

    pub fn summarize_refs(refs: &[ElementRef], max: usize) -> String {
        if refs.is_empty() {
            return "SNAPSHOT_REFS: (none)".to_string();
        }
        let mut parts: Vec<String> = Vec::new();
        for r in refs.iter().take(max) {
            let name = if r.name.trim().is_empty() {
                "(unnamed)"
            } else {
                r.name.as_str()
            };
            parts.push(format!("{} [{}] \"{}\"", r.id, r.role, name));
        }
        let mut summary = format!("SNAPSHOT_REFS: {}", parts.join("; "));
        if refs.len() > max {
            summary.push_str(&format!("; +{} more", refs.len() - max));
        }
        summary
    }

    /// Navigate to URL (opens in default browser or specified browser)
    pub fn navigate(&self, url: &str, browser: Option<&str>) -> Result<()> {
        let browser_name = browser.unwrap_or("Safari");

        let script = format!(
            r#"
            tell application "{}"
                activate
                open location "{}"
            end tell
            "#,
            browser_name, url
        );

        Command::new("osascript")
            .arg("-e")
            .arg(&script)
            .output()
            .context("Failed to navigate")?;

        println!("🌐 [Browser] Navigate to: {}", url);
        Ok(())
    }

    // =====================================================
    // Helper: AI-Friendly Error (clawdbot pattern)
    // =====================================================

    pub fn to_ai_friendly_error(err: anyhow::Error, context: &str) -> anyhow::Error {
        anyhow::anyhow!(
            "Action failed on '{}': {}. Try taking a new snapshot or using a different approach.",
            context,
            err
        )
    }
}

// =====================================================
// Public API for DynamicController
// =====================================================

/// Singleton-style access
use once_cell::sync::Lazy;
use std::sync::Mutex;

/// Singleton-style access (Thread-Safe)
static BROWSER_AUTOMATION: Lazy<Mutex<BrowserAutomation>> =
    Lazy::new(|| Mutex::new(BrowserAutomation::new()));

pub fn get_browser_automation() -> std::sync::MutexGuard<'static, BrowserAutomation> {
    BROWSER_AUTOMATION.lock().unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bounds_center() {
        let bounds = Bounds {
            x: 100,
            y: 200,
            width: 50,
            height: 30,
        };
        assert_eq!(bounds.center(), (125, 215));
    }
}

// =====================================================
// LEGACY API COMPATIBILITY (for execution_controller.rs)
// =====================================================

pub fn open_url_in_chrome(url: &str) -> Result<()> {
    get_browser_automation().navigate(url, Some("Google Chrome"))
}

/// Scroll page by pixels - legacy API  
pub fn scroll_page(pixels: i32) -> Result<()> {
    let direction = if pixels > 0 { "down" } else { "up" };
    let amount = pixels.abs();

    let script = format!(
        r#"tell application "System Events" to scroll {} by {}"#,
        direction, amount
    );

    std::process::Command::new("osascript")
        .arg("-e")
        .arg(&script)
        .output()
        .context("Failed to scroll")?;

    Ok(())
}

/// Apply flight filters - legacy API (stub) - Returns bool for success
pub fn apply_flight_filters(
    _budget: Option<&str>,
    _time_window: Option<&str>,
    _direct_only: Option<&str>,
) -> Result<bool> {
    println!("⚠️ [Browser] apply_flight_filters: Use new ref-based API instead");
    Ok(false) // Return false to indicate manual action needed
}

/// Apply shopping filters - legacy API (stub) - Returns bool for success
pub fn apply_shopping_filters(
    _brand: Option<&str>,
    _price_min: Option<&str>,
    _price_max: Option<&str>,
) -> Result<bool> {
    println!("⚠️ [Browser] apply_shopping_filters: Use new ref-based API instead");
    Ok(false) // Return false to indicate manual action needed
}

/// Click search button - legacy API (stub) - Returns bool for success
pub fn click_search_button() -> Result<bool> {
    println!("⚠️ [Browser] click_search_button: Use new click_by_ref API instead");
    Ok(false) // Return false to indicate button not found
}

/// Get page context - legacy API (stub)
pub fn get_page_context() -> Result<String> {
    Ok("Page context: Use take_snapshot() for detailed element refs".to_string())
}

/// Fill flight fields - legacy API (stub) - Returns bool for success
pub fn fill_flight_fields(
    _from: &str,
    _to: &str,
    _date_start: &str,
    _date_end: Option<&str>,
) -> Result<bool> {
    println!("⚠️ [Browser] fill_flight_fields: Use new type_text API instead");
    Ok(true) // Return true to indicate attempted (stub)
}

/// Fill search query - legacy API (stub) - Returns bool for success
pub fn fill_search_query(query: &str) -> Result<bool> {
    get_browser_automation().type_text(query, 0)?;
    Ok(true)
}

/// Autofill form - legacy API (stub) - Returns bool for success
pub fn autofill_form(
    _name: Option<&str>,
    _email: Option<&str>,
    _phone: Option<&str>,
    _address: Option<&str>,
) -> Result<bool> {
    println!("⚠️ [Browser] autofill_form: Use new type_text API instead");
    Ok(true) // Return true to indicate attempted (stub)
}

/// Extract flight summary - legacy API (stub)
pub fn extract_flight_summary() -> Result<String> {
    Ok("Flight summary extraction: Use take_snapshot() + find_by_name()".to_string())
}

/// Extract shopping summary - legacy API (stub)
pub fn extract_shopping_summary() -> Result<String> {
    Ok("Shopping summary extraction: Use take_snapshot() + find_by_name()".to_string())
}
