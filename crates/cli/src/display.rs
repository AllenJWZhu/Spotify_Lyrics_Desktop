//! Formatting for the in-place terminal status line.

use lyrics_core::player::{PlaybackStatus, PlayerSnapshot};

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

fn status_icon(status: PlaybackStatus) -> &'static str {
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
}
