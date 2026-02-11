use anyhow::{anyhow, Result};
use lazy_static::lazy_static;
use log::{error, info};
use std::collections::HashSet;
use std::process::Command;
use std::sync::Mutex;

// Global cache of installed applications (loaded once at startup).
lazy_static! {
    static ref INSTALLED_APPS: Mutex<Option<HashSet<String>>> = Mutex::new(None);
}

/// 1. Environment Scanner: Scan installed apps via system_profiler
pub fn scan_app_inventory() -> Result<()> {
    info!("🔍 [Reality] Scanning installed applications...");

    // Use system_profiler to get app names (SLOW but accurate)
    // Optimization: For PoC, we can also use `mdfind kMDItemContentType == 'com.apple.application-bundle'` which is faster
    // Let's use `mdfind` for speed.
    let output = Command::new("mdfind")
        .arg("kMDItemContentType == 'com.apple.application-bundle'")
        .output()?;

    if !output.status.success() {
        return Err(anyhow!("Failed to scan apps"));
    }

    let stdout = String::from_utf8(output.stdout)?;
    let mut apps = HashSet::new();

    for line in stdout.lines() {
        // Line is full path: /System/Applications/Calendar.app
        if let Some(name) = line.split('/').last() {
            // Remove .app extension
            let clean_name = name.trim_end_matches(".app").to_string();
            apps.insert(clean_name.to_lowercase()); // Store as lowercase for fuzzy matching
        }
    }

    // Add some known defaults that might be hidden or system aliases
    apps.insert("finder".to_string());
    apps.insert("terminal".to_string());
    apps.insert("safari".to_string());
    apps.insert("google chrome".to_string());

    println!(
        "✅ [Reality] Inventory Complete. Found {} apps.",
        apps.len()
    );
    // Debug print a few apps
    let sample: Vec<_> = apps.iter().take(5).collect();
    println!("   Sample: {:?}", sample);

    if let Ok(mut cache) = INSTALLED_APPS.lock() {
        *cache = Some(apps);
    }

    Ok(())
}

/// 2. Pre-Flight Check: App Existence
pub fn verify_app_exists(app_name: &str) -> Result<String> {
    let has_cache = INSTALLED_APPS.lock().map(|c| c.is_some()).unwrap_or(false);
    if !has_cache {
        println!("⚠️ [Reality] Inventory is NONE. Attempting lazy app scan...");
        if let Err(e) = scan_app_inventory() {
            println!("⚠️ [Reality] Lazy app scan failed: {}", e);
        }
    }

    if let Ok(cache) = INSTALLED_APPS.lock() {
        if let Some(ref apps) = *cache {
            let target = app_name.to_lowercase();
            if apps.contains(&target) {
                return Ok(app_name.to_string());
            }
            for installed in apps {
                if installed.contains(&target) || target.contains(installed) {
                    println!(
                        "      ⚠️ [Reality] Fuzzy match: '{}' -> '{}'",
                        app_name, installed
                    );
                    return Ok(installed.clone());
                }
            }

            println!(
                "      ❌ [Reality] REJECTED: App '{}' is not installed.",
                app_name
            );
            return Err(anyhow!(
                "HALLUCINATION DETECTED: Application '{}' is not installed on this machine.",
                app_name
            ));
        }
    }

    println!("❌ [Reality] Inventory unavailable. Failing closed by default.");
    Err(anyhow!(
        "REALITY_CHECK_UNAVAILABLE: app inventory is unavailable; refusing to auto-open '{}'",
        app_name
    ))
}

/// 3. Pre-Flight Check: File Existence
pub fn verify_file_exists(path: &str) -> Result<()> {
    let path_obj = std::path::Path::new(path);
    if path_obj.exists() {
        Ok(())
    } else {
        error!("      ❌ [Reality] REJECTED: File '{}' not found.", path);
        Err(anyhow!(
            "HALLUCINATION DETECTED: File '{}' does not exist.",
            path
        ))
    }
}
