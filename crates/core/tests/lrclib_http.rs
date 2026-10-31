//! Exercises the real HTTP transport (URL encoding, headers, 404 handling,
//! JSON decoding) against a tiny local server, so no network is needed.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use lyrics_core::lyrics::lrclib::{GetQuery, HttpTransport, Lrclib, Transport};
use lyrics_core::lyrics::{Lyrics, LyricsProvider};
use lyrics_core::player::TrackInfo;

/// A request as the server saw it.
#[derive(Debug, Clone)]
struct Seen {
    path: String,
    query: Vec<(String, String)>,
    user_agent: Option<String>,
}

impl Seen {
    fn param(&self, key: &str) -> Option<&str> {
        self.query
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }
}

type Router = dyn Fn(&Seen) -> (u16, String) + Send + Sync;

/// Serves HTTP/1.1 on 127.0.0.1 with one response per connection.
fn serve(router: Box<Router>) -> (String, Arc<Mutex<Vec<Seen>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = Arc::clone(&seen);

    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut request_line = String::new();
            if reader.read_line(&mut request_line).is_err() {
                continue;
            }
            let mut user_agent = None;
            loop {
                let mut header = String::new();
                if reader.read_line(&mut header).unwrap_or(0) == 0 || header == "\r\n" {
                    break;
                }
                if let Some((k, v)) = header.split_once(':') {
                    if k.eq_ignore_ascii_case("user-agent") {
                        user_agent = Some(v.trim().to_string());
                    }
                }
            }

            let target = request_line.split_whitespace().nth(1).unwrap_or("/");
            let (path, raw_query) = target.split_once('?').unwrap_or((target, ""));
            let request = Seen {
                path: path.to_string(),
                query: raw_query
                    .split('&')
                    .filter(|p| !p.is_empty())
                    .map(|p| {
                        let (k, v) = p.split_once('=').unwrap_or((p, ""));
                        (percent_decode(k), percent_decode(v))
                    })
                    .collect(),
                user_agent,
            };
            log.lock().unwrap().push(request.clone());

            let (status, body) = router(&request);
            let reason = if status == 200 { "OK" } else { "Error" };
            let _ = write!(
                stream,
                "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\n\
                 Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    (base, seen)
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap();
                out.push(u8::from_str_radix(hex, 16).unwrap());
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8(out).unwrap()
}

const RECORD: &str = r#"{"id": 7, "trackName": "歌名 & More", "artistName": "Made Up",
    "albumName": null, "duration": 200.0, "instrumental": false, "hasWordSync": false,
    "plainLyrics": "one\ntwo", "syncedLyrics": "[00:01.00]one\n[00:02.00]two", "lyricsfile": null}"#;

const NOT_FOUND: &str =
    r#"{"statusCode": 404, "name": "TrackNotFound", "message": "Failed to find specified track"}"#;

#[test]
fn get_encodes_query_and_decodes_record() {
    let (base, seen) = serve(Box::new(|_| (200, RECORD.to_string())));
    let transport = HttpTransport::with_base_url("test-agent/1.0", &base);

    let record = transport
        .get(&GetQuery {
            track_name: "歌名 & More".into(),
            artist_name: "A=B".into(),
            album_name: Some("Album?".into()),
            duration_s: Some(171.232),
        })
        .unwrap()
        .expect("record");
    assert_eq!(record.id, 7);
    assert_eq!(record.track_name.as_deref(), Some("歌名 & More"));

    let seen = seen.lock().unwrap();
    let req = &seen[0];
    assert_eq!(req.path, "/api/get");
    assert_eq!(req.param("track_name"), Some("歌名 & More"));
    assert_eq!(req.param("artist_name"), Some("A=B"));
    assert_eq!(req.param("album_name"), Some("Album?"));
    assert_eq!(req.param("duration"), Some("171.232"));
    assert_eq!(req.user_agent.as_deref(), Some("test-agent/1.0"));
}

#[test]
fn get_maps_404_to_none_and_omits_unset_params() {
    let (base, seen) = serve(Box::new(|_| (404, NOT_FOUND.to_string())));
    let transport = HttpTransport::with_base_url("t", &base);
    let result = transport
        .get(&GetQuery {
            track_name: "x".into(),
            artist_name: "y".into(),
            album_name: None,
            duration_s: None,
        })
        .unwrap();
    assert!(result.is_none());
    let req = &seen.lock().unwrap()[0];
    assert_eq!(req.param("album_name"), None);
    assert_eq!(req.param("duration"), None);
}

#[test]
fn persistent_overload_fails_after_retries() {
    let (base, seen) = serve(Box::new(|_| (503, "{}".to_string())));
    let transport = HttpTransport::with_base_url("t", &base).with_retry_delays(no_wait());
    assert!(transport.search("x", "y").is_err());
    // One attempt plus two retries.
    assert_eq!(seen.lock().unwrap().len(), 3);
}

#[test]
fn transient_overload_is_retried() {
    let calls = AtomicUsize::new(0);
    let (base, seen) = serve(Box::new(move |_| {
        if calls.fetch_add(1, Ordering::SeqCst) < 2 {
            (503, "{}".to_string())
        } else {
            (200, RECORD.to_string())
        }
    }));
    let transport = HttpTransport::with_base_url("t", &base).with_retry_delays(no_wait());
    let query = GetQuery {
        track_name: "x".into(),
        artist_name: "y".into(),
        album_name: None,
        duration_s: None,
    };
    assert!(transport.get(&query).unwrap().is_some());
    assert_eq!(seen.lock().unwrap().len(), 3);
}

#[test]
fn not_found_is_not_retried() {
    let (base, seen) = serve(Box::new(|_| (404, NOT_FOUND.to_string())));
    let transport = HttpTransport::with_base_url("t", &base).with_retry_delays(no_wait());
    let query = GetQuery {
        track_name: "x".into(),
        artist_name: "y".into(),
        album_name: None,
        duration_s: None,
    };
    assert!(transport.get(&query).unwrap().is_none());
    assert_eq!(seen.lock().unwrap().len(), 1);
}

fn no_wait() -> Vec<Duration> {
    vec![Duration::ZERO; 2]
}

#[test]
fn full_lookup_falls_through_to_search() {
    let (base, seen) = serve(Box::new(|req| match req.path.as_str() {
        "/api/get" => (404, NOT_FOUND.to_string()),
        "/api/search" => (200, format!("[{RECORD}]")),
        _ => (500, "{}".to_string()),
    }));
    let provider = Lrclib::with_transport(HttpTransport::with_base_url("t", &base));
    let track = TrackInfo {
        title: "Song (feat. Guest) - Remastered".into(),
        artist: "Made Up, Guest".into(),
        album: "Album".into(),
        duration_ms: 200_500,
    };
    let found = provider.fetch(&track).unwrap().expect("lyrics");
    assert!(matches!(found.lyrics, Lyrics::Synced(_)));

    let seen = seen.lock().unwrap();
    let paths: Vec<&str> = seen.iter().map(|r| r.path.as_str()).collect();
    assert_eq!(paths, ["/api/get", "/api/get", "/api/get", "/api/search"]);
    let search = seen.last().unwrap();
    assert_eq!(search.param("track_name"), Some("Song"));
    assert_eq!(search.param("artist_name"), Some("Made Up"));
}

/// A "proxy" that accepts connections and hangs up, counting them.
fn broken_proxy() -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let hits = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&hits);
    thread::spawn(move || {
        for stream in listener.incoming() {
            if stream.is_ok() {
                counter.fetch_add(1, Ordering::SeqCst);
            }
        }
    });
    (url, hits)
}

#[test]
fn falls_back_to_direct_when_the_proxy_fails() {
    let (proxy_url, proxy_hits) = broken_proxy();
    let (base, seen) = serve(Box::new(|_| (200, RECORD.to_string())));
    let proxy = ureq::Proxy::new(&proxy_url).unwrap();
    let transport = HttpTransport::with_proxy("t", &base, Some(proxy)).with_retry_delays(no_wait());
    let query = GetQuery {
        track_name: "x".into(),
        artist_name: "y".into(),
        album_name: None,
        duration_s: None,
    };

    assert!(transport.get(&query).unwrap().is_some());
    assert_eq!(proxy_hits.load(Ordering::SeqCst), 1);
    // Once direct worked, later requests skip the broken proxy.
    assert!(transport.get(&query).unwrap().is_some());
    assert_eq!(proxy_hits.load(Ordering::SeqCst), 1);
    assert_eq!(seen.lock().unwrap().len(), 2);
}
