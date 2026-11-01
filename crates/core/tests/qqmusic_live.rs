//! Calls the real QQ Music endpoints. Ignored by default; run with
//!
//! ```sh
//! cargo test -p lyrics-core --test qqmusic_live -- --ignored --nocapture
//! ```
//!
//! Only metadata and match results are printed, never lyrics text.

use lyrics_core::lyrics::qqmusic::QqMusic;
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
            format!("synced, {} lines ({shared} translated)", lines.len())
        }
        Lyrics::Plain(p) => format!("plain only, {} lines", p.lines().count()),
        Lyrics::Instrumental => "instrumental".into(),
    }
}

#[test]
#[ignore = "network"]
fn report_coverage_for_sample_tracks() {
    let provider = QqMusic::new();
    let tracks = [
        track("TIMEBOMB", "izna", 171_232),
        track("ETA", "NewJeans", 151_373),
        track("plot twist", "TWS", 152_400),
        track("LOVE ATTACK", "RESCENE", 181_560),
        track("Brought The Heat Back", "ENHYPEN", 176_424),
        track("Supernatural", "NewJeans", 191_000),
        track("Whiplash", "aespa", 183_000),
        track("Magnetic", "ILLIT", 160_000),
        track("HOT", "LE SSERAFIM", 143_000),
        track("APT.", "ROSÉ, Bruno Mars", 169_000),
        track("晴天", "周杰伦", 269_000),
        track("Lemon", "米津玄師", 255_000),
    ];
    let mut lines = Vec::new();
    let mut found = 0;
    for t in &tracks {
        let result = match provider.fetch(t) {
            Ok(Some(f)) => {
                found += 1;
                format!(
                    "{} (matched {:?} by {:?})",
                    describe(&f.lyrics),
                    f.matched_title,
                    f.matched_artist
                )
            }
            Ok(None) => "not found".into(),
            Err(e) => format!("error: {e}"),
        };
        let line = format!("{} — {} -> {result}", t.title, t.artist);
        println!("{line}");
        lines.push(line);
    }
    lines.push(format!("found {found} of {}", tracks.len()));
    // One annotation: GitHub shows at most ten per step.
    if std::env::var_os("GITHUB_ACTIONS").is_some() {
        println!("::notice title=QQ Music live::{}", lines.join(" ‖ "));
    }
    assert!(found > 0, "nothing found at all: the API may have changed");
}
