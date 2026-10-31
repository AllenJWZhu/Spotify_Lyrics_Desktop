//! A fake player and lyrics provider, so the overlay can be tried and
//! developed without Spotify (`--demo`, and the default off Windows).
//! The lyrics are original placeholder text.

use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::Duration;

use lyrics_core::lyrics::{FoundLyrics, Lyrics, LyricsError, LyricsProvider, SyncedLyrics};
use lyrics_core::player::{
    PlaybackStatus, PlayerAction, PlayerError, PlayerSnapshot, PlayerSource, TrackInfo, now_unix_ms,
};

const LOOP_MS: u64 = 40_000;

/// Mixed scripts on purpose: the overlay must render Latin, CJK and Hangul,
/// with a translation line sharing a timestamp and an instrumental gap.
const DEMO_LRC: &str = "\
[00:01.00]Lyrics Overlay demo
[00:04.00]Lines light up as they are sung
[00:07.50]这一行用来测试中文显示
[00:07.50](this line tests Chinese text)
[00:11.00]한국어 가사도 잘 보여야 해요
[00:15.00]
[00:19.00]A long line to check that text which does not fit shrinks instead of being cut off
[00:25.00]Drag the window to move it
[00:29.00]Hover for the controls
[00:33.00]The demo loops from here
";

fn demo_track() -> &'static TrackInfo {
    static TRACK: OnceLock<TrackInfo> = OnceLock::new();
    TRACK.get_or_init(|| TrackInfo {
        title: "Demo Song".into(),
        artist: "Lyrics Overlay".into(),
        album: String::new(),
        duration_ms: LOOP_MS,
    })
}

/// Plays the demo track on a loop, starting when created. Supports
/// play/pause; next and previous restart the track.
pub struct DemoSource {
    clock: Mutex<Clock>,
}

#[derive(Debug, Clone, Copy)]
enum Clock {
    /// Position 0 was at this wall-clock time.
    Playing {
        origin_unix_ms: i64,
    },
    Paused {
        position_ms: u64,
    },
}

impl Clock {
    fn position_ms(self, now: i64) -> u64 {
        match self {
            Clock::Playing { origin_unix_ms } => (now - origin_unix_ms).max(0) as u64 % LOOP_MS,
            Clock::Paused { position_ms } => position_ms,
        }
    }
}

impl DemoSource {
    pub fn new() -> Self {
        Self {
            clock: Mutex::new(Clock::Playing {
                origin_unix_ms: now_unix_ms(),
            }),
        }
    }

    fn clock(&self) -> std::sync::MutexGuard<'_, Clock> {
        self.clock.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl PlayerSource for DemoSource {
    fn snapshot(&self) -> Result<Option<PlayerSnapshot>, PlayerError> {
        let now = now_unix_ms();
        let clock = *self.clock();
        let status = match clock {
            Clock::Playing { .. } => PlaybackStatus::Playing,
            Clock::Paused { .. } => PlaybackStatus::Paused,
        };
        Ok(Some(PlayerSnapshot {
            track: demo_track().clone(),
            status,
            reported_position_ms: clock.position_ms(now),
            position_updated_unix_ms: now,
        }))
    }

    fn control(&self, action: PlayerAction) -> Result<(), PlayerError> {
        let now = now_unix_ms();
        let mut clock = self.clock();
        *clock = match (action, *clock) {
            (PlayerAction::PlayPause, c @ Clock::Playing { .. }) => Clock::Paused {
                position_ms: c.position_ms(now),
            },
            (PlayerAction::PlayPause, Clock::Paused { position_ms }) => Clock::Playing {
                origin_unix_ms: now - position_ms as i64,
            },
            (PlayerAction::Next | PlayerAction::Previous, _) => Clock::Playing {
                origin_unix_ms: now,
            },
        };
        Ok(())
    }
}

pub struct DemoProvider;

impl LyricsProvider for DemoProvider {
    fn fetch(&self, track: &TrackInfo) -> Result<Option<FoundLyrics>, LyricsError> {
        // Let the "loading" state show briefly, like a real lookup.
        thread::sleep(Duration::from_millis(400));
        if track != demo_track() {
            return Ok(None);
        }
        Ok(Some(FoundLyrics {
            lyrics: Lyrics::Synced(SyncedLyrics::parse(DEMO_LRC)),
            source: "demo".into(),
            matched_title: track.title.clone(),
            matched_artist: track.artist.clone(),
            matched_duration_ms: Some(LOOP_MS),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_lyrics_parse() {
        let found = DemoProvider.fetch(demo_track()).unwrap().unwrap();
        let Lyrics::Synced(s) = found.lyrics else {
            panic!("not synced")
        };
        assert_eq!(s.lines().len(), 10);
        assert!(s.lines().iter().all(|l| l.start_ms < LOOP_MS));
    }

    #[test]
    fn demo_source_starts_playing_within_the_loop() {
        let snap = DemoSource::new().snapshot().unwrap().unwrap();
        assert_eq!(snap.status, PlaybackStatus::Playing);
        assert!(snap.reported_position_ms < LOOP_MS);
    }

    #[test]
    fn demo_source_pauses_resumes_and_restarts() {
        let clock = Clock::Playing {
            origin_unix_ms: now_unix_ms() - 12_000,
        };
        let source = DemoSource {
            clock: Mutex::new(clock),
        };
        source.control(PlayerAction::PlayPause).unwrap();
        let paused = source.snapshot().unwrap().unwrap();
        assert_eq!(paused.status, PlaybackStatus::Paused);
        assert!((12_000..12_500).contains(&paused.reported_position_ms));

        source.control(PlayerAction::PlayPause).unwrap();
        let resumed = source.snapshot().unwrap().unwrap();
        assert_eq!(resumed.status, PlaybackStatus::Playing);
        assert!(resumed.reported_position_ms >= paused.reported_position_ms);

        source.control(PlayerAction::Next).unwrap();
        assert!(source.snapshot().unwrap().unwrap().reported_position_ms < 500);
    }
}
