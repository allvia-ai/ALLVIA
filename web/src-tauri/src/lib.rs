use tauri::{
  menu::{Menu, MenuItem},
  tray::{TrayIconBuilder, TrayIconEvent},
  Manager,
};

fn position_window_bottom_center(window: &tauri::WebviewWindow) {
  if let (Ok(Some(monitor)), Ok(window_size)) = (window.current_monitor(), window.outer_size()) {
    let monitor_size = monitor.size();
    let x = ((monitor_size.width.saturating_sub(window_size.width)) / 2) as i32;
    let y = monitor_size
      .height
      .saturating_sub(window_size.height)
      .saturating_sub(36) as i32;

    let _ = window.set_position(tauri::Position::Physical(tauri::PhysicalPosition::new(x, y)));
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

      // Updater is disabled for local builds unless a proper updater config is provided.
      // Initializing updater without config can crash at startup.

      // System Tray Setup
      let quit_i = MenuItem::with_id(app, "quit", "Quit Antigravity", true, None::<&str>)?;
      let show_i = MenuItem::with_id(app, "show", "Show Launcher", true, None::<&str>)?;
      let menu = Menu::with_items(app, &[&show_i, &quit_i])?;

      let _tray = TrayIconBuilder::new()
        .icon(app.default_window_icon().unwrap().clone())
        .menu(&menu)
        .on_menu_event(|app, event| {
            match event.id.as_ref() {
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
            }
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
    .run(tauri::generate_context!())
    .expect("error while running tauri application");
}
