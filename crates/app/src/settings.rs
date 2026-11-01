//! User settings: what they are, their limits, and how they're stored.
//!
//! Saved as JSON in the app's config directory
//! (`%APPDATA%\io.github.allenjwzhu.lyrics-overlay\settings.json` on Windows).
//! Unknown or missing fields fall back to defaults, and out-of-range values
//! are clamped, so a hand-edited or older file never prevents startup.

use std::fs;
use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

pub const MIN_FONT_SCALE: f64 = 0.6;
pub const MAX_FONT_SCALE: f64 = 1.8;
pub const FONT_STEP: f64 = 0.1;
pub const MAX_OFFSET_MS: i64 = 10_000;
pub const OFFSET_STEP_MS: i64 = 500;

/// Colour of the sung part of a line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    #[default]
    Blue,
    Pink,
    Green,
    Gold,
    Red,
}

impl Theme {
    const ALL: [Theme; 5] = [
        Theme::Blue,
        Theme::Pink,
        Theme::Green,
        Theme::Gold,
        Theme::Red,
    ];

    pub fn next(self) -> Self {
        let i = Self::ALL.iter().position(|t| *t == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }
}

/// Window top-left in physical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowPosition {
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// Click-through: the window ignores the mouse except for its unlock button.
    pub locked: bool,
    pub visible: bool,
    pub font_scale: f64,
    pub theme: Theme,
    /// Shifts lyric timing; positive shows lines earlier.
    pub offset_ms: i64,
    /// Last position; `None` means bottom center.
    pub position: Option<WindowPosition>,
    /// Also ask the backup sources (QQ Music, NetEase) alongside LRCLIB.
    pub backup_lyrics: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            locked: false,
            visible: true,
            font_scale: 1.0,
            theme: Theme::default(),
            offset_ms: 0,
            position: None,
            backup_lyrics: true,
        }
    }
}

impl Settings {
    /// Reads settings, falling back to defaults if the file is missing or
    /// unreadable.
    pub fn load(path: &Path) -> Self {
        let mut settings: Self = fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        settings.clamp();
        settings
    }

    /// Writes atomically (temp file + rename) so a crash mid-write can't
    /// leave a truncated file.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        let text = serde_json::to_string_pretty(self).map_err(io::Error::other)?;
        fs::write(&tmp, text)?;
        fs::rename(&tmp, path)
    }

    fn clamp(&mut self) {
        if !self.font_scale.is_finite() {
            self.font_scale = 1.0;
        }
        self.font_scale = round_tenth(self.font_scale.clamp(MIN_FONT_SCALE, MAX_FONT_SCALE));
        self.offset_ms = self.offset_ms.clamp(-MAX_OFFSET_MS, MAX_OFFSET_MS);
    }

    /// Changes the font size by `steps` × [`FONT_STEP`], within limits.
    pub fn step_font(&mut self, steps: i32) {
        self.font_scale += f64::from(steps) * FONT_STEP;
        self.clamp();
    }

    pub fn adjust_offset(&mut self, delta_ms: i64) {
        self.offset_ms += delta_ms;
        self.clamp();
    }
}

/// Avoids 1.2000000000000002 from repeated steps.
fn round_tenth(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn font_steps_are_clamped_and_rounded() {
        let mut s = Settings::default();
        for _ in 0..3 {
            s.step_font(1);
        }
        assert_eq!(s.font_scale, 1.3);
        s.step_font(100);
        assert_eq!(s.font_scale, MAX_FONT_SCALE);
        s.step_font(-100);
        assert_eq!(s.font_scale, MIN_FONT_SCALE);
    }

    #[test]
    fn offset_is_clamped() {
        let mut s = Settings::default();
        s.adjust_offset(OFFSET_STEP_MS);
        s.adjust_offset(OFFSET_STEP_MS);
        assert_eq!(s.offset_ms, 1_000);
        s.adjust_offset(-60_000);
        assert_eq!(s.offset_ms, -MAX_OFFSET_MS);
    }

    #[test]
    fn themes_cycle() {
        let mut t = Theme::Blue;
        let mut seen = vec![t];
        for _ in 0..4 {
            t = t.next();
            seen.push(t);
        }
        assert_eq!(seen.len(), 5);
        assert_eq!(t.next(), Theme::Blue);
    }

    fn temp_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir()
            .join(format!("lyrics-overlay-test-{}-{name}", std::process::id()))
            .join("settings.json")
    }

    #[test]
    fn round_trips_through_disk() {
        let path = temp_path("roundtrip");
        let s = Settings {
            locked: true,
            visible: false,
            font_scale: 1.4,
            theme: Theme::Gold,
            offset_ms: -500,
            position: Some(WindowPosition { x: -1200, y: 840 }),
            backup_lyrics: false,
        };
        s.save(&path).unwrap();
        assert_eq!(Settings::load(&path), s);
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn missing_or_broken_files_give_defaults() {
        assert_eq!(Settings::load(&temp_path("missing")), Settings::default());

        let path = temp_path("broken");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "{ not json").unwrap();
        assert_eq!(Settings::load(&path), Settings::default());
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn partial_and_out_of_range_files_are_repaired() {
        let path = temp_path("partial");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            r#"{"theme": "pink", "fontScale": 9, "offsetMs": 99999, "extra": 1}"#,
        )
        .unwrap();
        let s = Settings::load(&path);
        assert_eq!(s.theme, Theme::Pink);
        assert_eq!(s.font_scale, MAX_FONT_SCALE);
        assert_eq!(s.offset_ms, MAX_OFFSET_MS);
        assert!(s.visible);
        // Files from before the backup sources existed get them turned on.
        assert!(s.backup_lyrics);
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn serializes_for_the_page_in_camel_case() {
        let json = serde_json::to_string(&Settings::default()).unwrap();
        assert!(json.contains(r#""fontScale":1.0"#));
        assert!(json.contains(r#""theme":"blue""#));
    }
}
