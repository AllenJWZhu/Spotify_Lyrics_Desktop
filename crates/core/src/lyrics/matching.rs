//! Shared by the search-based sources (NetEase, QQ Music): deciding which
//! search result is the playing track, and cleaning up their LRC text.

use super::query::{clean_title, primary_artist};
use crate::player::TrackInfo;

/// A search result as matching needs it.
pub trait Candidate {
    fn title(&self) -> &str;
    fn artist_names(&self) -> Vec<&str>;
    /// Length in milliseconds, if known.
    fn duration_ms(&self) -> Option<u64>;
}

/// A search result must be this close to the track's length.
const DURATION_TOLERANCE_MS: u64 = 3_000;

/// Letters and digits only, lowercased, so "Love Attack!" == "love attack".
fn normalize(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// `a` and `b` name the same artist: equal, or one contains the other and
/// is a substantial part of it (NetEase writes "TWS (투어스)" for "TWS").
fn same_artist(a: &str, b: &str) -> bool {
    let (a, b) = (normalize(a), normalize(b));
    if a.is_empty() || b.is_empty() {
        return false;
    }
    a == b || ((a.contains(&b) || b.contains(&a)) && a.chars().count().min(b.chars().count()) >= 2)
}

/// Length difference allowed for the same title, and for the same title
/// once release details are cleaned off ("Song (Remastered 2011)"), which
/// may be another recording, so it has to be closer.
const EXACT_TITLE_TOLERANCE_MS: u64 = DURATION_TOLERANCE_MS;
const CLEANED_TITLE_TOLERANCE_MS: u64 = 1_500;

/// The search result that is this track, if any: one of the artists must
/// match (covers often have the same length), the title must be the same,
/// and the length close. Exact titles beat ones that only match after
/// cleanup, then the closer length wins.
pub fn best_match<'a, C: Candidate>(songs: &'a [C], track: &TrackInfo) -> Option<&'a C> {
    let title = normalize(&track.title);
    let clean = normalize(&clean_title(&track.title));
    let artist = primary_artist(&track.artist);
    songs
        .iter()
        .filter_map(|song| {
            if !song.artist_names().iter().any(|a| same_artist(a, &artist)) {
                return None;
            }
            let exact = normalize(song.title()) == title;
            if !exact && normalize(&clean_title(song.title())) != clean {
                return None;
            }
            let diff = match song.duration_ms() {
                Some(dt) if track.duration_ms > 0 => dt.abs_diff(track.duration_ms),
                _ => return None,
            };
            let tolerance = if exact {
                EXACT_TITLE_TOLERANCE_MS
            } else {
                CLEANED_TITLE_TOLERANCE_MS
            };
            (diff <= tolerance).then_some((song, (!exact, diff)))
        })
        .min_by_key(|(_, rank)| *rank)
        .map(|(song, _)| song)
}

/// Credit lines these services put at the top of their LRC ("作词 : …").
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
pub fn strip_credits(lrc: &str) -> String {
    lrc.lines()
        .filter(|line| {
            let text = line.rsplit(']').next().unwrap_or(line);
            !line.trim_start().starts_with('{') && !is_credit(text)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// An LRC whose only text is a note like "纯音乐，请欣赏" (instrumental).
pub fn is_instrumental_note(lrc: &str) -> bool {
    let texts: Vec<&str> = lrc
        .lines()
        .map(|l| l.rsplit(']').next().unwrap_or(l).trim())
        .filter(|t| !t.is_empty())
        .collect();
    texts.len() <= 2 && texts.iter().any(|t| t.contains("纯音乐"))
}
