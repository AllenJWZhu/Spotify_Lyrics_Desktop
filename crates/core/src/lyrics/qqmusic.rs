//! QQ Music (QQ 音乐) as a backup lyrics source.
//!
//! Tencent holds the Chinese rights to most K-pop, so QQ Music has the
//! original recordings, with synced lyrics, for many K-pop songs that other
//! sources lack or only have as covers. These are the endpoints QQ Music's
//! own clients call: unofficial and undocumented, so they may change.
//!
//! - Search: the desktop client's `DoSearchForQQMusicDesktop` through the
//!   `musicu.fcg` gateway, with the older web search as a fallback. Both
//!   sometimes answer with an empty list instead of an error (throttling),
//!   so an empty answer is retried once before the fallback.
//! - Lyrics: `GetPlayLyricInfo` through the gateway (base64 LRC plus a
//!   translation), with `fcg_query_lyric_new` as a fallback.

use std::time::Duration;

use base64::Engine;
use serde_json::{Value, json};

use super::lrclib::HttpTransport;
use super::matching::{Candidate, best_match, is_instrumental_note, strip_credits};
use super::query::{clean_title, primary_artist};
use super::{FoundLyrics, Lyrics, LyricsError, LyricsProvider, Service};
use crate::player::TrackInfo;

const GATEWAY: &str = "https://u.y.qq.com/cgi-bin/musicu.fcg";
const WEB_SEARCH: &str = "https://c.y.qq.com/soso/fcgi-bin/client_search_cp";
const WEB_LYRIC: &str = "https://c.y.qq.com/lyric/fcgi-bin/fcg_query_lyric_new.fcg";
const REFERER: &str = "https://y.qq.com/";
/// The endpoints are made for QQ Music's own web and desktop clients.
const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
     (KHTML, like Gecko) Chrome/130.0 Safari/537.36";
const RESULTS: u32 = 20;
const EMPTY_RETRY_DELAY: Duration = Duration::from_millis(800);

#[derive(Debug, Clone, PartialEq)]
pub struct Song {
    pub mid: String,
    pub name: String,
    pub singers: Vec<String>,
    /// Seconds.
    pub interval: Option<u64>,
}

impl Candidate for Song {
    fn title(&self) -> &str {
        &self.name
    }
    fn artist_names(&self) -> Vec<&str> {
        self.singers.iter().map(String::as_str).collect()
    }
    fn duration_ms(&self) -> Option<u64> {
        self.interval.map(|s| s * 1000)
    }
}

/// LRC text and its translation, either possibly empty.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LyricText {
    pub lyric: String,
    pub trans: String,
}

/// The calls we make. Abstracted for tests.
pub trait Api: Send + Sync {
    fn search(&self, query: &str) -> Result<Vec<Song>, LyricsError>;
    fn lyric(&self, mid: &str) -> Result<LyricText, LyricsError>;
}

fn service_error(error: ureq::Error) -> LyricsError {
    LyricsError::Service {
        service: Service::QqMusic,
        error,
    }
}

/// Songs from a search result list (gateway and web field names differ).
fn songs(list: &Value, mid: &str, name: &str) -> Vec<Song> {
    list.as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| {
            Some(Song {
                mid: s[mid].as_str()?.to_string(),
                name: s[name].as_str()?.to_string(),
                singers: s["singer"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|x| x["name"].as_str().map(str::to_string))
                    .collect(),
                interval: s["interval"].as_u64(),
            })
        })
        .collect()
}

impl HttpTransport {
    fn qq_gateway_search(&self, query: &str) -> Result<Vec<Song>, LyricsError> {
        let body = json!({
            "comm": { "ct": 19, "cv": 1859, "uin": "0" },
            "req": {
                "module": "music.search.SearchCgiService",
                "method": "DoSearchForQQMusicDesktop",
                "param": { "query": query, "num_per_page": RESULTS, "page_num": 1, "search_type": 0 }
            }
        });
        let v: Value = self
            .send(|agent| {
                agent
                    .post(GATEWAY)
                    .header("Referer", REFERER)
                    .header("User-Agent", USER_AGENT)
                    .send_json(&body)?
                    .body_mut()
                    .read_json()
            })
            .map_err(service_error)?;
        Ok(songs(
            &v["req"]["data"]["body"]["song"]["list"],
            "mid",
            "name",
        ))
    }

    fn qq_web_search(&self, query: &str) -> Result<Vec<Song>, LyricsError> {
        let v: Value = self
            .send(|agent| {
                agent
                    .get(WEB_SEARCH)
                    .header("Referer", REFERER)
                    .header("User-Agent", USER_AGENT)
                    .query("w", query)
                    .query("format", "json")
                    .query("p", "1")
                    .query("n", RESULTS.to_string())
                    .query("cr", "1")
                    .call()?
                    .body_mut()
                    .read_json()
            })
            .map_err(service_error)?;
        Ok(songs(&v["data"]["song"]["list"], "songmid", "songname"))
    }

    fn qq_gateway_lyric(&self, mid: &str) -> Result<LyricText, LyricsError> {
        let body = json!({
            "comm": { "ct": 19, "cv": 1859, "uin": "0" },
            "req": {
                "module": "music.musichallSong.PlayLyricInfo",
                "method": "GetPlayLyricInfo",
                "param": { "songMID": mid, "trans": 1, "roma": 0, "crypt": 0, "type": 0 }
            }
        });
        let v: Value = self
            .send(|agent| {
                agent
                    .post(GATEWAY)
                    .header("Referer", REFERER)
                    .header("User-Agent", USER_AGENT)
                    .send_json(&body)?
                    .body_mut()
                    .read_json()
            })
            .map_err(service_error)?;
        let data = &v["req"]["data"];
        Ok(LyricText {
            lyric: decode_field(&data["lyric"]),
            trans: decode_field(&data["trans"]),
        })
    }

    fn qq_web_lyric(&self, mid: &str) -> Result<LyricText, LyricsError> {
        let text = self
            .send(|agent| {
                agent
                    .get(WEB_LYRIC)
                    .header("Referer", REFERER)
                    .header("User-Agent", USER_AGENT)
                    .query("songmid", mid)
                    .query("format", "json")
                    .query("nobase64", "1")
                    .query("g_tk", "5381")
                    .call()?
                    .body_mut()
                    .read_to_string()
            })
            .map_err(service_error)?;
        // Sometimes wrapped as JSONP: MusicJsonCallback({...}).
        let json = text
            .find('{')
            .zip(text.rfind('}'))
            .map_or("", |(a, b)| &text[a..=b]);
        let v: Value = serde_json::from_str(json)
            .map_err(|e| LyricsError::Other(format!("QQ Music sent an unreadable lyric: {e}")))?;
        Ok(LyricText {
            lyric: unescape(v["lyric"].as_str().unwrap_or_default()),
            trans: unescape(v["trans"].as_str().unwrap_or_default()),
        })
    }
}

/// The gateway sends LRC base64-encoded; plain text is passed through.
fn decode_field(v: &Value) -> String {
    let s = v.as_str().unwrap_or_default().trim();
    if s.is_empty() {
        return String::new();
    }
    match base64::engine::general_purpose::STANDARD.decode(s) {
        Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
        Err(_) => unescape(s),
    }
}

/// The web endpoint escapes some characters as HTML entities.
fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(end) = rest.find(';').filter(|e| *e <= 10) else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let entity = &rest[1..end];
        let ch = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            _ => entity
                .strip_prefix('#')
                .and_then(|n| match n.strip_prefix(['x', 'X']) {
                    Some(hex) => u32::from_str_radix(hex, 16).ok(),
                    None => n.parse().ok(),
                })
                .and_then(char::from_u32),
        };
        match ch {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

impl Api for HttpTransport {
    fn search(&self, query: &str) -> Result<Vec<Song>, LyricsError> {
        match self.qq_gateway_search(query) {
            Ok(songs) if !songs.is_empty() => return Ok(songs),
            Ok(_) => {
                std::thread::sleep(EMPTY_RETRY_DELAY);
                if let Ok(songs) = self.qq_gateway_search(query) {
                    if !songs.is_empty() {
                        return Ok(songs);
                    }
                }
            }
            Err(e) => log::warn!("QQ Music search failed ({e}); trying the web search"),
        }
        self.qq_web_search(query)
    }

    fn lyric(&self, mid: &str) -> Result<LyricText, LyricsError> {
        match self.qq_gateway_lyric(mid) {
            Ok(text) if !text.lyric.trim().is_empty() => Ok(text),
            _ => self.qq_web_lyric(mid),
        }
    }
}

pub struct QqMusic<A: Api = HttpTransport> {
    api: A,
}

impl QqMusic<HttpTransport> {
    pub fn new() -> Self {
        let proxy = ureq::Proxy::try_from_env();
        Self::with_api(HttpTransport::with_proxy(USER_AGENT, "", proxy))
    }
}

impl Default for QqMusic<HttpTransport> {
    fn default() -> Self {
        Self::new()
    }
}

impl<A: Api> QqMusic<A> {
    pub fn with_api(api: A) -> Self {
        Self { api }
    }
}

impl<A: Api> LyricsProvider for QqMusic<A> {
    fn fetch(&self, track: &TrackInfo) -> Result<Option<FoundLyrics>, LyricsError> {
        let title = clean_title(track.title.trim());
        let artist = primary_artist(track.artist.trim());
        let mut songs = self.api.search(&format!("{title} {artist}"))?;
        if best_match(&songs, track).is_none() {
            songs = self.api.search(&title)?;
        }
        let Some(song) = best_match(&songs, track) else {
            return Ok(None);
        };
        let text = self.api.lyric(&song.mid)?;
        let Some(lyrics) = lyrics_from(&text) else {
            return Ok(None);
        };
        Ok(Some(FoundLyrics {
            lyrics,
            source: Service::QqMusic.name().into(),
            matched_title: song.name.clone(),
            matched_artist: song.singers.join(", "),
            matched_duration_ms: song.duration_ms(),
        }))
    }
}

/// Lyrics from QQ's LRC; the translation, if any, goes under each line.
pub fn lyrics_from(text: &LyricText) -> Option<Lyrics> {
    let original = strip_credits(&text.lyric);
    if original.trim().is_empty() {
        return None;
    }
    if is_instrumental_note(&original) {
        return Some(Lyrics::Instrumental);
    }
    // QQ fills untranslated lines with "//"; drop those.
    let translation: String = strip_credits(&text.trans)
        .lines()
        .filter(|l| {
            let t = l.rsplit(']').next().unwrap_or(l).trim();
            !t.is_empty() && t != "//"
        })
        .collect::<Vec<_>>()
        .join("\n");
    let combined = if translation.is_empty() {
        original.clone()
    } else {
        format!("{original}\n{translation}")
    };
    let plain: String = original
        .lines()
        .map(|l| l.rsplit(']').next().unwrap_or(l).trim())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    Lyrics::from_parts(Some(&combined), Some(&plain), false)
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

    fn song(mid: &str, name: &str, singers: &[&str], secs: u64) -> Song {
        Song {
            mid: mid.into(),
            name: name.into(),
            singers: singers.iter().map(|s| s.to_string()).collect(),
            interval: Some(secs),
        }
    }

    #[test]
    fn reads_both_search_result_shapes() {
        let gateway = json!([{ "mid": "a1", "name": "ETA", "singer": [{"name": "NewJeans"}], "interval": 151 }]);
        let web = json!([{ "songmid": "b2", "songname": "ETA", "singer": [{"name": "NewJeans"}], "interval": 151 }]);
        assert_eq!(
            songs(&gateway, "mid", "name"),
            [song("a1", "ETA", &["NewJeans"], 151)]
        );
        assert_eq!(
            songs(&web, "songmid", "songname"),
            [song("b2", "ETA", &["NewJeans"], 151)]
        );
    }

    #[test]
    fn decodes_base64_and_entities() {
        let encoded = base64::engine::general_purpose::STANDARD.encode("[00:01.00]Hi");
        assert_eq!(decode_field(&json!(encoded)), "[00:01.00]Hi");
        assert_eq!(
            unescape("It&apos;s &#58; &#x41; &amp; 5 & 6"),
            "It's : A & 5 & 6"
        );
    }

    #[test]
    fn merges_translation_and_skips_placeholders() {
        let text = LyricText {
            lyric: "[ti:x]\n[00:00.00]词：A\n[00:10.00]Hello\n[00:12.00]Bye".into(),
            trans: "[00:10.00]你好\n[00:12.00]//".into(),
        };
        let Some(Lyrics::Synced(s)) = lyrics_from(&text) else {
            panic!("expected synced lyrics");
        };
        let got: Vec<_> = s
            .lines()
            .iter()
            .map(|l| (l.start_ms, l.text.as_str()))
            .collect();
        assert_eq!(got, [(10_000, "Hello"), (10_000, "你好"), (12_000, "Bye")]);
        assert_eq!(
            lyrics_from(&LyricText {
                lyric: "[00:00.00]此歌曲为没有填词的纯音乐，请您欣赏".into(),
                trans: String::new(),
            }),
            Some(Lyrics::Instrumental)
        );
        assert_eq!(lyrics_from(&LyricText::default()), None);
    }

    struct FakeApi {
        results: Vec<Vec<Song>>,
        asked: Mutex<Vec<String>>,
    }

    impl Api for FakeApi {
        fn search(&self, query: &str) -> Result<Vec<Song>, LyricsError> {
            let mut asked = self.asked.lock().unwrap();
            let i = asked.iter().filter(|q| !q.starts_with("lyric")).count();
            asked.push(query.to_string());
            Ok(self.results.get(i).cloned().unwrap_or_default())
        }
        fn lyric(&self, mid: &str) -> Result<LyricText, LyricsError> {
            self.asked.lock().unwrap().push(format!("lyric {mid}"));
            Ok(LyricText {
                lyric: "[00:01.00]line".into(),
                trans: String::new(),
            })
        }
    }

    #[test]
    fn finds_the_original_and_falls_back_to_a_title_search() {
        let t = track("Brought The Heat Back", "ENHYPEN", 176_424);
        let provider = QqMusic::with_api(FakeApi {
            results: vec![
                vec![song("cover", "Brought The Heat Back", &["Someone"], 176)],
                vec![song("orig", "Brought The Heat Back", &["ENHYPEN"], 176)],
            ],
            asked: Mutex::default(),
        });
        let found = provider.fetch(&t).unwrap().unwrap();
        assert_eq!(found.source, "QQ Music");
        assert_eq!(
            *provider.api.asked.lock().unwrap(),
            [
                "Brought The Heat Back ENHYPEN",
                "Brought The Heat Back",
                "lyric orig"
            ]
        );
    }
}
