//! System-wide shortcuts, so a locked or hidden overlay can always be
//! brought back without finding the tray icon.

use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

use crate::controller::Controller;

/// Registers Ctrl+Alt+L (lock) and Ctrl+Alt+D (show/hide). Failures, e.g. a
/// shortcut already taken by another app, are reported and skipped: the
/// overlay works without them.
pub fn install(app: &AppHandle) {
    let lock = Shortcut::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::KeyL);
    let show = Shortcut::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::KeyD);

    let plugin = tauri_plugin_global_shortcut::Builder::new()
        .with_handler(move |app, shortcut, event| {
            if event.state() != ShortcutState::Pressed {
                return;
            }
            let controller = app.state::<Controller>();
            if *shortcut == lock {
                controller.toggle_locked();
            } else if *shortcut == show {
                controller.toggle_visible();
            }
        })
        .build();
    if let Err(e) = app.plugin(plugin) {
        log::warn!("global shortcuts unavailable: {e}");
        return;
    }
    for shortcut in [lock, show] {
        if let Err(e) = app.global_shortcut().register(shortcut) {
            log::warn!("could not register shortcut {shortcut:?}: {e}");
        }
    }
}
