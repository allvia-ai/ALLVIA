use anyhow::{Context, Result};
use std::process::Command;

pub fn run(script: &str) -> Result<String> {
    #[cfg(target_os = "macos")]
    {
        let output = Command::new("osascript")
            .arg("-e")
            .arg(script)
            .output()
            .context("Failed to run AppleScript")?;

        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

        if !output.status.success() {
            return Err(anyhow::anyhow!("AppleScript Error: {}", stderr));
        }

        Ok(stdout)
    }
    #[cfg(not(target_os = "macos"))]
    {
        Ok("AppleScript functionality is only available on macOS.".to_string())
    }
}

pub fn control_app(app: &str, command: &str) -> Result<String> {
    // Template-based control
    let script = match (app.to_lowercase().as_str(), command) {
        ("music", "play") => "tell application \"Music\" to play",
        ("music", "pause") => "tell application \"Music\" to pause",
        ("music", "next") => "tell application \"Music\" to next track",
        ("notes", "new") => "tell application \"Notes\" to make new note at folder \"Notes\"",
        _ => return Err(anyhow::anyhow!("Unknown app control command")),
    };

    run(script)
}

pub fn activate_app(app: &str) -> Result<String> {
    let script = format!(
        r#"
    tell application "{}"
        activate
        set _tries to 0
        repeat while (not frontmost) and _tries < 40
            delay 0.1
            set _tries to _tries + 1
        end repeat
        delay 0.2
    end tell
    "#,
        app
    );
    run(&script)
}

pub fn execute_js_in_chrome(script: &str) -> Result<String> {
    // Pass JS as argv to avoid breaking on quotes/newlines.
    let lines = [
        "on run argv",
        "set js to item 1 of argv",
        "tell application \"Google Chrome\" to execute javascript js in active tab of front window",
        "end run",
    ];
    run_lines_with_args(&lines, &[script.to_string()])
}

pub fn activate_frontmost_app() -> Result<String> {
    let script = r#"
        tell application "System Events"
            set frontApp to name of first application process whose frontmost is true
        end tell
        tell application frontApp to activate
        return frontApp
    "#;
    run(script)
}

pub fn check_accessibility() -> Result<()> {
    let script = r#"tell application "System Events" to get name of first application process whose frontmost is true"#;
    run(script).map(|_| ())
}

pub fn get_active_window_context() -> Result<(String, String)> {
    // Returns (Window Title, Browser URL)
    let script = r#"
        global frontApp, windowTitle, browserUrl
        set windowTitle to ""
        set browserUrl to ""
        
        tell application "System Events"
            set frontApp to name of first application process whose frontmost is true
        end tell

        if frontApp is "Google Chrome" then
            tell application "Google Chrome"
                if (count of windows) > 0 then
                    set windowTitle to title of active tab of front window
                    set browserUrl to URL of active tab of front window
                end if
            end tell
        else if frontApp is "Safari" then
            tell application "Safari"
                if (count of documents) > 0 then
                    set windowTitle to name of front document
                    set browserUrl to URL of front document
                end if
            end tell
        else
            tell application "System Events"
                tell process frontApp
                    if (count of windows) > 0 then
                        set windowTitle to name of front window
                    end if
                end tell
            end tell
        end if
        
        return windowTitle & "|||" & browserUrl
    "#;

    let output = run(script)?;
    let parts: Vec<&str> = output.split("|||").collect();
    let title = parts.get(0).unwrap_or(&"").trim().to_string();
    let url = parts.get(1).unwrap_or(&"").trim().to_string();

    Ok((title, url))
}

fn run_lines_with_args(lines: &[&str], args: &[String]) -> Result<String> {
    #[cfg(target_os = "macos")]
    {
        let mut cmd = Command::new("osascript");
        for line in lines {
            cmd.arg("-e").arg(line);
        }
        cmd.arg("--");
        for arg in args {
            cmd.arg(arg);
        }

        let output = cmd.output().context("Failed to run AppleScript")?;
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

        if !output.status.success() {
            return Err(anyhow::anyhow!("AppleScript Error: {}", stderr));
        }

        Ok(stdout)
    }
    #[cfg(not(target_os = "macos"))]
    {
        Ok("AppleScript functionality is only available on macOS.".to_string())
    }
}

pub fn run_with_args(lines: &[&str], args: &[String]) -> Result<String> {
    run_lines_with_args(lines, args)
}

pub fn open_url(url: &str) -> Result<String> {
    // Smart Open: Detects if Safari or Chrome is frontmost and force-opens there.
    // Otherwise falls back to system default.
    let script = format!(
        r#"
        tell application "System Events"
            set frontApp to name of first application process whose frontmost is true
        end tell
        
        if frontApp is "Safari" then
            tell application "Safari"
                activate
                open location "{}"
            end tell
            return "Opened in Safari"
        else if frontApp is "Google Chrome" then
            tell application "Google Chrome"
                activate
                open location "{}"
            end tell
            return "Opened in Chrome"
        else
            open location "{}"
            return "Opened in Default Browser"
        end if
    "#,
        url, url, url
    );

    run(&script)
}
