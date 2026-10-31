//! The Lyrics Overlay installer, as users see it: one window in the app's
//! own style with an Install button and two options.
//!
//! The installing itself is done by the app's NSIS installer, embedded in
//! this exe at build time and run silently. NSIS keeps doing what it's good
//! at (per-user install, Start menu entry, uninstaller, upgrading a running
//! copy, fetching WebView2); this window only replaces its wizard pages.
//! In-app updates use the NSIS installer directly, without this window.

// No console window in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod install;

use std::cmp::Ordering;
use std::thread;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, WebviewWindow, WebviewWindowBuilder};

use install::Options;

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// What the page needs to pick its button and option defaults.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Info {
    version: &'static str,
    /// The installed version, if any.
    installed: Option<String>,
    /// How this version compares to the installed one: `none`, `older`
    /// (installed is older: an update), `same` or `newer`.
    relation: &'static str,
    defaults: Options,
    /// No installer embedded: the install is simulated.
    demo: bool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Finished {
    ok: bool,
    detail: Option<String>,
}

#[tauri::command]
fn info() -> Info {
    let installed = install::installed();
    let relation = match installed.as_ref().map(|i| compare(&i.version, VERSION)) {
        None => "none",
        Some(Ordering::Less) => "older",
        Some(Ordering::Equal) => "same",
        Some(Ordering::Greater) => "newer",
    };
    Info {
        version: VERSION,
        installed: installed.as_ref().map(|i| i.version.clone()),
        relation,
        defaults: install::current_options(installed.is_some()),
        demo: !install::HAS_PAYLOAD,
    }
}

/// Shows the window once the page has drawn, so it never flashes empty.
#[tauri::command]
fn ready(window: WebviewWindow) {
    let _ = window.show();
    let _ = window.set_focus();
}

/// Installs in the background; the page hears back via `setup-finished`.
#[tauri::command]
fn install(app: AppHandle, options: Options) {
    thread::spawn(move || {
        let result = install::run(&options);
        let finished = match result {
            Ok(()) => Finished {
                ok: true,
                detail: None,
            },
            Err(detail) => Finished {
                ok: false,
                detail: Some(detail),
            },
        };
        let _ = app.emit("setup-finished", finished);
    });
}

/// Starts the installed app and closes the setup.
#[tauri::command]
fn launch(app: AppHandle) -> Result<(), String> {
    install::launch()?;
    app.exit(0);
    Ok(())
}

#[tauri::command]
fn quit(app: AppHandle) {
    app.exit(0);
}

/// Compares dotted version numbers ("0.1.10" > "0.1.9").
fn compare(a: &str, b: &str) -> Ordering {
    let parts = |v: &str| -> Vec<u64> {
        v.split(['.', '-', '+'])
            .take(3)
            .map(|p| p.parse().unwrap_or(0))
            .collect()
    };
    parts(a).cmp(&parts(b))
}

fn main() {
    // Without WebView2 this window can't render; the classic installer can,
    // and it installs WebView2 on the way.
    if install::fall_back_without_webview() {
        return;
    }

    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![info, ready, install, launch, quit])
        .setup(|app| {
            let config = app
                .config()
                .app
                .windows
                .first()
                .cloned()
                .expect("tauri.conf.json defines the setup window");
            // Keep the webview's profile in the temp folder rather than
            // leaving a folder for the setup in the user's app data.
            let window = WebviewWindowBuilder::from_config(app.handle(), &config)?
                .data_directory(std::env::temp_dir().join("LyricsOverlaySetup"))
                .build()?;
            // The page shows the window once it has drawn; if it never
            // says so, show it anyway rather than run invisibly.
            thread::spawn(move || {
                thread::sleep(Duration::from_secs(3));
                let _ = window.show();
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("failed to open the setup window");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_versions_numerically() {
        assert_eq!(compare("0.1.9", "0.1.10"), Ordering::Less);
        assert_eq!(compare("0.2.0", "0.1.10"), Ordering::Greater);
        assert_eq!(compare("0.1.2", "0.1.2"), Ordering::Equal);
        assert_eq!(compare("1.0.0-beta", "1.0.0"), Ordering::Equal);
    }
}
