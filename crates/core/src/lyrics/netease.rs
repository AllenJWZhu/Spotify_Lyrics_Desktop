//! NetEase Cloud Music (网易云音乐) as a backup lyrics source.
//!
//! These are the endpoints NetEase's own web player calls; they're
//! unofficial and undocumented, so they may change without notice. Used only
//! when LRCLIB has nothing synced or can't be reached, and it can be turned
//! off. Strong on Chinese, Japanese and Korean releases, and its lyrics
//! often come with a Chinese translation, which is shown under each line.
//!
//! - `/api/cloudsearch/pc?s=…&type=1` finds songs (`id`, `name`, artists
//!   `ar`, duration `dt` in ms).
//! - `/api/song/lyric?id=…&lv=-1&tv=-1` returns LRC text (`lrc.lyric`) and
//!   the translation (`tlyric.lyric`).

use serde::Deserialize;

use super::lrclib::{DEFAULT_USER_AGENT, HttpTransport};
use super::query::{clean_title, primary_artist};
use super::{FoundLyrics, Lyrics, LyricsError, LyricsProvider, Service};
use crate::player::TrackInfo;

pub const DEFAULT_BASE_URL: &str = "https://music.163.com";

/// A search result must be this close to the track's length.
const DURATION_TOLERANCE_MS: u64 = 3_000;
const SEARCH_LIMIT: u32 = 10;

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Song {
    pub id: i64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub ar: Vec<Artist>,
    /// Length in milliseconds.
    #[serde(default)]
    pub dt: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Artist {
    #[serde(default)]
    pub name: String,
}

#[derive(Debug, Deserialize)]
struct SearchResponse {
    #[serde(default)]
    code: i64,
    #[serde(default)]
    result: Option<SearchResult>,
}

#[derive(Debug, Deserialize)]
struct SearchResult {
    #[serde(default)]
    songs: Vec<Song>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct LyricResponse {
    #[serde(default)]
    pub code: i64,
    #[serde(default)]
    pub lrc: Option<LyricText>,
    #[serde(default)]
    pub tlyric: Option<LyricText>,
    /// No vocals.
    #[serde(default)]
    pub nolyric: bool,
    /// Nobody has added lyrics yet.
    #[serde(default)]
    pub uncollected: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct LyricText {
    #[serde(default)]
    pub lyric: Option<String>,
}

/// The two calls we make. Abstracted for tests.
pub trait Api: Send + Sync {
    fn search(&self, query: &str) -> Result<Vec<Song>, LyricsError>;
    fn lyric(&self, id: i64) -> Result<LyricResponse, LyricsError>;
}

fn service_error(error: ureq::Error) -> LyricsError {
    LyricsError::Service {
        service: Service::NetEase,
        error,
    }
}

fn check_code(code: i64) -> Result<(), LyricsError> {
    if code == 200 || code == 0 {
        Ok(())
    } else {
        Err(LyricsError::Other(format!(
            "NetEase answered with code {code}"
        )))
    }
}

impl Api for HttpTransport {
    fn search(&self, query: &str) -> Result<Vec<Song>, LyricsError> {
        let response = self
            .send(|agent| {
                agent
                    .get(format!("{}/api/cloudsearch/pc", self.base_url()))
                    .header("Referer", "https://music.163.com/")
                    .query("s", query)
                    .query("type", "1")
                    .query("limit", SEARCH_LIMIT.to_string())
                    .query("offset", "0")
                    .call()?
                    .body_mut()
                    .read_json::<SearchResponse>()
            })
            .map_err(service_error)?;
        check_code(response.code)?;
        Ok(response.result.map(|r| r.songs).unwrap_or_default())
    }

    fn lyric(&self, id: i64) -> Result<LyricResponse, LyricsError> {
        let response = self
            .send(|agent| {
                agent
                    .get(format!("{}/api/song/lyric", self.base_url()))
                    .header("Referer", "https://music.163.com/")
                    .query("id", id.to_string())
                    .query("lv", "-1")
                    .query("tv", "-1")
                    .call()?
                    .body_mut()
                    .read_json::<LyricResponse>()
            })
            .map_err(service_error)?;
        check_code(response.code)?;
        Ok(response)
    }
}

pub struct NetEase<A: Api = HttpTransport> {
    api: A,
}

impl NetEase<HttpTransport> {
    pub fn new() -> Self {
        let proxy = ureq::Proxy::try_from_env();
        Self::with_api(HttpTransport::with_proxy(
            DEFAULT_USER_AGENT,
            DEFAULT_BASE_URL,
            proxy,
        ))
    }
}

impl Default for NetEase<HttpTransport> {
    fn default() -> Self {
        Self::new()
    }
}

impl<A: Api> NetEase<A> {
    pub fn with_api(api: A) -> Self {
        Self { api }
    }
}

impl<A: Api> LyricsProvider for NetEase<A> {
    fn fetch(&self, track: &TrackInfo) -> Result<Option<FoundLyrics>, LyricsError> {
        let title = clean_title(track.title.trim());
        let artist = primary_artist(track.artist.trim());
        let songs = self.api.search(&format!("{title} {artist}"))?;
        let Some(song) = best_match(&songs, track) else {
            return Ok(None);
        };
        let response = self.api.lyric(song.id)?;
        let Some(lyrics) = lyrics_from(&response) else {
            return Ok(None);
        };
        Ok(Some(FoundLyrics {
            lyrics,
            source: Service::NetEase.name().into(),
            matched_title: song.name.clone(),
            matched_artist: song
                .ar
                .iter()
                .map(|a| a.name.as_str())
                .collect::<Vec<_>>()
                .join(", "),
            matched_duration_ms: song.dt,
        }))
    }
}

/// Letters and digits only, lowercased, so "Love Attack!" == "love attack".
fn normalize(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// `a` and `b` name the same thing: equal, or one contains the other and
/// is a substantial part of it (NetEase writes "TWS (투어스)" for "TWS").
fn same_name(a: &str, b: &str) -> bool {
    let (a, b) = (normalize(a), normalize(b));
    if a.is_empty() || b.is_empty() {
        return false;
    }
    a == b || ((a.contains(&b) || b.contains(&a)) && a.chars().count().min(b.chars().count()) >= 2)
}

/// The search result that is this track, if any: same title (after
/// cleanup), length within 3 s, and preferably the same artist. A title
/// match with no artist match needs a length within 1 s, for artists
/// NetEase lists under another script.
pub fn best_match<'a>(songs: &'a [Song], track: &TrackInfo) -> Option<&'a Song> {
    let title = clean_title(&track.title);
    let artist = primary_artist(&track.artist);
    songs
        .iter()
        .filter_map(|song| {
            let diff = match (song.dt, track.duration_ms) {
                (Some(dt), d) if d > 0 => dt.abs_diff(d),
                // Unknown length: only trust a full title and artist match.
                _ => u64::MAX,
            };
            if !same_name(&clean_title(&song.name), &title) {
                return None;
            }
            let artist_ok = song.ar.iter().any(|a| same_name(&a.name, &artist));
            let ok = match (artist_ok, diff) {
                (true, d) if d <= DURATION_TOLERANCE_MS => true,
                (true, u64::MAX) => true,
                (false, d) => d <= 1_000,
                _ => false,
            };
            // "Song" over "Song (Live)" when Spotify says "Song": other
            // versions are timed differently.
            let exact = normalize(&song.name) == normalize(&track.title);
            ok.then_some((song, (!artist_ok, !exact, diff)))
        })
        .min_by_key(|(_, rank)| *rank)
        .map(|(song, _)| song)
}

/// Credit lines NetEase puts at the top of its LRC ("作词 : …").
const CREDIT_PREFIXES: &[&str] = &[
    "作词",
    "作曲",
    "编曲",
    "制作人",
    "製作人",
    "监制",
    "出品",
    "发行",
    "和声",
    "混音",
    "母带",
    "录音",
    "词",
    "曲",
    "lyrics by",
    "composed by",
    "produced by",
    "arranged by",
    "written by",
];

fn is_credit(text: &str) -> bool {
    let text = text.trim();
    let Some((head, _)) = text.split_once([':', '：']) else {
        return false;
    };
    let head = head.trim().to_lowercase();
    CREDIT_PREFIXES.iter().any(|p| head == *p)
}

/// Drops credit lines (and non-LRC lines such as NetEase's JSON credit
/// blocks) from LRC text.
fn strip_credits(lrc: &str) -> String {
    lrc.lines()
        .filter(|line| {
            let text = line.rsplit(']').next().unwrap_or(line);
            !line.trim_start().starts_with('{') && !is_credit(text)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Lyrics from a lyric response; the translation, if any, becomes a second
/// line at the same time as each original line.
pub fn lyrics_from(response: &LyricResponse) -> Option<Lyrics> {
    if response.nolyric {
        return Some(Lyrics::Instrumental);
    }
    let text = |t: &Option<LyricText>| {
        t.as_ref()
            .and_then(|t| t.lyric.as_deref())
            .map(strip_credits)
            .filter(|s| !s.trim().is_empty())
    };
    let original = text(&response.lrc)?;
    if is_instrumental_note(&original) {
        return Some(Lyrics::Instrumental);
    }
    // Original first: lines sharing a timestamp keep file order.
    let combined = match text(&response.tlyric) {
        Some(translation) => format!("{original}\n{translation}"),
        None => original.clone(),
    };
    let plain: String = original
        .lines()
        .map(|l| l.rsplit(']').next().unwrap_or(l).trim())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    Lyrics::from_parts(Some(&combined), Some(&plain), false)
}

/// NetEase marks instrumentals with a single line like "纯音乐，请欣赏".
fn is_instrumental_note(lrc: &str) -> bool {
    let texts: Vec<&str> = lrc
        .lines()
        .map(|l| l.rsplit(']').next().unwrap_or(l).trim())
        .filter(|t| !t.is_empty())
        .collect();
    texts.len() <= 2 && texts.iter().any(|t| t.contains("纯音乐"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn track(title: &str, artist: &str, duration_ms: u64) -> TrackInfo {
        TrackInfo {
            title: title.into(),
            artist: artist.into(),
            album: String::new(),
            duration_ms,
        }
    }

    fn song(id: i64, name: &str, artists: &[&str], dt: u64) -> Song {
        Song {
            id,
            name: name.into(),
            ar: artists
                .iter()
                .map(|a| Artist { name: (*a).into() })
                .collect(),
            dt: Some(dt),
        }
    }

    #[test]
    fn matches_title_artist_and_length() {
        let songs = [
            song(1, "plot twist (Live)", &["TWS"], 152_400),
            song(2, "plot twist", &["Someone Else"], 190_000),
            song(3, "plot twist", &["TWS (투어스)"], 152_900),
        ];
        let t = track("plot twist", "TWS", 152_400);
        // The live version matches too once "(Live)" is cleaned off, but
        // the exact title wins over a closer length.
        assert_eq!(best_match(&songs, &t).map(|s| s.id), Some(3));
        assert_eq!(best_match(&songs[..1], &t).map(|s| s.id), Some(1));
    }

    #[test]
    fn rejects_wrong_length_and_wrong_title() {
        let t = track("ETA", "NewJeans", 151_373);
        assert!(best_match(&[song(1, "ETA", &["NewJeans"], 160_000)], &t).is_none());
        assert!(best_match(&[song(1, "Ditto", &["NewJeans"], 151_373)], &t).is_none());
        // Artist under another name: only with a near-exact length.
        assert!(best_match(&[song(1, "ETA", &["뉴진스"], 151_800)], &t).is_some());
        assert!(best_match(&[song(1, "ETA", &["뉴진스"], 153_000)], &t).is_none());
    }

    #[test]
    fn prefers_an_artist_match_over_a_closer_length() {
        let songs = [
            song(1, "Hello", &["Cover Band"], 295_500),
            song(2, "Hello", &["Adele"], 297_000),
        ];
        let t = track("Hello", "Adele", 295_500);
        assert_eq!(best_match(&songs, &t).map(|s| s.id), Some(2));
    }

    fn response(lrc: &str, tlyric: Option<&str>) -> LyricResponse {
        LyricResponse {
            code: 200,
            lrc: Some(LyricText {
                lyric: Some(lrc.into()),
            }),
            tlyric: tlyric.map(|t| LyricText {
                lyric: Some(t.into()),
            }),
            ..Default::default()
        }
    }

    #[test]
    fn drops_credits_and_adds_translation_under_each_line() {
        let lrc = "{\"t\":0,\"c\":[{\"tx\":\"作词: \"}]}\n[00:00.00] 作词 : A\n[00:00.50] 作曲 : B\n[00:12.00]Hello there\n[00:15.00]Goodbye";
        let tl = "[by:someone]\n[00:12.00]你好\n[00:15.00]再见";
        let Some(Lyrics::Synced(s)) = lyrics_from(&response(lrc, Some(tl))) else {
            panic!("expected synced lyrics");
        };
        let texts: Vec<_> = s
            .lines()
            .iter()
            .map(|l| (l.start_ms, l.text.as_str()))
            .collect();
        assert_eq!(
            texts,
            [
                (12_000, "Hello there"),
                (12_000, "你好"),
                (15_000, "Goodbye"),
                (15_000, "再见"),
            ]
        );
    }

    #[test]
    fn recognises_instrumentals_and_missing_lyrics() {
        assert_eq!(
            lyrics_from(&response("[00:00.00] 纯音乐，请欣赏", None)),
            Some(Lyrics::Instrumental)
        );
        assert_eq!(
            lyrics_from(&LyricResponse {
                nolyric: true,
                ..Default::default()
            }),
            Some(Lyrics::Instrumental)
        );
        assert_eq!(
            lyrics_from(&LyricResponse {
                uncollected: true,
                ..Default::default()
            }),
            None
        );
        assert_eq!(
            lyrics_from(&response("line one\nline two", None)),
            Some(Lyrics::Plain("line one\nline two".into()))
        );
    }

    struct FakeApi {
        songs: Vec<Song>,
        lyric: LyricResponse,
        asked: Mutex<Vec<String>>,
    }

    impl Api for FakeApi {
        fn search(&self, query: &str) -> Result<Vec<Song>, LyricsError> {
            self.asked.lock().unwrap().push(query.to_string());
            Ok(self.songs.clone())
        }
        fn lyric(&self, id: i64) -> Result<LyricResponse, LyricsError> {
            self.asked.lock().unwrap().push(format!("lyric {id}"));
            Ok(self.lyric.clone())
        }
    }

    #[test]
    fn searches_cleaned_title_and_main_artist() {
        let api = FakeApi {
            songs: vec![song(7, "Bohemian Rhapsody", &["Queen"], 354_000)],
            lyric: response("[00:01.00]Is this the real life?", None),
            asked: Mutex::default(),
        };
        let provider = NetEase::with_api(api);
        let found = provider
            .fetch(&track(
                "Bohemian Rhapsody - Remastered 2011",
                "Queen, Someone",
                354_320,
            ))
            .unwrap()
            .unwrap();
        assert_eq!(found.source, "NetEase");
        assert!(found.lyrics.is_synced());
        assert_eq!(
            *provider.api.asked.lock().unwrap(),
            ["Bohemian Rhapsody Queen", "lyric 7"]
        );
    }
}
