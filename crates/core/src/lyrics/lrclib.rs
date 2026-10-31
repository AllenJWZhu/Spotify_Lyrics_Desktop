//! [LRCLIB](https://lrclib.net) provider.
//!
//! How the server matches (read from its source, `get_track_by_metadata`):
//! - `/api/get` compares *normalized* track and artist names exactly
//!   (lowercased, punctuation → spaces, apostrophes removed), duration
//!   within ±2 s, and `album_name` exactly **if given**. Spotify album names
//!   often differ ("- Single", "Deluxe"), so we retry without the album.
//! - When several entries match, `/api/get` returns the lowest id, not
//!   necessarily one with synced lyrics, so a plain-only hit is followed by a
//!   `/api/search` for a synced alternative.

use std::time::Duration;

use serde::Deserialize;

use super::query::{clean_title, primary_artist};
use super::{FoundLyrics, Lyrics, LyricsError, LyricsProvider};
use crate::player::TrackInfo;

pub const DEFAULT_BASE_URL: &str = "https://lrclib.net";

/// LRCLIB asks clients to identify themselves.
pub const DEFAULT_USER_AGENT: &str = concat!(
    "Spotify_Lyrics_Desktop/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/AllenJWZhu/Spotify_Lyrics_Desktop)"
);

/// The server's own matching window for `/api/get`.
const DURATION_TOLERANCE_MS: u64 = 2_000;

/// The server rejects durations outside 1..=3600 s.
const VALID_DURATION_MS: std::ops::RangeInclusive<u64> = 1_000..=3_600_000;

/// One track entry as returned by `/api/get` and `/api/search`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Record {
    pub id: i64,
    #[serde(default)]
    pub track_name: Option<String>,
    #[serde(default)]
    pub artist_name: Option<String>,
    #[serde(default)]
    pub album_name: Option<String>,
    /// Seconds.
    #[serde(default)]
    pub duration: Option<f64>,
    #[serde(default)]
    pub instrumental: bool,
    #[serde(default)]
    pub plain_lyrics: Option<String>,
    #[serde(default)]
    pub synced_lyrics: Option<String>,
}

impl Record {
    fn lyrics(&self) -> Option<Lyrics> {
        Lyrics::from_parts(
            self.synced_lyrics.as_deref(),
            self.plain_lyrics.as_deref(),
            self.instrumental,
        )
    }

    fn duration_ms(&self) -> Option<u64> {
        self.duration
            .filter(|d| d.is_finite() && *d >= 0.0)
            .map(|d| (d * 1000.0).round() as u64)
    }

    fn into_found(self, lyrics: Lyrics) -> FoundLyrics {
        FoundLyrics {
            lyrics,
            source: "LRCLIB".into(),
            matched_duration_ms: self.duration_ms(),
            matched_title: self.track_name.unwrap_or_default(),
            matched_artist: self.artist_name.unwrap_or_default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct GetQuery {
    pub track_name: String,
    pub artist_name: String,
    pub album_name: Option<String>,
    pub duration_s: Option<f64>,
}

/// The two LRCLIB endpoints we use. Abstracted so the lookup strategy can
/// be tested without the network.
pub trait Transport: Send + Sync {
    /// `Ok(None)` on 404.
    fn get(&self, query: &GetQuery) -> Result<Option<Record>, LyricsError>;
    fn search(&self, track_name: &str, artist_name: &str) -> Result<Vec<Record>, LyricsError>;
}

/// Waits before each retry of a transient failure. LRCLIB answers 503 when
/// it sheds load, which a CI run hit on the very first request.
const DEFAULT_RETRY_DELAYS: [Duration; 2] = [Duration::from_secs(1), Duration::from_secs(3)];

/// Failures worth retrying: server overload or a flaky connection. Client
/// errors (400, 404) and malformed responses are not.
fn is_transient(e: &ureq::Error) -> bool {
    matches!(
        e,
        ureq::Error::StatusCode(429 | 500 | 502 | 503 | 504)
            | ureq::Error::Timeout(_)
            | ureq::Error::Io(_)
            | ureq::Error::ConnectionFailed
    )
}

/// Real HTTP transport.
pub struct HttpTransport {
    agent: ureq::Agent,
    base_url: String,
    retry_delays: Vec<Duration>,
}

impl HttpTransport {
    pub fn new(user_agent: &str) -> Self {
        Self::with_base_url(user_agent, DEFAULT_BASE_URL)
    }

    /// Overrides the waits between retries; an empty list disables retries.
    pub fn with_retry_delays(mut self, delays: Vec<Duration>) -> Self {
        self.retry_delays = delays;
        self
    }

    /// Runs `attempt`, retrying transient failures after each delay in turn.
    fn with_retries<T>(
        &self,
        mut attempt: impl FnMut() -> Result<T, ureq::Error>,
    ) -> Result<T, ureq::Error> {
        let mut delays = self.retry_delays.iter();
        loop {
            match attempt() {
                Err(e) if is_transient(&e) => match delays.next() {
                    Some(delay) => std::thread::sleep(*delay),
                    None => return Err(e),
                },
                result => return result,
            }
        }
    }

    pub fn with_base_url(user_agent: &str, base_url: &str) -> Self {
        let tls = ureq::tls::TlsConfig::builder()
            // OS trust store: keeps working behind antivirus or corporate
            // TLS inspection, which bundled roots would reject.
            .root_certs(ureq::tls::RootCerts::PlatformVerifier)
            .build();
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .user_agent(user_agent)
            .timeout_global(Some(Duration::from_secs(10)))
            .tls_config(tls)
            .build()
            .into();
        Self {
            agent,
            base_url: base_url.trim_end_matches('/').to_string(),
            retry_delays: DEFAULT_RETRY_DELAYS.to_vec(),
        }
    }
}

impl Transport for HttpTransport {
    fn get(&self, query: &GetQuery) -> Result<Option<Record>, LyricsError> {
        let result = self.with_retries(|| {
            let mut request = self
                .agent
                .get(format!("{}/api/get", self.base_url))
                .query("track_name", &query.track_name)
                .query("artist_name", &query.artist_name);
            if let Some(album) = &query.album_name {
                request = request.query("album_name", album);
            }
            if let Some(duration) = query.duration_s {
                request = request.query("duration", format!("{duration:.3}"));
            }
            request.call()?.body_mut().read_json::<Record>()
        });
        match result {
            Ok(record) => Ok(Some(record)),
            Err(ureq::Error::StatusCode(404)) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn search(&self, track_name: &str, artist_name: &str) -> Result<Vec<Record>, LyricsError> {
        let records = self.with_retries(|| {
            self.agent
                .get(format!("{}/api/search", self.base_url))
                .query("track_name", track_name)
                .query("artist_name", artist_name)
                .call()?
                .body_mut()
                .read_json::<Vec<Record>>()
        })?;
        Ok(records)
    }
}

pub struct Lrclib<T: Transport = HttpTransport> {
    transport: T,
}

impl Lrclib<HttpTransport> {
    pub fn new() -> Self {
        Self::with_transport(HttpTransport::new(DEFAULT_USER_AGENT))
    }
}

impl Default for Lrclib<HttpTransport> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Transport> Lrclib<T> {
    pub fn with_transport(transport: T) -> Self {
        Self { transport }
    }
}

impl<T: Transport> LyricsProvider for Lrclib<T> {
    fn fetch(&self, track: &TrackInfo) -> Result<Option<FoundLyrics>, LyricsError> {
        let title = track.title.trim();
        let artist = track.artist.trim();
        let album = track.album.trim();
        let clean = clean_title(title);
        let main_artist = primary_artist(artist);
        let duration_s = VALID_DURATION_MS
            .contains(&track.duration_ms)
            .then(|| track.duration_ms as f64 / 1000.0);

        let query = |t: &str, a: &str, album: Option<&str>| GetQuery {
            track_name: t.to_string(),
            artist_name: a.to_string(),
            album_name: album.map(str::to_string),
            duration_s,
        };
        let mut attempts = Vec::with_capacity(3);
        if !album.is_empty() {
            attempts.push(query(title, artist, Some(album)));
        }
        attempts.push(query(title, artist, None));
        if clean != title || main_artist != artist {
            attempts.push(query(&clean, &main_artist, None));
        }

        // A plain-only or instrumental hit, kept while we look for synced.
        let mut fallback: Option<(Record, Lyrics)> = None;
        for attempt in &attempts {
            let Some(record) = self.transport.get(attempt)? else {
                continue;
            };
            match record.lyrics() {
                Some(lyrics) if lyrics.is_synced() => return Ok(Some(record.into_found(lyrics))),
                Some(lyrics) if fallback.is_none() => fallback = Some((record, lyrics)),
                _ => {}
            }
        }

        let results = self.transport.search(&clean, &main_artist)?;
        let best = best_candidate(results, track.duration_ms);
        let chosen = match (best, fallback) {
            (Some(best), _) if best.1.is_synced() => Some(best),
            (best, fallback) => fallback.or(best),
        };
        Ok(chosen.map(|(record, lyrics)| record.into_found(lyrics)))
    }
}

/// Picks the best search result: within the duration window (when we know
/// the duration), then synced over plain over instrumental, then the
/// server's ranking order.
fn best_candidate(results: Vec<Record>, duration_ms: u64) -> Option<(Record, Lyrics)> {
    let rank = |l: &Lyrics| match l {
        Lyrics::Synced(_) => 2,
        Lyrics::Plain(_) => 1,
        Lyrics::Instrumental => 0,
    };
    results
        .into_iter()
        .filter(|r| {
            duration_ms == 0
                || r.duration_ms()
                    .is_some_and(|d| d.abs_diff(duration_ms) <= DURATION_TOLERANCE_MS)
        })
        .filter_map(|r| {
            let lyrics = r.lyrics()?;
            Some((r, lyrics))
        })
        // `max_by_key` keeps the last maximum; reverse so ties go to the
        // earliest (best-ranked) result.
        .rev()
        .max_by_key(|(_, l)| rank(l))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    const SYNCED: &str = "[00:01.00]made up line one\n[00:03.00]made up line two";

    fn record(id: i64, duration_s: f64, synced: bool, plain: bool) -> Record {
        Record {
            id,
            track_name: Some(format!("Track {id}")),
            artist_name: Some("Artist".into()),
            album_name: None,
            duration: Some(duration_s),
            instrumental: !synced && !plain,
            plain_lyrics: plain.then(|| "made up line one\nmade up line two".into()),
            synced_lyrics: synced.then(|| SYNCED.into()),
        }
    }

    /// Scripted transport: `get` answers by (track, artist, album) key and
    /// every call is logged.
    #[derive(Default)]
    struct Fake {
        gets: HashMap<(String, String, Option<String>), Record>,
        search: Vec<Record>,
        fail: bool,
        log: Mutex<Vec<String>>,
    }

    impl Fake {
        fn on_get(mut self, t: &str, a: &str, album: Option<&str>, r: Record) -> Self {
            self.gets
                .insert((t.into(), a.into(), album.map(Into::into)), r);
            self
        }
        fn on_search(mut self, results: Vec<Record>) -> Self {
            self.search = results;
            self
        }
        fn calls(&self) -> Vec<String> {
            self.log.lock().unwrap().clone()
        }
    }

    impl Transport for Fake {
        fn get(&self, q: &GetQuery) -> Result<Option<Record>, LyricsError> {
            self.log.lock().unwrap().push(format!(
                "get {}|{}|{}|{:?}",
                q.track_name,
                q.artist_name,
                q.album_name.as_deref().unwrap_or("-"),
                q.duration_s
            ));
            if self.fail {
                return Err(LyricsError::Other("boom".into()));
            }
            let key = (
                q.track_name.clone(),
                q.artist_name.clone(),
                q.album_name.clone(),
            );
            Ok(self.gets.get(&key).cloned())
        }
        fn search(&self, t: &str, a: &str) -> Result<Vec<Record>, LyricsError> {
            self.log.lock().unwrap().push(format!("search {t}|{a}"));
            Ok(self.search.clone())
        }
    }

    fn track(title: &str, artist: &str, album: &str, duration_ms: u64) -> TrackInfo {
        TrackInfo {
            title: title.into(),
            artist: artist.into(),
            album: album.into(),
            duration_ms,
        }
    }

    fn fetch(fake: &Fake, t: &TrackInfo) -> Option<FoundLyrics> {
        // `Lrclib` takes the transport by value; borrow it through a wrapper.
        struct Borrowed<'a>(&'a Fake);
        impl Transport for Borrowed<'_> {
            fn get(&self, q: &GetQuery) -> Result<Option<Record>, LyricsError> {
                self.0.get(q)
            }
            fn search(&self, t: &str, a: &str) -> Result<Vec<Record>, LyricsError> {
                self.0.search(t, a)
            }
        }
        Lrclib::with_transport(Borrowed(fake)).fetch(t).unwrap()
    }

    #[test]
    fn exact_match_with_album_needs_one_request() {
        let fake = Fake::default().on_get(
            "Song",
            "Artist",
            Some("Album"),
            record(1, 200.0, true, true),
        );
        let found = fetch(&fake, &track("Song", "Artist", "Album", 200_000)).unwrap();
        assert!(found.lyrics.is_synced());
        assert_eq!(found.source, "LRCLIB");
        assert_eq!(found.matched_duration_ms, Some(200_000));
        assert_eq!(fake.calls(), vec!["get Song|Artist|Album|Some(200.0)"]);
    }

    #[test]
    fn retries_without_album() {
        let fake = Fake::default().on_get("Song", "Artist", None, record(1, 200.0, true, false));
        let found = fetch(&fake, &track("Song", "Artist", "Song - Single", 200_000)).unwrap();
        assert!(found.lyrics.is_synced());
        assert_eq!(fake.calls().len(), 2);
    }

    #[test]
    fn falls_back_to_cleaned_title_and_primary_artist() {
        let fake = Fake::default().on_get("Song", "Main", None, record(1, 200.0, true, false));
        let t = track("Song - Remastered 2011", "Main, Guest", "", 200_000);
        assert!(fetch(&fake, &t).unwrap().lyrics.is_synced());
        assert_eq!(
            fake.calls(),
            vec![
                "get Song - Remastered 2011|Main, Guest|-|Some(200.0)",
                "get Song|Main|-|Some(200.0)",
            ]
        );
    }

    #[test]
    fn plain_hit_is_upgraded_by_synced_search_result() {
        let fake = Fake::default()
            .on_get("Song", "Artist", None, record(1, 200.0, false, true))
            .on_search(vec![
                record(2, 230.0, true, true), // wrong length
                record(3, 201.5, true, true), // within ±2 s
            ]);
        let found = fetch(&fake, &track("Song", "Artist", "", 200_000)).unwrap();
        assert!(found.lyrics.is_synced());
        assert_eq!(found.matched_title, "Track 3");
    }

    #[test]
    fn keeps_plain_hit_when_search_has_nothing_better() {
        let fake = Fake::default()
            .on_get("Song", "Artist", None, record(1, 200.0, false, true))
            .on_search(vec![record(2, 260.0, true, true)]);
        let found = fetch(&fake, &track("Song", "Artist", "", 200_000)).unwrap();
        assert!(matches!(found.lyrics, Lyrics::Plain(_)));
        assert_eq!(found.matched_title, "Track 1");
    }

    #[test]
    fn search_prefers_synced_then_server_order() {
        let fake = Fake::default().on_search(vec![
            record(1, 200.0, false, true),
            record(2, 200.0, true, false),
            record(3, 199.0, true, false),
        ]);
        let found = fetch(&fake, &track("Song", "Artist", "", 200_000)).unwrap();
        assert_eq!(found.matched_title, "Track 2");
    }

    #[test]
    fn reports_instrumental() {
        let fake = Fake::default().on_get("Song", "Artist", None, record(1, 200.0, false, false));
        let found = fetch(&fake, &track("Song", "Artist", "", 200_000)).unwrap();
        assert_eq!(found.lyrics, Lyrics::Instrumental);
    }

    #[test]
    fn nothing_found() {
        let fake = Fake::default();
        assert_eq!(
            fetch(&fake, &track("Song", "Artist", "Album", 200_000)),
            None
        );
        // get with album, get without, then search.
        assert_eq!(fake.calls().len(), 3);
    }

    #[test]
    fn unknown_duration_is_not_sent_and_not_filtered() {
        let fake = Fake::default().on_search(vec![record(1, 999.0, true, false)]);
        let found = fetch(&fake, &track("Song", "Artist", "", 0)).unwrap();
        assert!(found.lyrics.is_synced());
        assert_eq!(fake.calls()[0], "get Song|Artist|-|None");
    }

    #[test]
    fn transport_errors_propagate() {
        let fake = Fake {
            fail: true,
            ..Fake::default()
        };
        let err = Lrclib::with_transport(fake)
            .fetch(&track("Song", "Artist", "", 200_000))
            .unwrap_err();
        assert_eq!(err.to_string(), "boom");
    }

    #[test]
    fn deserializes_server_response_shape() {
        // Field set copied from the server's TrackResponse; values made up.
        let json = r#"{
            "id": 42, "name": "Made Up", "trackName": "Made Up", "artistName": "Nobody",
            "albumName": null, "duration": 171.232, "instrumental": false,
            "hasWordSync": false, "plainLyrics": "a\nb",
            "syncedLyrics": "[00:01.00]a\n[00:02.00]b", "lyricsfile": null
        }"#;
        let r: Record = serde_json::from_str(json).unwrap();
        assert_eq!(r.id, 42);
        assert_eq!(r.duration_ms(), Some(171_232));
        assert!(r.lyrics().unwrap().is_synced());

        let minimal: Record = serde_json::from_str(r#"{"id": 1}"#).unwrap();
        assert_eq!(minimal.lyrics(), None);
    }
}
