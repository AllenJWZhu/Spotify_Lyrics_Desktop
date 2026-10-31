//! A fake player and lyrics provider, so the overlay can be tried and
//! developed without Spotify (`--demo`, and the default off Windows).
//! The lyrics are original placeholder text.

use std::sync::OnceLock;
use std::thread;
use std::time::Duration;

use lyrics_core::lyrics::{FoundLyrics, Lyrics, LyricsError, LyricsProvider, SyncedLyrics};
use lyrics_core::player::{
    PlaybackStatus, PlayerError, PlayerSnapshot, PlayerSource, TrackInfo, now_unix_ms,
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
[00:29.00]Hover for the close button
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

/// Plays the demo track on a loop, starting when created.
pub struct DemoSource {
    started_unix_ms: i64,
}

impl DemoSource {
    pub fn new() -> Self {
        Self {
            started_unix_ms: now_unix_ms(),
        }
    }
}

impl PlayerSource for DemoSource {
    fn snapshot(&self) -> Result<Option<PlayerSnapshot>, PlayerError> {
        let now = now_unix_ms();
        let elapsed = (now - self.started_unix_ms).max(0) as u64;
        Ok(Some(PlayerSnapshot {
            track: demo_track().clone(),
            status: PlaybackStatus::Playing,
            reported_position_ms: elapsed % LOOP_MS,
            position_updated_unix_ms: now,
        }))
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
            source: "demo",
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
    fn demo_source_is_always_playing_within_the_loop() {
        let snap = DemoSource::new().snapshot().unwrap().unwrap();
        assert_eq!(snap.status, PlaybackStatus::Playing);
        assert!(snap.reported_position_ms < LOOP_MS);
    }
}
