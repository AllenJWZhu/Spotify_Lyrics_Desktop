//! Keeping the overlay above other windows.
//!
//! "Always on top" is applied once, when the window is created, and Windows
//! stacks topmost windows in the order they last asked to be on top. A video
//! player or browser that makes its fullscreen window topmost therefore lands
//! above the lyrics, and they seem to vanish until something else is clicked.
//! So the overlay asks again: whenever another window comes to the front, and
//! once a second while it's on screen. The request never takes focus.
//!
//! A game in *exclusive* fullscreen still covers the lyrics: it owns the
//! screen, and no ordinary window can draw over it. Borderless (windowed)
//! fullscreen is fine.

use std::time::{Duration, Instant};

use tauri::WebviewWindow;

/// How often to re-assert even when nothing seems to have changed: some
/// apps go topmost without coming to the front.
const EVERY: Duration = Duration::from_secs(1);

/// What the housekeeping thread remembers between ticks.
#[derive(Default)]
pub struct Topmost {
    /// The foreground window at the last tick (0: none or unknown).
    foreground: isize,
    last: Option<Instant>,
}

impl Topmost {
    /// Whether to re-assert now, given the current foreground window.
    fn due(&mut self, foreground: isize, now: Instant) -> bool {
        let changed = foreground != self.foreground;
        self.foreground = foreground;
        let due = changed || self.last.is_none_or(|t| now.duration_since(t) >= EVERY);
        if due {
            self.last = Some(now);
        }
        due
    }

    /// Call while the window is on screen.
    pub fn tick(&mut self, window: &WebviewWindow, now: Instant) {
        let Some(ours) = platform::handle(window) else {
            return;
        };
        let foreground = platform::foreground();
        let changed = foreground != self.foreground;
        if !self.due(foreground, now) {
            return;
        }
        if changed && foreground != ours {
            // Says what took the screen, to make "the lyrics vanished in
            // fullscreen" reports traceable.
            if let Some(what) = platform::fullscreen(foreground) {
                log::info!("fullscreen window in front ({what}); keeping the lyrics above it");
            }
        }
        if !platform::raise(ours) {
            log::warn!("could not keep the lyrics on top");
        }
    }

    /// Call while the window is hidden, so showing it re-asserts at once.
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

#[cfg(windows)]
mod platform {
    use tauri::WebviewWindow;
    use windows_sys::Win32::Foundation::{HWND, RECT};
    use windows_sys::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GWL_EXSTYLE, GetClassNameW, GetForegroundWindow, GetWindowLongW, GetWindowRect,
        HWND_TOPMOST, IsWindowVisible, SWP_ASYNCWINDOWPOS, SWP_NOACTIVATE, SWP_NOMOVE,
        SWP_NOOWNERZORDER, SWP_NOSIZE, SetWindowPos, WS_EX_TOPMOST,
    };

    pub fn handle(window: &WebviewWindow) -> Option<isize> {
        window.hwnd().ok().map(|h| h.0 as isize)
    }

    pub fn foreground() -> isize {
        // SAFETY: no arguments; returns null when there is none.
        unsafe { GetForegroundWindow() as isize }
    }

    pub fn raise(ours: isize) -> bool {
        // Async: the window belongs to the main thread; don't wait on it.
        let flags =
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER | SWP_ASYNCWINDOWPOS;
        // SAFETY: a window handle we own; a stale one just fails.
        unsafe { SetWindowPos(ours as HWND, HWND_TOPMOST, 0, 0, 0, 0, flags) != 0 }
    }

    /// The window's class, and whether it's topmost, if it covers its
    /// whole monitor. The desktop itself doesn't count.
    pub fn fullscreen(window: isize) -> Option<String> {
        let hwnd = window as HWND;
        if hwnd.is_null() {
            return None;
        }
        // SAFETY: plain queries on a window handle; all fail cleanly if the
        // window has gone, and every out-pointer is a local of the right size.
        unsafe {
            if IsWindowVisible(hwnd) == 0 {
                return None;
            }
            let mut rect = RECT {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            };
            if GetWindowRect(hwnd, &mut rect) == 0 {
                return None;
            }
            let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
            let mut info: MONITORINFO = std::mem::zeroed();
            info.cbSize = size_of::<MONITORINFO>() as u32;
            if GetMonitorInfoW(monitor, &mut info) == 0 {
                return None;
            }
            let m = info.rcMonitor;
            let covers = rect.left <= m.left
                && rect.top <= m.top
                && rect.right >= m.right
                && rect.bottom >= m.bottom;
            if !covers {
                return None;
            }
            let mut buf = [0u16; 128];
            let len = GetClassNameW(hwnd, buf.as_mut_ptr(), buf.len() as i32).max(0) as usize;
            let class = String::from_utf16_lossy(&buf[..len]);
            if matches!(class.as_str(), "Progman" | "WorkerW") {
                return None;
            }
            let topmost = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST != 0;
            Some(format!("class {class:?}, topmost: {topmost}"))
        }
    }
}

#[cfg(not(windows))]
mod platform {
    use tauri::WebviewWindow;

    pub fn handle(_: &WebviewWindow) -> Option<isize> {
        None
    }

    pub fn foreground() -> isize {
        0
    }

    pub fn raise(_: isize) -> bool {
        true
    }

    pub fn fullscreen(_: isize) -> Option<String> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asks_at_once_then_on_a_new_foreground_window_or_after_a_second() {
        let start = Instant::now();
        let mut t = Topmost::default();
        assert!(t.due(1, start), "first tick after showing");
        assert!(!t.due(1, start + Duration::from_millis(50)));
        assert!(
            t.due(2, start + Duration::from_millis(100)),
            "new foreground"
        );
        assert!(!t.due(2, start + Duration::from_millis(500)));
        assert!(
            t.due(2, start + Duration::from_millis(1100)),
            "a second later"
        );
        assert!(!t.due(2, start + Duration::from_millis(1150)));
    }

    #[test]
    fn showing_again_asks_at_once() {
        let start = Instant::now();
        let mut t = Topmost::default();
        assert!(t.due(1, start));
        t.reset();
        assert!(t.due(1, start + Duration::from_millis(50)));
    }
}
