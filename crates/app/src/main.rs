//! Lyrics Overlay: a transparent, always-on-top window showing the lyrics of
//! whatever the Spotify desktop app is playing.
//!
//! `--demo` plays a built-in fake track instead (always on non-Windows).

// No console window in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod controller;
mod demo;
mod geometry;
mod logging;
mod presence;
mod settings;
mod shortcuts;
mod tray;
mod updates;
mod view;
mod worker;

use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Manager, State, WindowEvent};

use lyrics_core::player::PlayerAction;

use controller::{Controller, WINDOW_LABEL};
use geometry::CssRect;
use settings::Settings;
use view::{OverlayState, PlaybackView};

/// Everything the page needs on (re)load, before events start arriving.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PageInit {
    state: OverlayState,
    playback: Option<PlaybackView>,
    settings: Settings,
}

#[tauri::command]
fn get_state(shared: State<'_, worker::Shared>, controller: State<'_, Controller>) -> PageInit {
    let initial = shared.current();
    PageInit {
        state: initial.state,
        playback: initial.playback,
        settings: controller.settings(),
    }
}

#[tauri::command]
fn set_locked(controller: State<'_, Controller>, locked: bool) {
    controller.set_locked(locked);
}

#[tauri::command]
fn step_font(controller: State<'_, Controller>, steps: i32) {
    controller.step_font(steps);
}

#[tauri::command]
fn cycle_theme(controller: State<'_, Controller>) {
    controller.cycle_theme();
}

#[tauri::command]
fn step_offset(controller: State<'_, Controller>, steps: i64) {
    controller.step_offset(steps);
}

#[tauri::command]
fn reset_offset(controller: State<'_, Controller>) {
    controller.reset_offset();
}

#[tauri::command]
fn player_control(controller: State<'_, Controller>, action: PlayerAction) {
    controller.control(action);
}

/// Where the page drew the unlock button (CSS px), or `None` when hidden.
#[tauri::command]
fn set_unlock_hotspot(controller: State<'_, Controller>, rect: Option<CssRect>) {
    controller.set_hotspot(rect);
}

#[tauri::command]
fn quit_app(app: AppHandle) {
    quit(&app);
}

pub fn quit(app: &AppHandle) {
    app.state::<Controller>().flush();
    app.exit(0);
}

/// Showing and hiding the window, cursor tracking for the locked window, and
/// periodic saving of the window position.
fn spawn_housekeeping(app: AppHandle) -> std::io::Result<()> {
    thread::Builder::new()
        .name("housekeeping".into())
        .spawn(move || {
            let mut last_flush = Instant::now();
            loop {
                let controller = app.state::<Controller>();
                controller.presence_tick();
                controller.pointer_tick();
                if last_flush.elapsed() >= Duration::from_millis(500) {
                    controller.flush();
                    last_flush = Instant::now();
                }
                thread::sleep(Duration::from_millis(50));
            }
        })?;
    Ok(())
}

/// Passed by the start-at-login entry.
const AT_LOGIN_ARG: &str = "--autostart";

fn main() {
    let demo = cfg!(not(windows)) || std::env::args().any(|a| a == "--demo");
    // Started by "Start with Windows" rather than by the user.
    let at_login = std::env::args().any(|a| a == AT_LOGIN_ARG);

    tauri::Builder::default()
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec![AT_LOGIN_ARG]),
        ))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(worker::Shared::default())
        .manage(updates::Updates::default())
        .invoke_handler(tauri::generate_handler![
            get_state,
            set_locked,
            step_font,
            cycle_theme,
            step_offset,
            reset_offset,
            player_control,
            set_unlock_hotspot,
            quit_app,
        ])
        .setup(move |app| {
            if let Ok(dir) = app.path().app_log_dir() {
                logging::init(&dir);
            }
            log::info!(
                "Lyrics Overlay {} starting{}{}",
                env!("CARGO_PKG_VERSION"),
                if demo { " in demo mode" } else { "" },
                if at_login { " at login" } else { "" }
            );
            let path = app.path().app_config_dir()?.join("settings.json");
            let settings = Settings::load(&path);
            app.manage(Controller::new(
                app.handle().clone(),
                path,
                settings.clone(),
            ));
            let controller = app.state::<Controller>();

            let window = app
                .get_webview_window(WINDOW_LABEL)
                .expect("tauri.conf.json defines the overlay window");
            let handle = app.handle().clone();
            window.on_window_event(move |event| {
                if let WindowEvent::Moved(pos) = event {
                    handle.state::<Controller>().window_moved(*pos);
                }
            });
            controller.restore_window(&window)?;
            // A launch at login stays out of sight until Spotify plays; one
            // the user started shows itself briefly so they see it worked.
            if !at_login {
                controller.peek();
            }

            match tray::build(app, &settings) {
                Ok(items) => controller.attach_tray(items),
                Err(e) => log::warn!("tray icon unavailable: {e}"),
            }
            shortcuts::install(app.handle());

            let (tx, rx) = mpsc::channel();
            controller.attach_worker(tx);
            let cache = app
                .path()
                .app_cache_dir()
                .ok()
                .map(|d| d.join("lyrics.sqlite3"));
            let backup = controller.backup_lyrics_flag();
            worker::spawn(
                app.handle().clone(),
                demo,
                cache,
                backup,
                settings.offset_ms,
                rx,
            )?;
            spawn_housekeeping(app.handle().clone())?;
            updates::spawn_background_checks(app.handle().clone());
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("failed to start the overlay");
}
