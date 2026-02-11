use anyhow::{anyhow, Result};
use log::{error, info};
use std::collections::HashSet;
use std::process::Command;

/// Global cache of installed applications (loaded once at startup)
pub static mut INSTALLED_APPS: Option<HashSet<String>> = None;

fn env_truthy(name: &str) -> bool {
    matches!(
        std::env::var(name).as_deref(),
        Ok("1") | Ok("true") | Ok("TRUE") | Ok("yes") | Ok("YES")
    )
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

    unsafe {
        println!(
            "✅ [Reality] Inventory Complete. Found {} apps.",
            apps.len()
        );
        // Debug print a few apps
        let sample: Vec<_> = apps.iter().take(5).collect();
        println!("   Sample: {:?}", sample);

        INSTALLED_APPS = Some(apps);
    }

    Ok(())
}

/// 2. Pre-Flight Check: App Existence
pub fn verify_app_exists(app_name: &str) -> Result<String> {
    unsafe {
        if INSTALLED_APPS.is_none() {
            println!("⚠️ [Reality] Inventory is NONE. Attempting lazy app scan...");
            if let Err(e) = scan_app_inventory() {
                println!("⚠️ [Reality] Lazy app scan failed: {}", e);
            }
        }

        if let Some(ref apps) = INSTALLED_APPS {
            let target = app_name.to_lowercase();
            // 1. Exact match
            if apps.contains(&target) {
                return Ok(app_name.to_string());
            }
            // 2. Partial match (e.g. "Microsoft Excel" vs "Excel")
            // Iterate to find the *best* match (shortest string that contains target?)
            // For now, first match.
            for installed in apps {
                if installed.contains(&target) || target.contains(installed) {
                    println!(
                        "      ⚠️ [Reality] Fuzzy match: '{}' -> '{}'",
                        app_name, installed
                    );
                    return Ok(installed.clone()); // Return the actual installed name
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
        } else {
            if env_truthy("STEER_REALITY_FAIL_OPEN") {
                println!("⚠️ [Reality] Inventory is NONE. Failing Open due to STEER_REALITY_FAIL_OPEN=1.");
                return Ok(app_name.to_string());
            }
            println!("❌ [Reality] Inventory unavailable. Failing closed by default.");
            return Err(anyhow!(
                "REALITY_CHECK_UNAVAILABLE: app inventory is unavailable; refusing to auto-open '{}'",
                app_name
            ));
        }
    }
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
