//! In-app updates from GitHub Releases.
//!
//! The release workflow publishes `latest.json` next to each release's
//! installer, signed with the project's updater key. We check it shortly
//! after startup and then every few hours; when there's a newer version we
//! ask once (per version, per run) whether to install it. Saying "Later"
//! leaves an "Install update" item in the tray. Installing downloads the
//! installer, verifies its signature against the public key in
//! tauri.conf.json, runs it without questions and restarts the overlay.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use tauri::menu::MenuItem;
use tauri::{AppHandle, Manager, Wry};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::controller::Controller;

/// Give the network a moment after login before the first check.
const FIRST_CHECK_AFTER: Duration = Duration::from_secs(15);
const CHECK_EVERY: Duration = Duration::from_secs(6 * 60 * 60);

pub const CHECK_LABEL: &str = "Check for updates";

/// Managed state: the tray item and what the last check found.
#[derive(Default)]
pub struct Updates {
    item: Mutex<Option<MenuItem<Wry>>>,
    pending: Mutex<Option<Update>>,
    /// The version we already asked about this run, so the periodic check
    /// doesn't ask again every few hours.
    asked: Mutex<Option<String>>,
    busy: AtomicBool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Trigger {
    /// The periodic check: quiet unless there is something to install.
    Background,
    /// The tray item: always says what happened.
    User,
}

impl Updates {
    pub fn attach_tray_item(&self, item: MenuItem<Wry>) {
        *lock(&self.item) = Some(item);
    }

    fn set_label(&self, text: &str, enabled: bool) {
        let item = lock(&self.item).clone();
        if let Some(item) = item {
            let _ = item.set_text(text);
            let _ = item.set_enabled(enabled);
        }
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Starts the periodic check. Only Windows builds are published.
pub fn spawn_background_checks(app: AppHandle) {
    if !cfg!(windows) {
        return;
    }
    let spawned = thread::Builder::new()
        .name("updates".into())
        .spawn(move || {
            thread::sleep(FIRST_CHECK_AFTER);
            loop {
                run(&app, Trigger::Background);
                thread::sleep(CHECK_EVERY);
            }
        });
    if let Err(e) = spawned {
        log::warn!("update checks disabled: {e}");
    }
}

/// The tray item: installs a found update, or checks for one.
pub fn on_tray_click(app: &AppHandle) {
    let app = app.clone();
    // Dialogs block, and the tray handler runs on the main thread.
    thread::spawn(move || run(&app, Trigger::User));
}

fn run(app: &AppHandle, trigger: Trigger) {
    let updates = app.state::<Updates>();
    if updates.busy.swap(true, Ordering::SeqCst) {
        return;
    }
    let pending = lock(&updates.pending).clone();
    let update = match pending {
        // Already found one and the user is asking for it: no need to check.
        Some(update) if trigger == Trigger::User => Some(update),
        _ => check(app, trigger),
    };
    if let Some(update) = update {
        offer(app, update, trigger);
    }
    updates.busy.store(false, Ordering::SeqCst);
}

fn check(app: &AppHandle, trigger: Trigger) -> Option<Update> {
    let updates = app.state::<Updates>();
    if trigger == Trigger::User {
        updates.set_label("Checking for updates…", false);
    }
    let result = app
        .updater()
        .and_then(|u| tauri::async_runtime::block_on(u.check()));
    match result {
        Ok(Some(update)) => {
            log::info!(
                "update available: {} (running {})",
                update.version,
                update.current_version
            );
            updates.set_label(&format!("Install update {}…", update.version), true);
            *lock(&updates.pending) = Some(update.clone());
            Some(update)
        }
        Ok(None) => {
            log::info!("no update; {} is the latest", app.package_info().version);
            updates.set_label(CHECK_LABEL, true);
            if trigger == Trigger::User {
                tell(
                    app,
                    MessageDialogKind::Info,
                    &format!(
                        "You have the latest version ({}).",
                        app.package_info().version
                    ),
                );
            }
            None
        }
        Err(e) => {
            log::warn!("update check failed: {e}");
            updates.set_label(CHECK_LABEL, true);
            if trigger == Trigger::User {
                tell(
                    app,
                    MessageDialogKind::Warning,
                    &format!("Couldn't check for updates.\n\n{e}"),
                );
            }
            None
        }
    }
}

/// Asks whether to install `update`, and installs it if so. The periodic
/// check asks only once per version.
fn offer(app: &AppHandle, update: Update, trigger: Trigger) {
    let updates = app.state::<Updates>();
    if trigger == Trigger::Background {
        let mut asked = lock(&updates.asked);
        if asked.as_deref() == Some(update.version.as_str()) {
            return;
        }
        *asked = Some(update.version.clone());
    }
    let install = app
        .dialog()
        .message(format!(
            "Lyrics Overlay {} is available. You have {}.\n\n\
             Install it now? The lyrics close for a few seconds and come back.",
            update.version, update.current_version
        ))
        .title("Update available")
        .kind(MessageDialogKind::Info)
        .buttons(MessageDialogButtons::OkCancelCustom(
            "Install now".into(),
            "Later".into(),
        ))
        .blocking_show();
    if install {
        self::install(app, update);
    }
}

fn install(app: &AppHandle, update: Update) {
    let updates = app.state::<Updates>();
    updates.set_label(&format!("Installing {}…", update.version), false);
    log::info!("installing update {}", update.version);
    // Settings are saved on change, except a window position from the last
    // half second; on Windows the updater ends the process itself.
    app.state::<Controller>().flush();
    log::logger().flush();
    let result = tauri::async_runtime::block_on(update.download_and_install(|_, _| {}, || {}));
    match result {
        // Windows never gets here: the installer takes over and restarts us.
        Ok(()) => app.restart(),
        Err(e) => {
            log::warn!("update {} failed: {e}", update.version);
            updates.set_label(&format!("Install update {}…", update.version), true);
            tell(
                app,
                MessageDialogKind::Error,
                &format!("Couldn't install the update.\n\n{e}"),
            );
        }
    }
}

fn tell(app: &AppHandle, kind: MessageDialogKind, text: &str) {
    app.dialog()
        .message(text)
        .title("Lyrics Overlay")
        .kind(kind)
        .blocking_show();
}
