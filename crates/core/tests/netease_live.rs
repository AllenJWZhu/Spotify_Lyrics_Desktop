//! Calls the real NetEase endpoints. Ignored by default; run with
//!
//! ```sh
//! cargo test -p lyrics-core --test netease_live -- --ignored --nocapture
//! ```
//!
//! Only metadata and match results are printed, never lyrics text.

use lyrics_core::lyrics::lrclib::{DEFAULT_USER_AGENT, HttpTransport};
use lyrics_core::lyrics::netease::{Api, DEFAULT_BASE_URL, NetEase};
use lyrics_core::lyrics::{Lyrics, LyricsProvider};
use lyrics_core::player::TrackInfo;

fn track(title: &str, artist: &str, duration_ms: u64) -> TrackInfo {
    TrackInfo {
        title: title.into(),
        artist: artist.into(),
        album: String::new(),
        duration_ms,
    }
}

fn describe(lyrics: &Lyrics) -> String {
    match lyrics {
        Lyrics::Synced(s) => {
            let lines = s.lines();
            let shared = lines
                .windows(2)
                .filter(|w| w[0].start_ms == w[1].start_ms)
                .count();
            format!(
                "synced, {} lines ({shared} with a translation)",
                lines.len()
            )
        }
        Lyrics::Plain(p) => format!("plain only, {} lines", p.lines().count()),
        Lyrics::Instrumental => "instrumental".into(),
    }
}

/// Coverage for the same tracks as the LRCLIB report, plus a few others.
#[test]
#[ignore = "network"]
fn report_coverage_for_sample_tracks() {
    let provider = NetEase::new();
    let tracks = [
        track("TIMEBOMB", "izna", 171_232),
        track("ETA", "NewJeans", 151_373),
        track("plot twist", "TWS", 152_400),
        track("LOVE ATTACK", "RESCENE", 181_560),
        track("Brought The Heat Back", "ENHYPEN", 176_424),
        track("Bohemian Rhapsody - Remastered 2011", "Queen", 354_320),
        track("晴天", "周杰伦", 269_000),
        track("Lemon", "米津玄師", 255_000),
    ];
    let mut found = 0;
    for t in &tracks {
        let result = match provider.fetch(t) {
            Ok(Some(f)) => {
                found += 1;
                format!(
                    "{} (matched {:?} by {:?}, {:?} ms)",
                    describe(&f.lyrics),
                    f.matched_title,
                    f.matched_artist,
                    f.matched_duration_ms
                )
            }
            Ok(None) => format!("not found; NetEase had {}", candidates(t)),
            Err(e) => format!("error: {e}"),
        };
        report(&format!("{} — {} -> {result}", t.title, t.artist));
    }
    assert!(found > 0, "nothing found at all: the API may have changed");
}

/// The first few search results, to see why nothing matched.
fn candidates(t: &TrackInfo) -> String {
    let api = HttpTransport::with_base_url(DEFAULT_USER_AGENT, DEFAULT_BASE_URL);
    match api.search(&format!("{} {}", t.title, t.artist)) {
        Ok(songs) => songs
            .iter()
            .take(6)
            .map(|s| {
                let artists: Vec<_> = s.ar.iter().map(|a| a.name.as_str()).collect();
                format!("{:?} by {:?} {:?} ms", s.name, artists.join(", "), s.dt)
            })
            .collect::<Vec<_>>()
            .join(" | "),
        Err(e) => format!("search error: {e}"),
    }
}

fn report(line: &str) {
    println!("{line}");
    if std::env::var_os("GITHUB_ACTIONS").is_some() {
        println!("::notice title=NetEase live::{line}");
    }
}
