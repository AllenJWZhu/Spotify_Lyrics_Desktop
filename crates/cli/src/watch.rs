//! Default mode: follow Spotify and print each lyric line as it's sung.
//!
//! All decisions live in [`lyrics_core::follow::Follower`]; this module
//! turns its events into terminal lines and runs fetches on worker threads.

// On non-Windows targets `main` never calls `run`.
#![cfg_attr(not(windows), allow(dead_code))]

use std::io;
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Duration;

use lyrics_core::follow::{FetchError, FollowEvent, Follower, LookupState};
use lyrics_core::lyrics::{FoundLyrics, Lyrics, LyricsProvider, SyncedLyrics};
use lyrics_core::player::{PlayerSnapshot, PlayerSource, TrackInfo, now_unix_ms};

use crate::WatchArgs;
use crate::display::{Screen, format_ms, status_icon};

/// Terminal lines for one follower event. Fetch requests produce none.
pub fn render(event: &FollowEvent, follower: &Follower) -> Vec<String> {
    match event {
        FollowEvent::TrackChanged(t) => {
            vec![String::new(), format!("♪ {} — {}", t.title, t.artist)]
        }
        FollowEvent::Cleared => vec![String::new(), "(nothing playing)".into()],
        FollowEvent::FetchRequested(_) => Vec::new(),
        FollowEvent::LyricsReady(found) => describe(found),
        FollowEvent::LookupFailed { error, retry_in_ms } => {
            let next = match retry_in_ms {
                Some(ms) => format!("retrying in {}s", ms / 1000),
                None => "giving up for this track".into(),
            };
            vec![format!("  lyrics lookup failed: {error}; {next}")]
        }
        FollowEvent::LineChanged(Some(i)) => follower
            .synced()
            .map(|s| line_group(s, *i))
            .unwrap_or_default(),
        FollowEvent::LineChanged(None) => Vec::new(),
    }
}

/// Line `index` plus any lines sharing its timestamp (typically original +
/// translation, where the follower reports the last of the group).
fn line_group(synced: &SyncedLyrics, index: usize) -> Vec<String> {
    let lines = synced.lines();
    let start = lines[index].start_ms;
    let first = lines[..index]
        .iter()
        .rposition(|l| l.start_ms != start)
        .map_or(0, |i| i + 1);
    lines[first..=index]
        .iter()
        .map(|l| {
            let text = if l.text.is_empty() { "♪" } else { &l.text };
            format!("  [{}] {}", format_ms(l.start_ms), text)
        })
        .collect()
}

/// One-time summary when lyrics become available for the current track.
fn describe(found: &Option<FoundLyrics>) -> Vec<String> {
    let Some(f) = found else {
        return vec!["  no lyrics found".into()];
    };
    let matched = format!("matched “{}” by {}", f.matched_title, f.matched_artist);
    match &f.lyrics {
        Lyrics::Synced(s) => vec![format!(
            "  synced lyrics from {} ({} lines, {matched})",
            f.source,
            s.lines().len()
        )],
        Lyrics::Plain(text) => {
            let mut out = vec![format!(
                "  only unsynced lyrics on {} ({matched}):",
                f.source
            )];
            out.extend(text.lines().map(|l| format!("    {l}")));
            out
        }
        Lyrics::Instrumental => vec![format!("  instrumental according to {}", f.source)],
    }
}

pub fn status(follower: &Follower, snapshot: Option<&PlayerSnapshot>, now_ms: i64) -> String {
    let Some(s) = snapshot else {
        return "waiting for Spotify…".into();
    };
    let lyrics = match follower.state() {
        LookupState::Idle => "",
        LookupState::Loading { attempt: 1 } => "lyrics: loading…",
        LookupState::Loading { .. } => "lyrics: retrying…",
        LookupState::Failed {
            retry_at_ms: Some(_),
            ..
        } => "lyrics: error, will retry",
        LookupState::Failed {
            retry_at_ms: None, ..
        } => "lyrics: error",
        LookupState::Ready(None) => "lyrics: none",
        LookupState::Ready(Some(f)) => match f.lyrics {
            Lyrics::Synced(_) => "lyrics: synced",
            Lyrics::Plain(_) => "lyrics: unsynced",
            Lyrics::Instrumental => "lyrics: instrumental",
        },
    };
    let offset = match follower.offset_ms() {
        0 => String::new(),
        ms => format!("  offset {ms:+}ms"),
    };
    format!(
        "{} {} / {}  {}{offset}  {} — {}",
        status_icon(s.status),
        format_ms(s.estimated_position_ms(now_ms)),
        format_ms(s.track.duration_ms),
        lyrics,
        s.track.title,
        s.track.artist,
    )
}

type FetchResult = (TrackInfo, Result<Option<FoundLyrics>, FetchError>);

pub fn run(
    source: &impl PlayerSource,
    provider: Arc<dyn LyricsProvider>,
    args: &WatchArgs,
) -> anyhow::Result<()> {
    let mut screen = Screen::new(io::stdout());
    let mut follower = Follower::new(args.settle_ms, args.offset_ms);
    let (tx, rx) = mpsc::channel::<FetchResult>();

    screen.line("Following Spotify. Lyrics from LRCLIB (lrclib.net). Ctrl+C to stop.")?;

    loop {
        let now = now_unix_ms();
        let snapshot = source.snapshot().unwrap_or_else(|e| {
            // Sessions can vanish between calls while Spotify starts or quits.
            let _ = screen.line(&format!("SMTC error: {e}"));
            None
        });

        let mut events = follower.tick(snapshot.as_ref(), now);
        while let Ok((track, result)) = rx.try_recv() {
            events.extend(follower.on_fetched(track, result, now));
        }

        for event in &events {
            if let FollowEvent::FetchRequested(track) = event {
                let provider = Arc::clone(&provider);
                let tx = tx.clone();
                let track = track.clone();
                thread::spawn(move || {
                    // The terminal can show the technical detail too.
                    let result = provider.fetch(&track).map_err(|e| FetchError {
                        message: format!("{} ({e})", e.message()),
                        retryable: e.kind().is_retryable(),
                    });
                    // The receiver only goes away when we exit.
                    let _ = tx.send((track, result));
                });
            }
            for line in render(event, &follower) {
                screen.line(&line)?;
            }
        }

        screen.status(&status(&follower, snapshot.as_ref(), now))?;
        thread::sleep(Duration::from_millis(args.interval_ms));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lyrics_core::player::PlaybackStatus;

    const LRC: &str = "[00:01.00]line one\n[00:03.00]line two\n[00:03.00]translation two\n\
                       [00:05.00]\n[00:06.00]line four";

    fn track() -> TrackInfo {
        TrackInfo {
            title: "Song".into(),
            artist: "Artist".into(),
            album: String::new(),
            duration_ms: 200_000,
        }
    }

    fn playing(pos: u64, at: i64) -> PlayerSnapshot {
        PlayerSnapshot {
            track: track(),
            status: PlaybackStatus::Playing,
            reported_position_ms: pos,
            position_updated_unix_ms: at,
        }
    }

    fn found(lyrics: Lyrics) -> Option<FoundLyrics> {
        Some(FoundLyrics {
            lyrics,
            source: "LRCLIB".into(),
            matched_title: "Song".into(),
            matched_artist: "Artist".into(),
            matched_duration_ms: Some(200_000),
        })
    }

    /// Runs the follower like `run` does and collects the printed lines.
    struct Harness {
        follower: Follower,
        printed: Vec<String>,
    }

    impl Harness {
        fn new(offset_ms: i64) -> Self {
            Self {
                follower: Follower::new(1_000, offset_ms),
                printed: Vec::new(),
            }
        }
        fn apply(&mut self, events: Vec<FollowEvent>) {
            for e in &events {
                self.printed.extend(render(e, &self.follower));
            }
        }
        fn tick(&mut self, snap: &PlayerSnapshot, now: i64) {
            let events = self.follower.tick(Some(snap), now);
            self.apply(events);
        }
        fn fetched(&mut self, result: Result<Option<FoundLyrics>, FetchError>, now: i64) {
            let events = self.follower.on_fetched(track(), result, now);
            self.apply(events);
        }
        fn take(&mut self) -> Vec<String> {
            std::mem::take(&mut self.printed)
        }
    }

    #[test]
    fn prints_track_header_summary_and_line_groups() {
        let mut h = Harness::new(0);
        h.tick(&playing(0, 0), 0);
        h.tick(&playing(0, 0), 1_000);
        assert_eq!(h.take(), vec!["", "♪ Song — Artist"]);

        h.fetched(Ok(found(Lyrics::Synced(SyncedLyrics::parse(LRC)))), 1_000);
        assert_eq!(
            h.take(),
            vec!["  synced lyrics from LRCLIB (5 lines, matched “Song” by Artist)"]
        );

        h.tick(&playing(0, 0), 1_100);
        h.tick(&playing(0, 0), 3_000);
        h.tick(&playing(0, 0), 5_000);
        assert_eq!(
            h.take(),
            vec![
                "  [00:01.0] line one",
                "  [00:03.0] line two",
                "  [00:03.0] translation two",
                "  [00:05.0] ♪",
            ]
        );
    }

    #[test]
    fn prints_plain_lyrics_and_failures() {
        let mut h = Harness::new(0);
        h.tick(&playing(0, 0), 0);
        h.tick(&playing(0, 0), 1_000);
        h.take();
        h.fetched(
            Err(FetchError {
                message: "LRCLIB is having problems right now (HTTP 503)".into(),
                retryable: true,
            }),
            1_000,
        );
        assert_eq!(
            h.take(),
            vec![
                "  lyrics lookup failed: LRCLIB is having problems right now (HTTP 503); retrying in 5s"
            ]
        );
        h.tick(&playing(0, 0), 6_000);
        h.fetched(Ok(found(Lyrics::Plain("a\nb".into()))), 6_000);
        assert_eq!(
            h.take(),
            vec![
                "  only unsynced lyrics on LRCLIB (matched “Song” by Artist):",
                "    a",
                "    b",
            ]
        );
    }

    #[test]
    fn status_line_reflects_lookup_and_offset() {
        let mut h = Harness::new(-300);
        h.tick(&playing(0, 0), 0);
        h.tick(&playing(0, 0), 1_000);
        assert_eq!(
            status(&h.follower, Some(&playing(10_000, 1_000)), 2_500),
            "▶ 00:11.5 / 03:20.0  lyrics: loading…  offset -300ms  Song — Artist"
        );
        h.fetched(Ok(None), 2_500);
        assert!(status(&h.follower, Some(&playing(0, 0)), 2_500).contains("lyrics: none"));
        assert_eq!(status(&h.follower, None, 0), "waiting for Spotify…");
    }
}
