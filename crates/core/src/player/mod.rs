//! Player state model and position estimation.
//!
//! Media sessions report playback position as an *anchor*: "the position was
//! P at wall-clock time T". They do not push a new value every frame, so
//! between reports we extrapolate locally. See
//! [`PlayerSnapshot::estimated_position_ms`].

mod time;

#[cfg(windows)]
pub mod smtc;

pub use time::{now_unix_ms, winrt_datetime_to_unix_ms, winrt_timespan_to_ms};

/// Identity and length of the track the player is on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackInfo {
    pub title: String,
    pub artist: String,
    pub album: String,
    /// Track length in milliseconds; 0 if the player didn't report one.
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackStatus {
    Playing,
    Paused,
    Stopped,
    /// The player is switching tracks or buffering.
    Changing,
    Unknown,
}

/// One reading of the player's state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerSnapshot {
    pub track: TrackInfo,
    pub status: PlaybackStatus,
    /// Position the player last reported, in milliseconds.
    pub reported_position_ms: u64,
    /// Wall-clock time (Unix ms) at which `reported_position_ms` was true.
    pub position_updated_unix_ms: i64,
}

impl PlayerSnapshot {
    /// Best guess of the playback position at `now_unix_ms`.
    ///
    /// While playing, the position advances in real time from the last
    /// anchor. Otherwise it stays at the reported value. The result is
    /// clamped to the track length when one is known.
    pub fn estimated_position_ms(&self, now_unix_ms: i64) -> u64 {
        let mut position = self.reported_position_ms;
        if self.status == PlaybackStatus::Playing {
            // An anchor stamped in the future (clock skew) must not move us backwards.
            let elapsed = (now_unix_ms - self.position_updated_unix_ms).max(0) as u64;
            position = position.saturating_add(elapsed);
        }
        if self.track.duration_ms > 0 {
            position = position.min(self.track.duration_ms);
        }
        position
    }

    /// Whether `other` carries a different position anchor than `self`.
    pub fn anchor_differs(&self, other: &Self) -> bool {
        self.reported_position_ms != other.reported_position_ms
            || self.position_updated_unix_ms != other.position_updated_unix_ms
    }
}

/// How far a new anchor lands from where the previous anchor predicted,
/// in milliseconds (`reported - predicted`). Positive means the player is
/// ahead of our extrapolation.
///
/// Only meaningful when both readings are of the same track and both are
/// playing; returns `None` otherwise.
pub fn anchor_drift_ms(prev: &PlayerSnapshot, next: &PlayerSnapshot) -> Option<i64> {
    let both_playing =
        prev.status == PlaybackStatus::Playing && next.status == PlaybackStatus::Playing;
    if prev.track != next.track || !both_playing {
        return None;
    }
    let predicted = prev.estimated_position_ms(next.position_updated_unix_ms);
    Some(next.reported_position_ms as i64 - predicted as i64)
}

/// Whether an app's AppUserModelID belongs to Spotify.
///
/// Covers both the classic installer (`Spotify.exe`) and the Microsoft
/// Store build (`SpotifyAB.SpotifyMusic_<publisher>!Spotify`).
pub fn is_spotify_app_id(app_id: &str) -> bool {
    app_id.to_ascii_lowercase().contains("spotify")
}

#[derive(Debug, thiserror::Error)]
pub enum PlayerError {
    #[cfg(windows)]
    #[error("Windows media session API call failed: {0}")]
    Windows(#[from] windows::core::Error),

    #[error("{0}")]
    Other(String),
}

/// Something that can tell us what a music player is doing.
pub trait PlayerSource {
    /// Current state of the target player, or `None` if it isn't running
    /// or has no media session.
    fn snapshot(&self) -> Result<Option<PlayerSnapshot>, PlayerError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(duration_ms: u64) -> TrackInfo {
        TrackInfo {
            title: "Test Title".into(),
            artist: "Test Artist".into(),
            album: "Test Album".into(),
            duration_ms,
        }
    }

    fn snap(status: PlaybackStatus, pos: u64, at: i64) -> PlayerSnapshot {
        PlayerSnapshot {
            track: track(200_000),
            status,
            reported_position_ms: pos,
            position_updated_unix_ms: at,
        }
    }

    #[test]
    fn playing_advances_from_anchor() {
        let s = snap(PlaybackStatus::Playing, 10_000, 1_000);
        assert_eq!(s.estimated_position_ms(1_000), 10_000);
        assert_eq!(s.estimated_position_ms(3_500), 12_500);
    }

    #[test]
    fn paused_holds_position() {
        let s = snap(PlaybackStatus::Paused, 10_000, 1_000);
        assert_eq!(s.estimated_position_ms(60_000), 10_000);
    }

    #[test]
    fn future_anchor_does_not_rewind() {
        let s = snap(PlaybackStatus::Playing, 10_000, 5_000);
        assert_eq!(s.estimated_position_ms(4_000), 10_000);
    }

    #[test]
    fn clamps_to_track_length() {
        let s = snap(PlaybackStatus::Playing, 199_000, 0);
        assert_eq!(s.estimated_position_ms(10_000), 200_000);
    }

    #[test]
    fn unknown_length_is_not_clamped() {
        let mut s = snap(PlaybackStatus::Playing, 199_000, 0);
        s.track.duration_ms = 0;
        assert_eq!(s.estimated_position_ms(10_000), 209_000);
    }

    #[test]
    fn drift_is_zero_when_player_matches_wall_clock() {
        let a = snap(PlaybackStatus::Playing, 10_000, 1_000);
        let b = snap(PlaybackStatus::Playing, 15_000, 6_000);
        assert_eq!(anchor_drift_ms(&a, &b), Some(0));
    }

    #[test]
    fn drift_sign_matches_player_lead() {
        let a = snap(PlaybackStatus::Playing, 10_000, 1_000);
        let ahead = snap(PlaybackStatus::Playing, 15_200, 6_000);
        let behind = snap(PlaybackStatus::Playing, 14_700, 6_000);
        assert_eq!(anchor_drift_ms(&a, &ahead), Some(200));
        assert_eq!(anchor_drift_ms(&a, &behind), Some(-300));
    }

    #[test]
    fn drift_undefined_across_tracks_or_pauses() {
        let a = snap(PlaybackStatus::Playing, 10_000, 1_000);
        let mut other_track = snap(PlaybackStatus::Playing, 0, 6_000);
        other_track.track.title = "Another".into();
        let paused = snap(PlaybackStatus::Paused, 15_000, 6_000);
        assert_eq!(anchor_drift_ms(&a, &other_track), None);
        assert_eq!(anchor_drift_ms(&a, &paused), None);
    }

    #[test]
    fn recognizes_spotify_app_ids() {
        assert!(is_spotify_app_id("Spotify.exe"));
        assert!(is_spotify_app_id(
            "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify"
        ));
        assert!(!is_spotify_app_id("Chrome"));
        assert!(!is_spotify_app_id(
            "Microsoft.ZuneMusic_8wekyb3d8bbwe!Microsoft.ZuneMusic"
        ));
    }
}
