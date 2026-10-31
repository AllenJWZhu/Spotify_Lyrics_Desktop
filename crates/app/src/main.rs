//! Lyrics Overlay: a transparent, always-on-top window showing the lyrics of
//! whatever the Spotify desktop app is playing.
//!
//! `--demo` plays a built-in fake track instead (always on non-Windows).

// No console window in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod demo;
mod view;
mod worker;

use tauri::{LogicalSize, Manager, PhysicalPosition, WebviewWindow};

/// Latest state and playback, so a (re)loaded page can render immediately.
#[tauri::command]
fn get_state(shared: tauri::State<'_, worker::Shared>) -> worker::Initial {
    shared.current()
}

#[tauri::command]
fn quit(app: tauri::AppHandle) {
    app.exit(0);
}

/// Bottom center of the primary monitor's work area (above the taskbar).
fn place_bottom_center(window: &WebviewWindow) -> tauri::Result<()> {
    let Some(monitor) = window.primary_monitor()? else {
        return Ok(());
    };
    let area = monitor.work_area();
    // A window that hasn't been shown yet may report 0×0 (seen on GTK);
    // fall back to the size configured in tauri.conf.json.
    let mut size = window.outer_size()?;
    if size.width == 0 || size.height == 0 {
        if let Some(config) = window.config().app.windows.first() {
            size =
                LogicalSize::new(config.width, config.height).to_physical(monitor.scale_factor());
        }
    }
    let margin = (40.0 * monitor.scale_factor()) as i32;
    let x = area.position.x + (area.size.width as i32 - size.width as i32) / 2;
    let y = area.position.y + area.size.height as i32 - size.height as i32 - margin;
    window.set_position(PhysicalPosition::new(x, y))
}

fn main() {
    let demo = cfg!(not(windows)) || std::env::args().any(|a| a == "--demo");

    tauri::Builder::default()
        .manage(worker::Shared::default())
        .invoke_handler(tauri::generate_handler![get_state, quit])
        .setup(move |app| {
            let window = app
                .get_webview_window("overlay")
                .expect("tauri.conf.json defines the overlay window");
            place_bottom_center(&window)?;
            window.show()?;
            worker::spawn(app.handle().clone(), demo)?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("failed to start the overlay");
}
