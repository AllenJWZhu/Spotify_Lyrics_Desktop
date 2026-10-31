//! Default mode: follow Spotify and print each lyric line as it's sung.
//!
//! [`Watcher`] holds all decisions (when a track is current, when to fetch,
//! which line to print) and is driven by plain values, so it is tested
//! without Windows or the network. [`run`] wires it to SMTC, a worker thread
//! per lyrics fetch, and the terminal.

// On non-Windows targets `main` never calls `run`.
#![cfg_attr(not(windows), allow(dead_code))]

use std::collections::HashMap;
use std::io;
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Duration;

use lyrics_core::lyrics::{FoundLyrics, Lyrics, LyricsProvider, SyncedLyrics};
use lyrics_core::player::{
    PlayerSnapshot, PlayerSource, SettleEvent, TrackInfo, TrackSettler, now_unix_ms,
};

use crate::WatchArgs;
use crate::display::{Screen, format_ms, status_icon};

#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    /// Print a permanent line.
    Print(String),
    /// Start fetching lyrics for this track.
    Fetch(TrackInfo),
}

#[derive(Debug, Clone, PartialEq)]
enum LyricsState {
    Idle,
    Loading,
    Ready(Option<FoundLyrics>),
    Failed,
}

pub struct Watcher {
    settler: TrackSettler,
    offset_ms: i64,
    /// Results by track, including "nothing found", so revisiting a track
    /// (or repeat-one) doesn't refetch. Errors are not cached.
    cache: HashMap<TrackInfo, Option<FoundLyrics>>,
    state: LyricsState,
    /// Index of the last printed lyric line.
    last_line: Option<usize>,
}

impl Watcher {
    pub fn new(settle_ms: i64, offset_ms: i64) -> Self {
        Self {
            settler: TrackSettler::new(settle_ms),
            offset_ms,
            cache: HashMap::new(),
            state: LyricsState::Idle,
            last_line: None,
        }
    }

    /// Processes one player reading.
    pub fn tick(&mut self, snapshot: Option<&PlayerSnapshot>, now_ms: i64) -> Vec<Effect> {
        let mut effects = Vec::new();

        match self.settler.observe(snapshot.map(|s| &s.track), now_ms) {
            Some(SettleEvent::Settled(track)) => {
                effects.push(Effect::Print(String::new()));
                effects.push(Effect::Print(format!(
                    "♪ {} — {}",
                    track.title, track.artist
                )));
                self.last_line = None;
                match self.cache.get(&track).cloned() {
                    Some(found) => {
                        effects.extend(describe(&found));
                        self.state = LyricsState::Ready(found);
                    }
                    None => {
                        effects.push(Effect::Fetch(track));
                        self.state = LyricsState::Loading;
                    }
                }
            }
            Some(SettleEvent::Cleared) => {
                effects.push(Effect::Print(String::new()));
                effects.push(Effect::Print("(nothing playing)".into()));
                self.state = LyricsState::Idle;
                self.last_line = None;
            }
            None => {}
        }

        if let Some(snapshot) = snapshot {
            effects.extend(self.follow(snapshot, now_ms));
        }
        effects
    }

    /// Prints the lyric line(s) that just started, if any.
    fn follow(&mut self, snapshot: &PlayerSnapshot, now_ms: i64) -> Vec<Effect> {
        let LyricsState::Ready(Some(FoundLyrics {
            lyrics: Lyrics::Synced(synced),
            ..
        })) = &self.state
        else {
            return Vec::new();
        };
        // While a track change is in flight the snapshot can mix two
        // tracks; only follow when it is exactly the settled one.
        if self.settler.settled() != Some(&snapshot.track) {
            return Vec::new();
        }

        let position = snapshot.estimated_position_ms(now_ms) as i64 + self.offset_ms;
        let index = synced.line_index_at(position.max(0) as u64);
        if index == self.last_line {
            return Vec::new();
        }
        self.last_line = index;
        index.map(|i| print_group(synced, i)).unwrap_or_default()
    }

    /// Handles a finished fetch. Results for a track that is no longer
    /// current are cached but not shown.
    pub fn on_fetched(
        &mut self,
        track: TrackInfo,
        result: Result<Option<FoundLyrics>, String>,
    ) -> Vec<Effect> {
        let current = self.settler.settled() == Some(&track);
        match result {
            Ok(found) => {
                self.cache.insert(track, found.clone());
                if !current {
                    return Vec::new();
                }
                let effects = describe(&found);
                self.state = LyricsState::Ready(found);
                effects
            }
            Err(e) if current => {
                self.state = LyricsState::Failed;
                vec![Effect::Print(format!("  lyrics lookup failed: {e}"))]
            }
            Err(_) => Vec::new(),
        }
    }

    pub fn status(&self, snapshot: Option<&PlayerSnapshot>, now_ms: i64) -> String {
        let Some(s) = snapshot else {
            return "waiting for Spotify…".into();
        };
        let lyrics = match &self.state {
            LyricsState::Idle => "",
            LyricsState::Loading => "lyrics: loading…",
            LyricsState::Failed => "lyrics: error",
            LyricsState::Ready(None) => "lyrics: none",
            LyricsState::Ready(Some(f)) => match f.lyrics {
                Lyrics::Synced(_) => "lyrics: synced",
                Lyrics::Plain(_) => "lyrics: unsynced",
                Lyrics::Instrumental => "lyrics: instrumental",
            },
        };
        let offset = if self.offset_ms != 0 {
            format!("  offset {:+}ms", self.offset_ms)
        } else {
            String::new()
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
}

/// Prints line `index` together with any lines sharing its timestamp
/// (typically original + translation).
fn print_group(synced: &SyncedLyrics, index: usize) -> Vec<Effect> {
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
            Effect::Print(format!("  [{}] {}", format_ms(l.start_ms), text))
        })
        .collect()
}

/// One-time summary when lyrics become available for the current track.
fn describe(found: &Option<FoundLyrics>) -> Vec<Effect> {
    let Some(f) = found else {
        return vec![Effect::Print("  no lyrics found".into())];
    };
    let matched = format!("matched “{}” by {}", f.matched_title, f.matched_artist);
    match &f.lyrics {
        Lyrics::Synced(s) => vec![Effect::Print(format!(
            "  synced lyrics from {} ({} lines, {matched})",
            f.source,
            s.lines().len()
        ))],
        Lyrics::Plain(text) => {
            let mut effects = vec![Effect::Print(format!(
                "  only unsynced lyrics on {} ({matched}):",
                f.source
            ))];
            effects.extend(text.lines().map(|l| Effect::Print(format!("    {l}"))));
            effects
        }
        Lyrics::Instrumental => vec![Effect::Print(format!(
            "  instrumental according to {}",
            f.source
        ))],
    }
}

type FetchResult = (TrackInfo, Result<Option<FoundLyrics>, String>);

pub fn run(
    source: &impl PlayerSource,
    provider: Arc<dyn LyricsProvider>,
    args: &WatchArgs,
) -> anyhow::Result<()> {
    let mut screen = Screen::new(io::stdout());
    let mut watcher = Watcher::new(args.settle_ms, args.offset_ms);
    let (tx, rx) = mpsc::channel::<FetchResult>();

    screen.line("Following Spotify. Lyrics from LRCLIB (lrclib.net). Ctrl+C to stop.")?;

    loop {
        let now = now_unix_ms();
        let snapshot = source.snapshot().unwrap_or_else(|e| {
            // Sessions can vanish between calls while Spotify starts or quits.
            let _ = screen.line(&format!("SMTC error: {e}"));
            None
        });

        let mut effects = watcher.tick(snapshot.as_ref(), now);
        while let Ok((track, result)) = rx.try_recv() {
            effects.extend(watcher.on_fetched(track, result));
        }

        for effect in effects {
            match effect {
                Effect::Print(text) => screen.line(&text)?,
                Effect::Fetch(track) => {
                    let provider = Arc::clone(&provider);
                    let tx = tx.clone();
                    thread::spawn(move || {
                        let result = provider.fetch(&track).map_err(|e| e.to_string());
                        // The receiver only goes away when we exit.
                        let _ = tx.send((track, result));
                    });
                }
            }
        }

        screen.status(&watcher.status(snapshot.as_ref(), now))?;
        thread::sleep(Duration::from_millis(args.interval_ms));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lyrics_core::player::PlaybackStatus;

    const LRC: &str = "[00:01.00]line one\n[00:03.00]line two\n[00:03.00]translation two\n\
                       [00:05.00]\n[00:06.00]line four";

    fn track(title: &str) -> TrackInfo {
        TrackInfo {
            title: title.into(),
            artist: "Artist".into(),
            album: String::new(),
            duration_ms: 200_000,
        }
    }

    /// Playing, with position `pos` reported at time `at`.
    fn playing(t: &TrackInfo, pos: u64, at: i64) -> PlayerSnapshot {
        PlayerSnapshot {
            track: t.clone(),
            status: PlaybackStatus::Playing,
            reported_position_ms: pos,
            position_updated_unix_ms: at,
        }
    }

    fn found(lyrics: Lyrics) -> Option<FoundLyrics> {
        Some(FoundLyrics {
            lyrics,
            source: "LRCLIB",
            matched_title: "Song".into(),
            matched_artist: "Artist".into(),
            matched_duration_ms: Some(200_000),
        })
    }

    fn synced() -> Option<FoundLyrics> {
        found(Lyrics::Synced(SyncedLyrics::parse(LRC)))
    }

    fn printed(effects: &[Effect]) -> Vec<&str> {
        effects
            .iter()
            .filter_map(|e| match e {
                Effect::Print(s) => Some(s.as_str()),
                Effect::Fetch(_) => None,
            })
            .collect()
    }

    /// Settles `t` (playing from position 0 at time 0) and delivers `result`.
    fn settled_with(t: &TrackInfo, result: Option<FoundLyrics>) -> Watcher {
        let mut w = Watcher::new(1_000, 0);
        // Track starts at time 0; settles after 1 s at position ~1 s.
        assert!(w.tick(Some(&playing(t, 0, 0)), 0).is_empty());
        let effects = w.tick(Some(&playing(t, 0, 0)), 1_000);
        assert!(effects.contains(&Effect::Fetch(t.clone())));
        w.on_fetched(t.clone(), Ok(result));
        w
    }

    #[test]
    fn fetches_once_track_settles() {
        let t = track("Song");
        let mut w = Watcher::new(1_000, 0);
        assert!(w.tick(Some(&playing(&t, 0, 0)), 0).is_empty());
        assert!(w.tick(Some(&playing(&t, 0, 0)), 500).is_empty());
        let effects = w.tick(Some(&playing(&t, 0, 0)), 1_000);
        assert_eq!(
            effects,
            vec![
                Effect::Print(String::new()),
                Effect::Print("♪ Song — Artist".into()),
                Effect::Fetch(t.clone()),
            ]
        );
        assert!(
            w.status(Some(&playing(&t, 0, 0)), 1_000)
                .contains("loading")
        );
    }

    #[test]
    fn prints_each_line_once_as_playback_reaches_it() {
        let t = track("Song");
        let mut w = settled_with(&t, synced());
        let snap = playing(&t, 0, 0);

        // Fetch finished at ~1 s: line one (start 1.0 s) is current.
        assert_eq!(
            printed(&w.tick(Some(&snap), 1_100)),
            vec!["  [00:01.0] line one"]
        );
        assert!(w.tick(Some(&snap), 2_900).is_empty());
        // Original and translation share a timestamp and print together.
        assert_eq!(
            printed(&w.tick(Some(&snap), 3_000)),
            vec!["  [00:03.0] line two", "  [00:03.0] translation two"]
        );
        // A blank line marks an instrumental gap.
        assert_eq!(printed(&w.tick(Some(&snap), 5_100)), vec!["  [00:05.0] ♪"]);
        assert_eq!(
            printed(&w.tick(Some(&snap), 6_000)),
            vec!["  [00:06.0] line four"]
        );
        assert!(w.tick(Some(&snap), 9_000).is_empty());
    }

    #[test]
    fn seeking_back_reprints_from_new_position() {
        let t = track("Song");
        let mut w = settled_with(&t, synced());
        w.tick(Some(&playing(&t, 0, 0)), 6_500);
        // Seek back to 1.2 s.
        let after_seek = playing(&t, 1_200, 7_000);
        assert_eq!(
            printed(&w.tick(Some(&after_seek), 7_000)),
            vec!["  [00:01.0] line one"]
        );
        // Seek before the first line: nothing to print.
        assert!(w.tick(Some(&playing(&t, 0, 8_000)), 8_000).is_empty());
    }

    #[test]
    fn offset_shifts_lines_earlier() {
        let t = track("Song");
        let mut w = Watcher::new(1_000, 500);
        w.tick(Some(&playing(&t, 0, 0)), 0);
        w.tick(Some(&playing(&t, 0, 0)), 1_000);
        w.on_fetched(t.clone(), Ok(synced()));
        // At 2.5 s playback, +500 ms offset reaches line two at 3.0 s.
        let effects = w.tick(Some(&playing(&t, 0, 0)), 2_500);
        assert_eq!(printed(&effects)[0], "  [00:03.0] line two");
        assert!(
            w.status(Some(&playing(&t, 0, 0)), 2_500)
                .contains("offset +500ms")
        );
    }

    #[test]
    fn ignores_snapshots_that_mix_two_tracks() {
        let t = track("Song");
        let mut w = settled_with(&t, synced());
        w.tick(Some(&playing(&t, 0, 0)), 1_100);
        // Spotify updated the duration before the title.
        let mut hybrid = playing(&t, 0, 2_000);
        hybrid.track.duration_ms = 150_000;
        assert!(w.tick(Some(&hybrid), 4_000).is_empty());
    }

    #[test]
    fn late_result_for_previous_track_is_cached_not_shown() {
        let a = track("A");
        let b = track("B");
        let mut w = Watcher::new(1_000, 0);
        w.tick(Some(&playing(&a, 0, 0)), 0);
        w.tick(Some(&playing(&a, 0, 0)), 1_000); // fetch A
        w.tick(Some(&playing(&b, 0, 2_000)), 2_000);
        w.tick(Some(&playing(&b, 0, 2_000)), 3_000); // fetch B
        assert!(w.on_fetched(a.clone(), Ok(synced())).is_empty());
        assert!(
            w.status(Some(&playing(&b, 0, 2_000)), 3_000)
                .contains("loading")
        );

        // Back to A: served from cache, no new fetch.
        w.tick(Some(&playing(&a, 0, 4_000)), 4_000);
        let effects = w.tick(Some(&playing(&a, 0, 4_000)), 5_000);
        assert!(!effects.iter().any(|e| matches!(e, Effect::Fetch(_))));
        assert!(
            printed(&effects)
                .iter()
                .any(|l| l.contains("synced lyrics from LRCLIB"))
        );
    }

    #[test]
    fn describes_plain_instrumental_and_missing() {
        let t = track("Song");
        let w = settled_with(&t, found(Lyrics::Instrumental));
        assert!(
            w.status(Some(&playing(&t, 0, 0)), 1_000)
                .contains("instrumental")
        );

        let mut w = Watcher::new(1_000, 0);
        w.tick(Some(&playing(&t, 0, 0)), 0);
        w.tick(Some(&playing(&t, 0, 0)), 1_000);
        let effects = w.on_fetched(t.clone(), Ok(found(Lyrics::Plain("a\nb".into()))));
        assert_eq!(printed(&effects)[1..], ["    a", "    b"]);

        let mut w = Watcher::new(1_000, 0);
        w.tick(Some(&playing(&t, 0, 0)), 0);
        w.tick(Some(&playing(&t, 0, 0)), 1_000);
        assert_eq!(
            printed(&w.on_fetched(t.clone(), Ok(None))),
            vec!["  no lyrics found"]
        );
    }

    #[test]
    fn errors_are_reported_and_not_cached() {
        let t = track("Song");
        let mut w = Watcher::new(1_000, 0);
        w.tick(Some(&playing(&t, 0, 0)), 0);
        w.tick(Some(&playing(&t, 0, 0)), 1_000);
        let effects = w.on_fetched(t.clone(), Err("timeout".into()));
        assert_eq!(printed(&effects), vec!["  lyrics lookup failed: timeout"]);
        assert!(!w.cache.contains_key(&t));
    }

    #[test]
    fn player_closing_clears_state() {
        let t = track("Song");
        let mut w = settled_with(&t, synced());
        w.tick(None, 2_000);
        let effects = w.tick(None, 3_000);
        assert_eq!(printed(&effects), vec!["", "(nothing playing)"]);
        assert_eq!(w.status(None, 3_000), "waiting for Spotify…");
    }
}
