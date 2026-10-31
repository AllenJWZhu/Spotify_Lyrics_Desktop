//! Terminal output: scrolling lines above a single in-place status line.

use std::io::{self, Write};

use lyrics_core::player::{PlaybackStatus, PlayerSnapshot};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// Keeps the status line from wrapping in a default-width terminal; a
/// wrapped line can't be redrawn in place with `\r`.
const MAX_STATUS_WIDTH: usize = 100;

/// Writes permanent lines that scroll up, plus one status line at the bottom
/// that is redrawn in place. Widths are measured in terminal columns, so
/// CJK text (two columns per character) is padded correctly.
pub struct Screen<W: Write> {
    out: W,
    status_width: usize,
}

impl<W: Write> Screen<W> {
    pub fn new(out: W) -> Self {
        Self {
            out,
            status_width: 0,
        }
    }

    /// Prints a line that stays on screen, replacing the current status line.
    pub fn line(&mut self, text: &str) -> io::Result<()> {
        let pad = self.status_width.saturating_sub(text.width());
        write!(self.out, "\r{text}{:pad$}\n", "")?;
        self.status_width = 0;
        self.out.flush()
    }

    /// Redraws the status line.
    pub fn status(&mut self, text: &str) -> io::Result<()> {
        let text = truncate_to_width(text, MAX_STATUS_WIDTH);
        let width = text.width();
        let pad = self.status_width.saturating_sub(width);
        write!(self.out, "\r{text}{:pad$}", "")?;
        self.status_width = width;
        self.out.flush()
    }

    /// Leaves the current status line on screen and starts a fresh one.
    pub fn keep_status(&mut self) -> io::Result<()> {
        if self.status_width > 0 {
            writeln!(self.out)?;
            self.status_width = 0;
        }
        self.out.flush()
    }

    #[cfg(test)]
    pub fn into_inner(self) -> W {
        self.out
    }
}

/// Cuts `text` to at most `max` columns, ending with `…` when shortened.
pub fn truncate_to_width(text: &str, max: usize) -> String {
    if text.width() <= max {
        return text.to_string();
    }
    let mut out = String::new();
    let mut width = 0;
    for c in text.chars() {
        let w = c.width().unwrap_or(0);
        if width + w + 1 > max {
            break;
        }
        out.push(c);
        width += w;
    }
    out.push('…');
    out
}

/// `mm:ss.t`, e.g. `03:45.0`.
pub fn format_ms(ms: u64) -> String {
    let tenths = ms / 100;
    format!(
        "{:02}:{:02}.{}",
        tenths / 600,
        (tenths / 10) % 60,
        tenths % 10
    )
}

pub fn status_icon(status: PlaybackStatus) -> &'static str {
    match status {
        PlaybackStatus::Playing => "▶",
        PlaybackStatus::Paused => "⏸",
        PlaybackStatus::Stopped => "■",
        PlaybackStatus::Changing => "…",
        PlaybackStatus::Unknown => "?",
    }
}

/// One status line: icon, estimated / total time, track, and how stale the
/// player's last position report is.
pub fn status_line(snapshot: Option<&PlayerSnapshot>, now_unix_ms: i64) -> String {
    let Some(s) = snapshot else {
        return "waiting for Spotify…".to_string();
    };
    let anchor_age_ms = (now_unix_ms - s.position_updated_unix_ms).max(0) as u64;
    format!(
        "{} {} / {}  {} — {}  (anchor age {:.1}s)",
        status_icon(s.status),
        format_ms(s.estimated_position_ms(now_unix_ms)),
        format_ms(s.track.duration_ms),
        s.track.title,
        s.track.artist,
        anchor_age_ms as f64 / 1000.0,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use lyrics_core::player::TrackInfo;

    #[test]
    fn formats_minutes_seconds_tenths() {
        assert_eq!(format_ms(0), "00:00.0");
        assert_eq!(format_ms(225_049), "03:45.0");
        assert_eq!(format_ms(61_550), "01:01.5");
        assert_eq!(format_ms(3_600_000), "60:00.0");
    }

    #[test]
    fn status_line_uses_estimated_position() {
        let s = PlayerSnapshot {
            track: TrackInfo {
                title: "Title".into(),
                artist: "Artist".into(),
                album: "Album".into(),
                duration_ms: 225_000,
            },
            status: PlaybackStatus::Playing,
            reported_position_ms: 10_000,
            position_updated_unix_ms: 1_000,
        };
        assert_eq!(
            status_line(Some(&s), 2_500),
            "▶ 00:11.5 / 03:45.0  Title — Artist  (anchor age 1.5s)"
        );
        assert_eq!(status_line(None, 0), "waiting for Spotify…");
    }

    #[test]
    fn truncates_by_columns_not_chars() {
        assert_eq!(truncate_to_width("short", 10), "short");
        assert_eq!(truncate_to_width("abcdefghij", 5), "abcd…");
        // Each CJK character takes two columns.
        assert_eq!(truncate_to_width("一二三四五", 6), "一二…");
        assert!(truncate_to_width("一二三四五", 6).width() <= 6);
    }

    fn output(f: impl FnOnce(&mut Screen<Vec<u8>>)) -> String {
        let mut s = Screen::new(Vec::new());
        f(&mut s);
        String::from_utf8(s.into_inner()).unwrap()
    }

    #[test]
    fn status_redraw_pads_over_wider_previous_text() {
        let out = output(|s| {
            s.status("一二三").unwrap(); // 6 columns
            s.status("ab").unwrap();
        });
        assert_eq!(out, "\r一二三\rab    ");
    }

    #[test]
    fn line_replaces_status_and_resets_padding() {
        let out = output(|s| {
            s.status("status").unwrap();
            s.line("ly").unwrap();
            s.status("x").unwrap();
        });
        assert_eq!(out, "\rstatus\rly    \n\rx");
    }
}
