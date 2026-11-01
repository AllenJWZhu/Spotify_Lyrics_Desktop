//! What the overlay page is told, and when.
//!
//! Two messages, both plain data:
//! - [`OverlayState`]: the current track and its lyrics. Sent when it
//!   changes (track change, lookup finished or failed).
//! - [`PlaybackView`]: a position anchor plus the current line. The page
//!   extrapolates from the anchor every animation frame for the karaoke
//!   fill, so this is only sent when the page's extrapolation would be off:
//!   line change, play/pause, seek, or a periodic refresh.

use serde::Serialize;

use lyrics_core::follow::{Follower, LookupState};
use lyrics_core::lyrics::Lyrics;
use lyrics_core::player::{PlaybackStatus, PlayerSnapshot};

pub const STATE_EVENT: &str = "overlay-state";
pub const PLAYBACK_EVENT: &str = "overlay-playback";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackView {
    pub title: String,
    pub artist: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LineView {
    pub start_ms: u64,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Lookup {
    /// Nothing playing.
    Idle,
    Loading,
    Synced,
    /// Lyrics exist but without timing.
    Plain,
    Instrumental,
    /// No lyrics found.
    Missing,
    /// Lookup failed: `error` says why, `retrying` whether another attempt
    /// is scheduled.
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlayState {
    pub track: Option<TrackView>,
    pub lookup: Lookup,
    pub retrying: bool,
    /// Why the lookup failed, for the user.
    pub error: Option<String>,
    /// Synced lines; empty unless `lookup` is `synced`.
    pub lines: Vec<LineView>,
    /// Where the lyrics came from, for attribution.
    pub source: Option<String>,
    /// Waiting for the player itself (e.g. Spotify not running).
    pub message: Option<String>,
}

impl Default for OverlayState {
    fn default() -> Self {
        Self {
            track: None,
            lookup: Lookup::Idle,
            retrying: false,
            error: None,
            lines: Vec::new(),
            source: None,
            message: None,
        }
    }
}

impl OverlayState {
    pub fn from_follower(follower: &Follower) -> Self {
        let track = follower.track().map(|t| TrackView {
            title: t.title.clone(),
            artist: t.artist.clone(),
        });
        let mut state = Self {
            track,
            ..Self::default()
        };
        match follower.state() {
            LookupState::Idle => {}
            LookupState::Loading { .. } => state.lookup = Lookup::Loading,
            LookupState::Failed {
                retry_at_ms, error, ..
            } => {
                state.lookup = Lookup::Error;
                state.retrying = retry_at_ms.is_some();
                state.error = Some(error.clone());
            }
            LookupState::Ready(None) => state.lookup = Lookup::Missing,
            LookupState::Ready(Some(found)) => {
                state.source = Some(found.source.to_string());
                match &found.lyrics {
                    Lyrics::Synced(synced) => {
                        state.lookup = Lookup::Synced;
                        state.lines = synced
                            .lines()
                            .iter()
                            .map(|l| LineView {
                                start_ms: l.start_ms,
                                text: l.text.clone(),
                            })
                            .collect();
                    }
                    Lyrics::Plain(_) => state.lookup = Lookup::Plain,
                    Lyrics::Instrumental => state.lookup = Lookup::Instrumental,
                }
            }
        }
        state
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackView {
    /// Lyric-time position (offset applied) at `at_unix_ms`.
    pub position_ms: u64,
    pub at_unix_ms: i64,
    pub playing: bool,
    /// Index into `OverlayState::lines` of the line being sung.
    pub line: Option<usize>,
}

impl PlaybackView {
    pub fn from_follower(follower: &Follower, snapshot: &PlayerSnapshot, now_ms: i64) -> Self {
        Self {
            position_ms: follower.lyric_position_ms(snapshot, now_ms),
            at_unix_ms: now_ms,
            playing: snapshot.status == PlaybackStatus::Playing,
            line: follower.line(),
        }
    }

    /// Where the page will think playback is at `now_ms`.
    fn extrapolate(&self, now_ms: i64) -> i64 {
        let elapsed = if self.playing {
            (now_ms - self.at_unix_ms).max(0)
        } else {
            0
        };
        self.position_ms as i64 + elapsed
    }
}

/// Decides when the page needs a fresh [`PlaybackView`].
#[derive(Default)]
pub struct PlaybackThrottle {
    last_sent: Option<PlaybackView>,
}

impl PlaybackThrottle {
    /// The page's extrapolation may be off by this much before we correct it.
    const MAX_ERROR_MS: i64 = 150;
    /// Refresh at least this often regardless.
    const REFRESH_MS: i64 = 1_000;

    /// Returns the view to send, if the page needs one.
    pub fn update(&mut self, current: PlaybackView) -> Option<PlaybackView> {
        let send = match &self.last_sent {
            None => true,
            Some(last) => {
                last.playing != current.playing
                    || last.line != current.line
                    || (last.extrapolate(current.at_unix_ms) - current.position_ms as i64).abs()
                        > Self::MAX_ERROR_MS
                    || current.at_unix_ms - last.at_unix_ms >= Self::REFRESH_MS
            }
        };
        if send {
            self.last_sent = Some(current.clone());
            Some(current)
        } else {
            None
        }
    }

    /// Forget what was sent, e.g. after the track changed.
    pub fn reset(&mut self) {
        self.last_sent = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lyrics_core::follow::FetchError;
    use lyrics_core::lyrics::{FoundLyrics, SyncedLyrics};
    use lyrics_core::player::TrackInfo;

    fn track() -> TrackInfo {
        TrackInfo {
            title: "Song".into(),
            artist: "Artist".into(),
            album: String::new(),
            duration_ms: 200_000,
        }
    }

    fn snapshot(status: PlaybackStatus, pos: u64, at: i64) -> PlayerSnapshot {
        PlayerSnapshot {
            track: track(),
            status,
            reported_position_ms: pos,
            position_updated_unix_ms: at,
        }
    }

    fn settled(result: Result<Option<FoundLyrics>, FetchError>) -> Follower {
        let mut f = Follower::new(1_000, 0);
        let s = snapshot(PlaybackStatus::Playing, 0, 0);
        f.tick(Some(&s), 0);
        f.tick(Some(&s), 1_000);
        f.on_fetched(track(), result, 1_000);
        f
    }

    fn found(lyrics: Lyrics) -> Option<FoundLyrics> {
        Some(FoundLyrics {
            lyrics,
            source: "LRCLIB".into(),
            matched_title: "Song".into(),
            matched_artist: "Artist".into(),
            matched_duration_ms: None,
        })
    }

    #[test]
    fn state_for_synced_lyrics_lists_lines() {
        let lrc = "[00:01.00]one\n[00:02.50]two";
        let lyrics = Lyrics::Synced(SyncedLyrics::parse(lrc));
        let state = OverlayState::from_follower(&settled(Ok(found(lyrics))));
        assert_eq!(state.lookup, Lookup::Synced);
        assert_eq!(
            state.track,
            Some(TrackView {
                title: "Song".into(),
                artist: "Artist".into()
            })
        );
        assert_eq!(state.lines.len(), 2);
        assert_eq!(state.lines[1].start_ms, 2_500);
        assert_eq!(state.source.as_deref(), Some("LRCLIB"));
    }

    #[test]
    fn state_for_other_outcomes() {
        let cases = [
            (Ok(None), Lookup::Missing, false),
            (Ok(found(Lyrics::Plain("x".into()))), Lookup::Plain, false),
            (Ok(found(Lyrics::Instrumental)), Lookup::Instrumental, false),
            (
                Err(FetchError {
                    message: "LRCLIB is down right now".into(),
                    retryable: true,
                }),
                Lookup::Error,
                true,
            ),
        ];
        for (result, lookup, retrying) in cases {
            let state = OverlayState::from_follower(&settled(result));
            assert_eq!(state.lookup, lookup);
            assert_eq!(state.retrying, retrying);
            assert_eq!(state.error.is_some(), lookup == Lookup::Error);
            assert!(state.lines.is_empty());
        }
        assert_eq!(
            OverlayState::from_follower(&Follower::new(1_000, 0)),
            OverlayState::default()
        );
    }

    #[test]
    fn serializes_in_camel_case() {
        let view = PlaybackView {
            position_ms: 1,
            at_unix_ms: 2,
            playing: true,
            line: None,
        };
        assert_eq!(
            serde_json::to_string(&view).unwrap(),
            r#"{"positionMs":1,"atUnixMs":2,"playing":true,"line":null}"#
        );
        let state = OverlayState::default();
        assert!(
            serde_json::to_string(&state)
                .unwrap()
                .contains(r#""lookup":"idle""#)
        );
    }

    fn view(position_ms: u64, at_unix_ms: i64, playing: bool, line: Option<usize>) -> PlaybackView {
        PlaybackView {
            position_ms,
            at_unix_ms,
            playing,
            line,
        }
    }

    #[test]
    fn throttle_skips_updates_the_page_can_extrapolate() {
        let mut t = PlaybackThrottle::default();
        assert!(t.update(view(10_000, 0, true, Some(1))).is_some());
        // Steady playback: position matches extrapolation.
        assert!(t.update(view(10_100, 100, true, Some(1))).is_none());
        assert!(t.update(view(10_900, 900, true, Some(1))).is_none());
        // Periodic refresh.
        assert!(t.update(view(11_000, 1_000, true, Some(1))).is_some());
    }

    #[test]
    fn throttle_sends_on_line_pause_and_seek() {
        let mut t = PlaybackThrottle::default();
        t.update(view(10_000, 0, true, Some(1)));
        assert!(t.update(view(10_100, 100, true, Some(2))).is_some(), "line");
        assert!(
            t.update(view(10_200, 200, false, Some(2))).is_some(),
            "pause"
        );
        assert!(t.update(view(10_200, 300, false, Some(2))).is_none());
        assert!(
            t.update(view(60_000, 400, false, Some(2))).is_some(),
            "seek"
        );
        t.reset();
        assert!(
            t.update(view(60_000, 450, false, Some(2))).is_some(),
            "reset"
        );
    }
}
