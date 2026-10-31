//! Calls the real LRCLIB API. Ignored by default; run with
//!
//! ```sh
//! cargo test -p lyrics-core --test lrclib_live -- --ignored --nocapture
//! ```
//!
//! Only metadata and match results are printed, never lyrics text.

use lyrics_core::lyrics::lrclib::Lrclib;
use lyrics_core::lyrics::{Lyrics, LyricsProvider};
use lyrics_core::player::TrackInfo;

fn track(title: &str, artist: &str, album: &str, duration_ms: u64) -> TrackInfo {
    TrackInfo {
        title: title.into(),
        artist: artist.into(),
        album: album.into(),
        duration_ms,
    }
}

fn describe(lyrics: &Lyrics) -> String {
    match lyrics {
        Lyrics::Synced(s) => format!("synced, {} lines", s.lines().len()),
        Lyrics::Plain(p) => format!("plain only, {} lines", p.lines().count()),
        Lyrics::Instrumental => "instrumental".into(),
    }
}

#[test]
#[ignore = "network"]
fn finds_synced_lyrics_through_title_cleanup() {
    // Spotify-style metadata: remaster suffix in the title and album.
    let t = track(
        "Bohemian Rhapsody - Remastered 2011",
        "Queen",
        "A Night At The Opera (2011 Remaster)",
        354_320,
    );
    let found = Lrclib::new()
        .fetch(&t)
        .expect("request failed")
        .expect("no lyrics found");
    report(&format!(
        "{} — {} -> {} (matched {:?} by {:?}, {:?} ms)",
        t.title,
        t.artist,
        describe(&found.lyrics),
        found.matched_title,
        found.matched_artist,
        found.matched_duration_ms,
    ));
    assert!(found.lyrics.is_synced());
}

/// Coverage report for the tracks recorded in the first `drift.csv`.
/// Prints results; doesn't assert, since coverage is what we're measuring.
#[test]
#[ignore = "network"]
fn report_coverage_for_sample_tracks() {
    let provider = Lrclib::new();
    let tracks = [
        track("TIMEBOMB", "izna", "", 171_232),
        track("ETA", "NewJeans", "", 151_373),
        track("plot twist", "TWS", "", 152_400),
        track("LOVE ATTACK", "RESCENE", "", 181_560),
        track("Brought The Heat Back", "ENHYPEN", "", 176_424),
    ];
    for t in &tracks {
        let result = match provider.fetch(t) {
            Ok(Some(f)) => format!(
                "{} (matched {:?}, {:?} ms)",
                describe(&f.lyrics),
                f.matched_title,
                f.matched_duration_ms
            ),
            Ok(None) => "not found".into(),
            Err(e) => format!("error: {e}"),
        };
        report(&format!("{} — {} -> {result}", t.title, t.artist));
    }
}

/// Prints a result line. Under GitHub Actions it is also emitted as a
/// notice annotation, which shows on the PR and is readable through the
/// checks API (job logs themselves are only downloadable from a browser).
fn report(line: &str) {
    println!("{line}");
    if std::env::var_os("GITHUB_ACTIONS").is_some() {
        println!("::notice title=LRCLIB live::{line}");
    }
}
