//! Window geometry as plain arithmetic, kept apart from Tauri so it can be
//! tested: sizing by font scale, placement, and hit-testing the cursor.
//! All screen coordinates are physical pixels.

use serde::Deserialize;

/// Height of the toolbar strip at the top of the window (logical px). The
/// page reserves the same space (`--toolbar-height` in style.css).
pub const TOOLBAR_HEIGHT: f64 = 36.0;
/// Lyrics area at font scale 1.0 (logical px).
const BASE_LYRICS_HEIGHT: f64 = 140.0;
const BASE_WIDTH: f64 = 960.0;
/// Gap between the window and the bottom of the work area (logical px).
const BOTTOM_MARGIN: f64 = 32.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    pub fn contains(&self, px: f64, py: f64) -> bool {
        px >= self.x && px < self.x + self.width && py >= self.y && py < self.y + self.height
    }
}

/// A rectangle in the page's CSS pixels, relative to the window's top-left.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub struct CssRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Logical window size for a font scale. Text grows with the scale; the
/// toolbar doesn't. Width only grows, so small fonts keep a wide window
/// for long lines.
pub fn logical_size(font_scale: f64) -> (f64, f64) {
    (
        (BASE_WIDTH * font_scale.max(1.0)).round(),
        (TOOLBAR_HEIGHT + BASE_LYRICS_HEIGHT * font_scale).round(),
    )
}

/// Top-left for a window of `size` centered at the bottom of `work_area`.
pub fn bottom_center(work_area: Rect, size: (f64, f64), scale_factor: f64) -> (i32, i32) {
    let x = work_area.x + (work_area.width - size.0) / 2.0;
    let y = work_area.y + work_area.height - size.1 - BOTTOM_MARGIN * scale_factor;
    (x.round() as i32, y.round() as i32)
}

/// New top-left after resizing from `old_size` to `new_size`, keeping the
/// bottom edge and the horizontal center in place (the overlay usually sits
/// near the bottom of the screen and should grow upward).
pub fn resize_anchored(
    old_pos: (i32, i32),
    old_size: (f64, f64),
    new_size: (f64, f64),
) -> (i32, i32) {
    let x = f64::from(old_pos.0) + (old_size.0 - new_size.0) / 2.0;
    let y = f64::from(old_pos.1) + old_size.1 - new_size.1;
    (x.round() as i32, y.round() as i32)
}

/// Whether a window at `pos` with `size` would be reachable: its center lies
/// on one of the monitors. Guards against restoring a position saved on a
/// monitor that has since been disconnected.
pub fn is_on_some_monitor(monitors: &[Rect], pos: (i32, i32), size: (f64, f64)) -> bool {
    let cx = f64::from(pos.0) + size.0 / 2.0;
    let cy = f64::from(pos.1) + size.1 / 2.0;
    monitors.iter().any(|m| m.contains(cx, cy))
}

/// Where the cursor is relative to the window: inside it at all, and over
/// the unlock button.
pub fn hit_test(
    cursor: (f64, f64),
    window: Rect,
    scale_factor: f64,
    hotspot: Option<CssRect>,
) -> (bool, bool) {
    let inside = window.contains(cursor.0, cursor.1);
    let over_hotspot = hotspot.is_some_and(|h| {
        Rect {
            x: window.x + h.x * scale_factor,
            y: window.y + h.y * scale_factor,
            width: h.width * scale_factor,
            height: h.height * scale_factor,
        }
        .contains(cursor.0, cursor.1)
    });
    (inside, inside && over_hotspot)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: Rect = Rect {
        x: 0.0,
        y: 0.0,
        width: 1920.0,
        height: 1040.0, // 1080 minus a 40 px taskbar
    };

    #[test]
    fn size_grows_with_font_but_width_never_shrinks() {
        assert_eq!(logical_size(1.0), (960.0, 176.0));
        assert_eq!(logical_size(1.5), (1440.0, 246.0));
        assert_eq!(logical_size(0.6), (960.0, 120.0));
    }

    #[test]
    fn bottom_center_respects_work_area_and_dpi() {
        assert_eq!(bottom_center(SCREEN, (960.0, 176.0), 1.0), (480, 832));
        // 150% scaling: physical size and margin both scale.
        assert_eq!(bottom_center(SCREEN, (1440.0, 264.0), 1.5), (240, 728));
        // Secondary monitor to the left of the primary.
        let left = Rect {
            x: -1280.0,
            y: 0.0,
            width: 1280.0,
            height: 1024.0,
        };
        assert_eq!(bottom_center(left, (960.0, 176.0), 1.0).0, -1120);
    }

    #[test]
    fn resizing_keeps_bottom_edge_and_center() {
        let old_pos = (480, 832);
        let new = resize_anchored(old_pos, (960.0, 176.0), (1056.0, 190.0));
        assert_eq!(new, (432, 818));
        // Bottom edge: 832 + 176 == 818 + 190.
        assert_eq!(old_pos.1 + 176, new.1 + 190);
    }

    #[test]
    fn saved_positions_must_be_on_a_monitor() {
        let monitors = [SCREEN];
        assert!(is_on_some_monitor(&monitors, (480, 832), (960.0, 176.0)));
        // Saved on a monitor that's gone.
        assert!(!is_on_some_monitor(&monitors, (2400, 300), (960.0, 176.0)));
        // Partly off-screen but centered on it: still fine.
        assert!(is_on_some_monitor(&monitors, (-300, 900), (960.0, 176.0)));
    }

    #[test]
    fn hit_test_scales_the_hotspot() {
        let window = Rect {
            x: 100.0,
            y: 200.0,
            width: 1440.0,
            height: 264.0,
        };
        let button = Some(CssRect {
            x: 460.0,
            y: 4.0,
            width: 40.0,
            height: 28.0,
        });
        // Button at 150%: x 790..850, y 206..248.
        assert_eq!(hit_test((800.0, 220.0), window, 1.5, button), (true, true));
        assert_eq!(hit_test((700.0, 220.0), window, 1.5, button), (true, false));
        assert_eq!(hit_test((50.0, 220.0), window, 1.5, button), (false, false));
        assert_eq!(hit_test((800.0, 220.0), window, 1.5, None), (true, false));
    }
}
