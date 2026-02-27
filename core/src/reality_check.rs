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
        if let Some(name) = line.split('/').next_back() {
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

fn normalize_app_name(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_lowercase()
}

fn target_aliases(target_norm: &str) -> Vec<String> {
    let mut aliases = vec![target_norm.to_string()];
    match target_norm {
        "textedit" | "texteditor" => aliases.push("textedit".to_string()),
        "googlechrome" | "chrome" => aliases.push("googlechrome".to_string()),
        "chatgptatlas" | "atlas" => aliases.push("chatgptatlas".to_string()),
        _ => {}
    }
    aliases
}

/// 2. Pre-Flight Check: App Existence
pub fn verify_app_exists(app_name: &str) -> Result<String> {
    let app_name = app_name.trim();
    if app_name.is_empty() {
        return Err(anyhow!("REALITY_CHECK_INVALID_INPUT: app name is empty"));
    }

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
            let target_norm = normalize_app_name(&target);
            let aliases = target_aliases(&target_norm);

            if apps.contains(&target) {
                return Ok(app_name.to_string());
            }
            for installed in apps {
                let installed_norm = normalize_app_name(installed);
                if aliases.iter().any(|alias| alias == &installed_norm) {
                    println!(
                        "      ℹ️ [Reality] Normalized match: '{}' -> '{}'",
                        app_name, installed
                    );
                    return Ok(installed.clone());
                }
            }
            for installed in apps {
                if target.len() >= 5
                    && ((installed.starts_with(&target))
                        || (target.starts_with(installed) && installed.len() >= 5))
                {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    lazy_static! {
        static ref TEST_MUTEX: Mutex<()> = Mutex::new(());
    }

    fn set_cache(apps: &[&str]) {
        if let Ok(mut cache) = INSTALLED_APPS.lock() {
            let mut set = HashSet::new();
            for app in apps {
                set.insert(app.to_lowercase());
            }
            *cache = Some(set);
        }
    }

    #[test]
    fn verify_app_exists_uses_normalized_exact_match() {
        let _guard = TEST_MUTEX.lock().expect("test mutex");
        set_cache(&["google chrome", "textedit"]);
        let result = verify_app_exists("GoogleChrome");
        assert!(result.is_ok());
    }

    #[test]
    fn verify_app_exists_rejects_short_substring_false_positive() {
        let _guard = TEST_MUTEX.lock().expect("test mutex");
        set_cache(&["calendar", "mail"]);
        let result = verify_app_exists("cal");
        assert!(result.is_err());
    }
}
