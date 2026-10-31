//! Owns the settings and applies every change to them, whether it comes from
//! the page, the tray menu or a global shortcut, so all three stay in sync.
//!
//! Locking rule: no mutex here is held while calling into Tauri. Tauri
//! window calls made on the main thread run immediately and can emit window
//! events (e.g. `Moved`) whose handlers call back into this controller.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, mpsc};

use serde::Serialize;
use tauri::menu::CheckMenuItem;
use tauri::{AppHandle, Emitter, LogicalSize, Manager, PhysicalPosition, WebviewWindow, Wry};

use lyrics_core::player::PlayerAction;

use crate::geometry::{self, CssRect, Rect};
use crate::settings::{OFFSET_STEP_MS, Settings, WindowPosition};
use crate::worker::WorkerCommand;

pub const WINDOW_LABEL: &str = "overlay";
pub const SETTINGS_EVENT: &str = "overlay-settings";
pub const POINTER_EVENT: &str = "overlay-pointer";

/// Tray menu entries whose check marks mirror state.
#[derive(Clone)]
pub struct TrayItems {
    pub visible: CheckMenuItem<Wry>,
    pub locked: CheckMenuItem<Wry>,
    pub autostart: CheckMenuItem<Wry>,
}

#[derive(Clone, Serialize)]
struct PointerPayload {
    inside: bool,
}

pub struct Controller {
    app: AppHandle,
    path: PathBuf,
    settings: Mutex<Settings>,
    /// The window moved and the new position isn't saved yet.
    position_dirty: AtomicBool,
    worker: Mutex<Option<mpsc::Sender<WorkerCommand>>>,
    tray: Mutex<Option<TrayItems>>,
    /// The unlock button, as last reported by the page.
    hotspot: Mutex<Option<CssRect>>,
    /// What we last told the window about click-through.
    ignoring_cursor: AtomicBool,
    /// What we last told the page about the pointer (only tracked while locked).
    pointer_inside: AtomicBool,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    // The data stays consistent even if a holder panicked; keep going.
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Controller {
    pub fn new(app: AppHandle, path: PathBuf, settings: Settings) -> Self {
        Self {
            app,
            path,
            settings: Mutex::new(settings),
            position_dirty: AtomicBool::new(false),
            worker: Mutex::new(None),
            tray: Mutex::new(None),
            hotspot: Mutex::new(None),
            ignoring_cursor: AtomicBool::new(false),
            pointer_inside: AtomicBool::new(false),
        }
    }

    pub fn settings(&self) -> Settings {
        lock(&self.settings).clone()
    }

    pub fn attach_worker(&self, tx: mpsc::Sender<WorkerCommand>) {
        *lock(&self.worker) = Some(tx);
    }

    pub fn attach_tray(&self, items: TrayItems) {
        *lock(&self.tray) = Some(items);
    }

    fn window(&self) -> Option<WebviewWindow> {
        self.app.get_webview_window(WINDOW_LABEL)
    }

    /// Applies `change`, saves, and tells the page. Returns the new settings.
    fn update(&self, change: impl FnOnce(&mut Settings)) -> Settings {
        let settings = {
            let mut s = lock(&self.settings);
            change(&mut s);
            s.clone()
        };
        if let Err(e) = settings.save(&self.path) {
            log::warn!("could not save settings to {}: {e}", self.path.display());
        }
        let _ = self.app.emit(SETTINGS_EVENT, settings.clone());
        settings
    }

    fn send_worker(&self, command: WorkerCommand) {
        if let Some(tx) = lock(&self.worker).as_ref() {
            let _ = tx.send(command);
        }
    }

    fn set_ignore_cursor(&self, window: &WebviewWindow, ignore: bool) {
        if window.set_ignore_cursor_events(ignore).is_ok() {
            self.ignoring_cursor.store(ignore, Ordering::SeqCst);
        }
    }

    fn sync_tray(&self, settings: &Settings) {
        let items = lock(&self.tray).clone();
        if let Some(items) = items {
            let _ = items.visible.set_checked(settings.visible);
            let _ = items.locked.set_checked(settings.locked);
        }
    }

    // ---- Actions -----------------------------------------------------------

    pub fn set_locked(&self, locked: bool) {
        let settings = self.update(|s| s.locked = locked);
        if let Some(window) = self.window() {
            self.set_ignore_cursor(&window, locked);
        }
        self.sync_tray(&settings);
    }

    pub fn toggle_locked(&self) {
        self.set_locked(!self.settings().locked);
    }

    pub fn set_visible(&self, visible: bool) {
        let settings = self.update(|s| s.visible = visible);
        if let Some(window) = self.window() {
            let _ = if visible {
                window.show()
            } else {
                window.hide()
            };
        }
        self.sync_tray(&settings);
    }

    pub fn toggle_visible(&self) {
        self.set_visible(!self.settings().visible);
    }

    pub fn step_font(&self, steps: i32) {
        let before = self.settings().font_scale;
        let after = self.update(|s| s.step_font(steps)).font_scale;
        if after != before {
            if let Some(window) = self.window() {
                if let Err(e) = resize_for_font(&window, before, after) {
                    log::warn!("could not resize the window: {e}");
                }
            }
        }
    }

    pub fn cycle_theme(&self) {
        self.update(|s| s.theme = s.theme.next());
    }

    pub fn step_offset(&self, steps: i64) {
        let settings = self.update(|s| s.adjust_offset(steps * OFFSET_STEP_MS));
        self.send_worker(WorkerCommand::SetOffset(settings.offset_ms));
    }

    pub fn reset_offset(&self) {
        let settings = self.update(|s| s.offset_ms = 0);
        self.send_worker(WorkerCommand::SetOffset(settings.offset_ms));
    }

    pub fn control(&self, action: PlayerAction) {
        self.send_worker(WorkerCommand::Control(action));
    }

    /// Looks up the current track's lyrics again, bypassing the cache.
    pub fn refresh_lyrics(&self) {
        self.send_worker(WorkerCommand::Refresh);
    }

    /// Whether the app is registered to start at login. The OS registration
    /// (on Windows, the Run registry key) is the source of truth.
    pub fn autostart_enabled(&self) -> bool {
        use tauri_plugin_autostart::ManagerExt;
        self.app.autolaunch().is_enabled().unwrap_or(false)
    }

    pub fn toggle_autostart(&self) {
        use tauri_plugin_autostart::ManagerExt;
        let launcher = self.app.autolaunch();
        let result = if self.autostart_enabled() {
            launcher.disable()
        } else {
            launcher.enable()
        };
        if let Err(e) = result {
            log::warn!("could not change start-at-login: {e}");
        }
        let enabled = self.autostart_enabled();
        log::info!("start at login: {enabled}");
        let items = lock(&self.tray).clone();
        if let Some(items) = items {
            let _ = items.autostart.set_checked(enabled);
        }
    }

    /// Opens the folder holding overlay.log in the file manager.
    pub fn open_log_folder(&self) {
        let Ok(dir) = self.app.path().app_log_dir() else {
            return;
        };
        let _ = std::fs::create_dir_all(&dir);
        let opener = if cfg!(windows) {
            "explorer"
        } else if cfg!(target_os = "macos") {
            "open"
        } else {
            "xdg-open"
        };
        if let Err(e) = std::process::Command::new(opener).arg(&dir).spawn() {
            log::warn!("could not open {}: {e}", dir.display());
        }
    }

    pub fn set_hotspot(&self, rect: Option<CssRect>) {
        *lock(&self.hotspot) = rect;
    }

    /// Records a new window position; saved by [`Controller::flush`].
    pub fn window_moved(&self, pos: PhysicalPosition<i32>) {
        lock(&self.settings).position = Some(WindowPosition { x: pos.x, y: pos.y });
        self.position_dirty.store(true, Ordering::SeqCst);
    }

    /// Saves a pending position change. Called periodically, so dragging
    /// doesn't write the file on every pixel.
    pub fn flush(&self) {
        if self.position_dirty.swap(false, Ordering::SeqCst) {
            if let Err(e) = self.settings().save(&self.path) {
                log::warn!("could not save settings: {e}");
            }
        }
    }

    /// While locked the window ignores the mouse, except over the unlock
    /// button: this polls the cursor (the window gets no mouse events to
    /// tell us) and lets clicks through everywhere else. Also tells the page
    /// when the pointer is over the window, so it can reveal the button.
    pub fn pointer_tick(&self) {
        let settings = self.settings();
        let Some(window) = self.window() else { return };

        if !settings.locked || !settings.visible {
            // Also repairs a race with set_locked(false) on another thread.
            if self.ignoring_cursor.load(Ordering::SeqCst) != settings.locked {
                self.set_ignore_cursor(&window, settings.locked);
            }
            self.report_pointer(false);
            return;
        }

        let (Ok(cursor), Ok(pos), Ok(size), Ok(scale)) = (
            self.app.cursor_position(),
            window.inner_position(),
            window.inner_size(),
            window.scale_factor(),
        ) else {
            return;
        };
        let rect = Rect {
            x: f64::from(pos.x),
            y: f64::from(pos.y),
            width: f64::from(size.width),
            height: f64::from(size.height),
        };
        let hotspot = *lock(&self.hotspot);
        let (inside, over_button) = geometry::hit_test((cursor.x, cursor.y), rect, scale, hotspot);

        let ignore = !over_button;
        if self.ignoring_cursor.load(Ordering::SeqCst) != ignore {
            self.set_ignore_cursor(&window, ignore);
        }
        self.report_pointer(inside);
    }

    fn report_pointer(&self, inside: bool) {
        if self.pointer_inside.swap(inside, Ordering::SeqCst) != inside {
            let _ = self.app.emit(POINTER_EVENT, PointerPayload { inside });
        }
    }

    /// Size, position, click-through and visibility from saved settings.
    pub fn restore_window(&self, window: &WebviewWindow) -> tauri::Result<()> {
        let settings = self.settings();
        let (w, h) = geometry::logical_size(settings.font_scale);
        window.set_size(LogicalSize::new(w, h))?;

        let monitor = window.primary_monitor()?;
        let scale = monitor.as_ref().map_or(1.0, |m| m.scale_factor());
        let physical = (w * scale, h * scale);

        let monitors: Vec<Rect> = window
            .available_monitors()?
            .iter()
            .map(|m| Rect {
                x: f64::from(m.position().x),
                y: f64::from(m.position().y),
                width: f64::from(m.size().width),
                height: f64::from(m.size().height),
            })
            .collect();
        let saved = settings
            .position
            .map(|p| (p.x, p.y))
            .filter(|p| geometry::is_on_some_monitor(&monitors, *p, physical));
        let position = match (saved, monitor) {
            (Some(p), _) => Some(p),
            (None, Some(m)) => {
                let area = m.work_area();
                let area = Rect {
                    x: f64::from(area.position.x),
                    y: f64::from(area.position.y),
                    width: f64::from(area.size.width),
                    height: f64::from(area.size.height),
                };
                Some(geometry::bottom_center(area, physical, scale))
            }
            (None, None) => None,
        };
        if let Some((x, y)) = position {
            window.set_position(PhysicalPosition::new(x, y))?;
        }

        self.set_ignore_cursor(window, settings.locked);
        if settings.visible {
            window.show()?;
        }
        Ok(())
    }
}

/// Resizes for a new font scale, growing upward from the bottom edge.
fn resize_for_font(window: &WebviewWindow, before: f64, after: f64) -> tauri::Result<()> {
    let scale = window.scale_factor()?;
    // Anchor on the window's actual size; the window manager may not have
    // honored the computed one exactly (GTK adds height, for one).
    let actual = window.outer_size()?;
    let old = if actual.width > 0 && actual.height > 0 {
        (f64::from(actual.width), f64::from(actual.height))
    } else {
        let (w, h) = geometry::logical_size(before);
        (w * scale, h * scale)
    };
    let new = geometry::logical_size(after);
    let pos = window.outer_position()?;
    let (x, y) = geometry::resize_anchored((pos.x, pos.y), old, (new.0 * scale, new.1 * scale));
    window.set_size(LogicalSize::new(new.0, new.1))?;
    window.set_position(PhysicalPosition::new(x, y))
}
