use tauri::{
    menu::{Menu, MenuItem},
    tray::{TrayIconBuilder, TrayIconEvent},
    Manager,
};

fn is_local_port_open(port: u16) -> bool {
    use std::net::{SocketAddr, TcpStream};
    use std::time::Duration;

    let addr: SocketAddr = format!("127.0.0.1:{}", port).parse().unwrap();
    TcpStream::connect_timeout(&addr, Duration::from_millis(220)).is_ok()
}

fn bundled_core_path() -> Option<std::path::PathBuf> {
    // In the macOS bundle, both `app` and `core` live under Contents/MacOS/.
    // We resolve relative to the current executable to avoid relying on CWD.
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let core = dir.join("core");
    if core.exists() {
        Some(core)
    } else {
        None
    }
}

fn ensure_core_running() -> Result<(), String> {
    // Avoid spawning duplicate servers if something is already bound to the port.
    if is_local_port_open(5680) {
        return Ok(());
    }

    let core = bundled_core_path().ok_or_else(|| "bundled_core_not_found".to_string())?;

    // Resolve the .env location: prefer the project core/ directory if it exists,
    // otherwise fall back to the binary's own directory.
    let core_dir = core.parent().map(|p| p.to_path_buf());
    let project_core_env = std::env::var("HOME")
        .ok()
        .map(|h| {
            std::path::PathBuf::from(h)
                .join("Desktop/python/github/Allrounder/Steer/local-os-agent/core")
        })
        .filter(|p| p.join(".env").exists());
    let working_dir = project_core_env
        .or(core_dir)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());

    std::process::Command::new(core)
        .current_dir(&working_dir)
        .env("STEER_LAUNCHED_BY_APP", "1")
        .env("STEER_API_ALLOW_NO_KEY", "1")
        .env("STEER_DISABLE_EVENT_TAP", "1")
        .env("STEER_PREFLIGHT_SCREEN_CAPTURE", "0")
        .env("STEER_PREFLIGHT_AX_SNAPSHOT", "0")
        .env("RUST_LOG", "info")
        .env("PATH", "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin")
        .spawn()
        .map_err(|e| format!("failed_to_spawn_core: {}", e))?;

    Ok(())
}

#[tauri::command]
fn open_artifact_path(path: String) -> Result<String, String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err("empty_path".to_string());
    }

    let mut target = std::path::PathBuf::from(trimmed);
    if !target.is_absolute() {
        let cwd = std::env::current_dir().map_err(|e| format!("cwd_error: {}", e))?;
        target = cwd.join(target);
    }

    let resolved = target.canonicalize().unwrap_or(target.clone());
    let metadata = std::fs::metadata(&resolved)
        .map_err(|_| format!("path_not_found: {}", resolved.to_string_lossy()))?;

    let mut cmd = std::process::Command::new("open");
    if metadata.is_file() {
        cmd.arg("-R").arg(&resolved);
    } else {
        cmd.arg(&resolved);
    }

    let status = cmd.status().map_err(|e| format!("open_error: {}", e))?;
    if !status.success() {
        return Err(format!("open_failed: {}", status));
    }

    Ok(resolved.to_string_lossy().to_string())
}

fn position_window_bottom_center(window: &tauri::WebviewWindow) {
    if let (Ok(Some(monitor)), Ok(window_size)) = (window.current_monitor(), window.outer_size()) {
        let monitor_size = monitor.size();
        let x = ((monitor_size.width.saturating_sub(window_size.width)) / 2) as i32;
        let y = monitor_size
            .height
            .saturating_sub(window_size.height)
            .saturating_sub(36) as i32;

        let _ = window.set_position(tauri::Position::Physical(tauri::PhysicalPosition::new(
            x, y,
        )));
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }

            // Best-effort: start the bundled core server when launching the app.
            // If a different core is already running on :5680, we won't override it here.
            if let Err(err) = ensure_core_running() {
                log::warn!("core auto-start skipped: {}", err);
            }

            // Updater is disabled for local builds unless a proper updater config is provided.
            // Initializing updater without config can crash at startup.

            // System Tray Setup
            let quit_i = MenuItem::with_id(app, "quit", "Quit Antigravity", true, None::<&str>)?;
            let show_i = MenuItem::with_id(app, "show", "Show Launcher", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show_i, &quit_i])?;

            let _tray = TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .menu(&menu)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "quit" => {
                        app.exit(0);
                    }
                    "show" => {
                        if let Some(window) = app.get_webview_window("main") {
                            position_window_bottom_center(&window);
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click { .. } = event {
                        let app = tray.app_handle();
                        if let Some(window) = app.get_webview_window("main") {
                            if window.is_visible().unwrap_or(false) {
                                let _ = window.hide();
                            } else {
                                position_window_bottom_center(&window);
                                let _ = window.show();
                                let _ = window.set_focus();
                            }
                        }
                    }
                })
                .build(app)?;

            if let Some(window) = app.get_webview_window("main") {
                position_window_bottom_center(&window);
            }

            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                // Prevent close, hide instead
                window.hide().unwrap();
                api.prevent_close();
            }
        })
        .invoke_handler(tauri::generate_handler![open_artifact_path])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
