//! Cleans up Spotify metadata into forms more likely to match a lyrics
//! database entry.
//!
//! Spotify titles often carry release details that lyrics databases don't
//! ("Song - Remastered 2011", "Song (feat. X)"), and multi-artist tracks list
//! every artist while databases usually store only the main one. These are
//! used as a fallback after the raw metadata fails to match.

/// Words that mark a bracketed group or " - " suffix as release detail
/// rather than part of the song name. Matched case-insensitively as substrings.
const DETAIL_MARKERS: &[&str] = &[
    "feat.",
    "feat ",
    "ft.",
    "featuring",
    "with ",
    "remaster",
    "live",
    "version",
    "edit",
    "mix",
    "mono",
    "stereo",
    "deluxe",
    "anniversary",
    "bonus",
    "demo",
    "acoustic",
    "from ",
    "sped up",
    "slowed",
];

fn is_release_detail(s: &str) -> bool {
    let s = s.to_lowercase();
    DETAIL_MARKERS.iter().any(|m| s.contains(m))
}

/// Strips release details from a title. Returns the input unchanged (but
/// trimmed) when nothing applies, and never returns an empty string for a
/// non-empty input.
pub fn clean_title(title: &str) -> String {
    let mut out = strip_detail_groups(title);

    // "Song - Remastered 2011", "Song - Live at X"
    if let Some(i) = out.find(" - ") {
        if is_release_detail(&out[i + 3..]) {
            out.truncate(i);
        }
    }

    let out = collapse_spaces(&out);
    if out.is_empty() {
        title.trim().to_string()
    } else {
        out
    }
}

/// Removes `( … )` and `[ … ]` groups whose content is release detail.
fn strip_detail_groups(title: &str) -> String {
    let mut out = String::with_capacity(title.len());
    let mut rest = title;
    while let Some(open) = rest.find(['(', '[']) {
        let close_char = if rest[open..].starts_with('(') {
            ')'
        } else {
            ']'
        };
        let inner_start = open + 1;
        match rest[inner_start..].find(close_char) {
            Some(len) if is_release_detail(&rest[inner_start..inner_start + len]) => {
                out.push_str(&rest[..open]);
                rest = &rest[inner_start + len + 1..];
            }
            _ => {
                out.push_str(&rest[..inner_start]);
                rest = &rest[inner_start..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn collapse_spaces(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The first artist from a multi-artist credit.
pub fn primary_artist(artist: &str) -> String {
    const SEPARATORS: &[&str] = &[", ", " & ", " feat. ", " ft. ", " featuring ", "; ", "、"];
    let lower = artist.to_lowercase();
    let cut = SEPARATORS
        .iter()
        .filter_map(|sep| lower.find(sep))
        .min()
        .unwrap_or(artist.len());
    // `lower` may differ in byte length from `artist` for some scripts; only
    // trust the cut if it lands on a char boundary of the original.
    let head = if artist.is_char_boundary(cut) {
        &artist[..cut]
    } else {
        artist
    };
    let head = head.trim();
    if head.is_empty() {
        artist.trim().to_string()
    } else {
        head.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_dash_suffix_release_details() {
        assert_eq!(clean_title("Song - Remastered 2011"), "Song");
        assert_eq!(clean_title("Song - 2015 Remaster"), "Song");
        assert_eq!(clean_title("Song - Live at Wembley"), "Song");
        assert_eq!(clean_title("Song - Radio Edit"), "Song");
        assert_eq!(clean_title("Song - From \"Some Movie\""), "Song");
    }

    #[test]
    fn keeps_dash_that_is_part_of_title() {
        assert_eq!(
            clean_title("Part 1 - The Beginning"),
            "Part 1 - The Beginning"
        );
    }

    #[test]
    fn strips_bracketed_details() {
        assert_eq!(clean_title("Song (feat. Someone)"), "Song");
        assert_eq!(clean_title("Song [feat. Someone]"), "Song");
        assert_eq!(clean_title("Song (with Someone)"), "Song");
        assert_eq!(clean_title("Song (Remastered) - Mono"), "Song");
        assert_eq!(clean_title("Song (Sped Up Version)"), "Song");
    }

    #[test]
    fn keeps_meaningful_brackets() {
        assert_eq!(clean_title("Song (Part 2)"), "Song (Part 2)");
        assert_eq!(clean_title("(Untitled)"), "(Untitled)");
        assert_eq!(clean_title("Unclosed (paren"), "Unclosed (paren");
    }

    #[test]
    fn never_returns_empty() {
        assert_eq!(clean_title("(feat. Someone)"), "(feat. Someone)");
        assert_eq!(clean_title("  Song  "), "Song");
    }

    #[test]
    fn works_with_cjk_titles() {
        assert_eq!(clean_title("歌名 (Live)"), "歌名");
        assert_eq!(clean_title("歌名（伴奏）"), "歌名（伴奏）");
    }

    #[test]
    fn primary_artist_splits_credits() {
        assert_eq!(primary_artist("Solo Artist"), "Solo Artist");
        assert_eq!(primary_artist("A, B"), "A");
        assert_eq!(primary_artist("A & B, C"), "A");
        assert_eq!(primary_artist("A feat. B"), "A");
        assert_eq!(primary_artist("A FEAT. B"), "A");
        assert_eq!(primary_artist("歌手甲、歌手乙"), "歌手甲");
        assert_eq!(primary_artist(", B"), ", B");
    }
}
