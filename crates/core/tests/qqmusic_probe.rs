//! Coverage probe for QQ Music (QQ 音乐), before deciding whether to add it
//! as a lyrics source. Not used by the app. Ignored by default; run with
//!
//! ```sh
//! cargo test -p lyrics-core --test qqmusic_probe -- --ignored --nocapture
//! ```
//!
//! Prints match metadata and line counts only, never lyrics text.

use std::time::Duration;

use serde_json::{Value, json};

const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0 Safari/537.36";

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .user_agent(UA)
        .timeout_global(Some(Duration::from_secs(10)))
        .http_status_as_error(false)
        .build()
        .into()
}

struct Hit {
    mid: String,
    name: String,
    singers: String,
    seconds: u64,
}

fn hits(list: &Value, mid: &str, name: &str, secs: &str) -> Vec<Hit> {
    list.as_array()
        .map(|a| {
            a.iter()
                .map(|s| Hit {
                    mid: s[mid].as_str().unwrap_or_default().to_string(),
                    name: s[name].as_str().unwrap_or_default().to_string(),
                    singers: s["singer"]
                        .as_array()
                        .map(|v| {
                            v.iter()
                                .filter_map(|x| x["name"].as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        })
                        .unwrap_or_default(),
                    seconds: s[secs].as_u64().unwrap_or(0),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The desktop client's search, through the musicu gateway.
fn search_musicu(agent: &ureq::Agent, query: &str) -> Result<Vec<Hit>, String> {
    let body = json!({
        "comm": { "ct": 19, "cv": 1859, "uin": "0" },
        "req": {
            "module": "music.search.SearchCgiService",
            "method": "DoSearchForQQMusicDesktop",
            "param": { "query": query, "num_per_page": 10, "page_num": 1, "search_type": 0 }
        }
    });
    let mut res = agent
        .post("https://u.y.qq.com/cgi-bin/musicu.fcg")
        .header("Referer", "https://y.qq.com/")
        .send_json(&body)
        .map_err(|e| e.to_string())?;
    let status = res.status();
    let v: Value = res
        .body_mut()
        .read_json()
        .map_err(|e| format!("{status}: {e}"))?;
    let list = &v["req"]["data"]["body"]["song"]["list"];
    if list.is_null() {
        return Err(format!("{status}, no list; code {}", v["req"]["code"]));
    }
    Ok(hits(list, "mid", "name", "interval"))
}

/// The older web search endpoint.
fn search_soso(agent: &ureq::Agent, query: &str) -> Result<Vec<Hit>, String> {
    let mut res = agent
        .get("https://c.y.qq.com/soso/fcgi-bin/client_search_cp")
        .header("Referer", "https://y.qq.com/")
        .query("w", query)
        .query("format", "json")
        .query("p", "1")
        .query("n", "10")
        .query("cr", "1")
        .call()
        .map_err(|e| e.to_string())?;
    let status = res.status();
    let v: Value = res
        .body_mut()
        .read_json()
        .map_err(|e| format!("{status}: {e}"))?;
    let list = &v["data"]["song"]["list"];
    if list.is_null() {
        return Err(format!("{status}, no list; code {}", v["code"]));
    }
    Ok(hits(list, "songmid", "songname", "interval"))
}

/// (synced lines, translation lines) or why not.
fn lyric(agent: &ureq::Agent, mid: &str) -> Result<(usize, usize), String> {
    let mut res = agent
        .get("https://c.y.qq.com/lyric/fcgi-bin/fcg_query_lyric_new.fcg")
        .header("Referer", "https://y.qq.com/")
        .query("songmid", mid)
        .query("format", "json")
        .query("nobase64", "1")
        .query("g_tk", "5381")
        .call()
        .map_err(|e| e.to_string())?;
    let status = res.status();
    let text = res.body_mut().read_to_string().map_err(|e| e.to_string())?;
    let v: Value = serde_json::from_str(&text).map_err(|e| format!("{status}: {e}"))?;
    let count = |s: &str| {
        s.lines()
            .filter(|l| {
                let b = l.as_bytes();
                b.len() > 10
                    && b[0] == b'['
                    && b[1].is_ascii_digit()
                    && !l.rsplit(']').next().unwrap_or("").trim().is_empty()
            })
            .count()
    };
    let lrc = v["lyric"].as_str().unwrap_or_default();
    let trans = v["trans"].as_str().unwrap_or_default();
    if lrc.is_empty() {
        return Err(format!("retcode {} code {}", v["retcode"], v["code"]));
    }
    Ok((count(lrc), count(trans)))
}

#[test]
#[ignore = "network"]
fn report_qq_music_coverage() {
    let agent = agent();
    // (title, artist, Spotify length in seconds where known, 0 otherwise)
    let tracks = [
        ("TIMEBOMB", "izna", 171),
        ("ETA", "NewJeans", 151),
        ("plot twist", "TWS", 152),
        ("LOVE ATTACK", "RESCENE", 182),
        ("Brought The Heat Back", "ENHYPEN", 176),
        ("Supernatural", "NewJeans", 0),
        ("How Sweet", "NewJeans", 0),
        ("Whiplash", "aespa", 0),
        ("Magnetic", "ILLIT", 0),
        ("HOT", "LE SSERAFIM", 0),
        ("REBEL HEART", "IVE", 0),
        ("DRIP", "BABYMONSTER", 0),
        ("APT.", "ROSÉ", 0),
        ("Love wins all", "IU", 0),
        ("晴天", "周杰伦", 269),
    ];
    let mut synced = 0;
    for (title, artist, secs) in tracks {
        let query = format!("{title} {artist}");
        let found = search_musicu(&agent, &query)
            .map(|h| ("musicu", h))
            .or_else(|e1| {
                search_soso(&agent, &query)
                    .map(|h| ("soso", h))
                    .map_err(|e2| format!("musicu: {e1}; soso: {e2}"))
            });
        let line = match found {
            Err(e) => format!("search failed ({e})"),
            Ok((via, hits)) => {
                let want = |h: &&Hit| {
                    let n = |s: &str| s.to_lowercase().replace(' ', "");
                    n(&h.name).contains(&n(title))
                        && n(&h.singers).contains(&n(artist))
                        && (secs == 0 || h.seconds.abs_diff(secs) <= 3)
                };
                match hits.iter().find(want) {
                    None => format!(
                        "no match via {via}; top: {}",
                        hits.iter()
                            .take(4)
                            .map(|h| format!("{:?} by {:?} {}s", h.name, h.singers, h.seconds))
                            .collect::<Vec<_>>()
                            .join(" | ")
                    ),
                    Some(h) => match lyric(&agent, &h.mid) {
                        Ok((n, t)) => {
                            if n > 5 {
                                synced += 1;
                            }
                            format!(
                                "via {via}: {:?} by {:?} {}s -> {n} synced lines, {t} translated",
                                h.name, h.singers, h.seconds
                            )
                        }
                        Err(e) => format!("matched {:?} but no lyric ({e})", h.name),
                    },
                }
            }
        };
        report(&format!("{title} — {artist}: {line}"));
    }
    report(&format!("synced: {synced} of {}", tracks.len()));
}

fn report(line: &str) {
    println!("{line}");
    if std::env::var_os("GITHUB_ACTIONS").is_some() {
        println!("::notice title=QQ Music probe::{line}");
    }
}
