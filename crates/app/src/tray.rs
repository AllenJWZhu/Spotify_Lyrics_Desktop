//! Notification-area icon. Its menu mirrors the two settings that matter
//! when the window can't be clicked (locked) or can't be seen (hidden).

use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{App, Manager};

use crate::controller::{Controller, TrayItems};
use crate::settings::Settings;

pub fn build(app: &App, settings: &Settings) -> tauri::Result<TrayItems> {
    let visible = CheckMenuItem::with_id(
        app,
        "visible",
        "Show lyrics",
        true,
        settings.visible,
        Some("Ctrl+Alt+D"),
    )?;
    let locked = CheckMenuItem::with_id(
        app,
        "locked",
        "Lock lyrics (click-through)",
        true,
        settings.locked,
        Some("Ctrl+Alt+L"),
    )?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&visible, &locked, &separator, &quit])?;

    let mut builder = TrayIconBuilder::with_id("main")
        .tooltip("Lyrics Overlay")
        .menu(&menu)
        .on_menu_event(|app, event| {
            let controller = app.state::<Controller>();
            match event.id().as_ref() {
                "visible" => controller.toggle_visible(),
                "locked" => controller.toggle_locked(),
                "quit" => crate::quit(app),
                _ => {}
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;

    Ok(TrayItems { visible, locked })
}
