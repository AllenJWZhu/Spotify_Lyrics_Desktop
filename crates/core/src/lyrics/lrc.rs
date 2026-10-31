//! LRC parsing and current-line lookup.
//!
//! Handles the variants seen in the wild:
//! - `[mm:ss]`, `[mm:ss.x]`, `[mm:ss.xx]`, `[mm:ss.xxx]` and `[mm:ss:xx]`
//! - several timestamps on one line (`[00:12.00][01:30.00]repeated line`)
//! - `[offset:±ms]` (positive = show lyrics earlier) and other ID tags
//! - enhanced-LRC word timestamps (`<mm:ss.xx>`), which are stripped for now
//! - blank timestamped lines, kept as empty lines to mark instrumental gaps

/// One timed line of lyrics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LyricLine {
    pub start_ms: u64,
    pub text: String,
}

/// Time-synced lyrics, sorted by start time.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SyncedLyrics {
    lines: Vec<LyricLine>,
}

impl SyncedLyrics {
    /// Parses LRC text. Never fails: lines without a valid timestamp are
    /// skipped, so the result may be empty.
    pub fn parse(lrc: &str) -> Self {
        let lrc = lrc.strip_prefix('\u{feff}').unwrap_or(lrc);
        let mut offset_ms: i64 = 0;
        let mut lines = Vec::new();

        for raw in lrc.lines() {
            let mut rest = raw.trim();
            let mut stamps = Vec::new();

            while let Some(after_open) = rest.strip_prefix('[') {
                let Some(close) = after_open.find(']') else {
                    break;
                };
                let tag = &after_open[..close];
                if let Some(ms) = parse_timestamp(tag) {
                    stamps.push(ms);
                } else if let Some(value) = tag.strip_prefix("offset:") {
                    offset_ms = value.trim().parse().unwrap_or(offset_ms);
                } else {
                    // An ID tag such as [ar:...], or text that merely starts
                    // with a bracket. Either way, no more timestamps follow.
                    break;
                }
                rest = &after_open[close + 1..];
            }

            if stamps.is_empty() {
                continue;
            }
            let text = strip_word_timestamps(rest).trim().to_string();
            for start_ms in stamps {
                lines.push(LyricLine {
                    start_ms,
                    text: text.clone(),
                });
            }
        }

        for line in &mut lines {
            line.start_ms = (line.start_ms as i64 - offset_ms).max(0) as u64;
        }
        // Stable sort keeps the file order for lines sharing a timestamp
        // (e.g. a translation line right after the original).
        lines.sort_by_key(|l| l.start_ms);
        Self { lines }
    }

    pub fn lines(&self) -> &[LyricLine] {
        &self.lines
    }

    /// Writes the lines back as LRC with millisecond timestamps. Parsing the
    /// result gives back equal lyrics (offsets are already applied).
    pub fn to_lrc(&self) -> String {
        let mut out = String::new();
        for line in &self.lines {
            let ms = line.start_ms;
            out.push_str(&format!(
                "[{:02}:{:02}.{:03}]{}\n",
                ms / 60_000,
                ms / 1000 % 60,
                ms % 1000,
                line.text
            ));
        }
        out
    }

    /// True when there is no line with actual text.
    pub fn is_empty(&self) -> bool {
        self.lines.iter().all(|l| l.text.is_empty())
    }

    /// Index of the line being sung at `position_ms`: the last line that has
    /// started. `None` before the first line.
    pub fn line_index_at(&self, position_ms: u64) -> Option<usize> {
        self.lines
            .partition_point(|l| l.start_ms <= position_ms)
            .checked_sub(1)
    }
}

/// Parses `mm:ss`, `mm:ss.f+` or `mm:ss:ff` into milliseconds.
fn parse_timestamp(tag: &str) -> Option<u64> {
    let (minutes, rest) = tag.split_once(':')?;
    let (seconds, fraction) = match rest.find(['.', ':']) {
        Some(i) => (&rest[..i], Some(&rest[i + 1..])),
        None => (rest, None),
    };

    let minutes: u64 = parse_digits(minutes)?;
    let seconds: u64 = parse_digits(seconds)?;
    if seconds >= 60 {
        return None;
    }
    let fraction_ms = match fraction {
        None => 0,
        Some(f) if f.is_empty() || f.len() > 3 => return None,
        // ".5" = 500 ms, ".05" = 50 ms, ".005" = 5 ms
        Some(f) => parse_digits(f)? * 10u64.pow(3 - f.len() as u32),
    };
    Some((minutes * 60 + seconds) * 1000 + fraction_ms)
}

fn parse_digits(s: &str) -> Option<u64> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

/// Removes `<mm:ss.xx>` word timings from enhanced LRC.
fn strip_word_timestamps(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('<') {
        let after = &rest[open + 1..];
        match after.find('>') {
            Some(close) if parse_timestamp(&after[..close]).is_some() => {
                out.push_str(&rest[..open]);
                rest = &after[close + 1..];
            }
            _ => {
                out.push_str(&rest[..=open]);
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn starts_and_texts(l: &SyncedLyrics) -> Vec<(u64, &str)> {
        l.lines()
            .iter()
            .map(|l| (l.start_ms, l.text.as_str()))
            .collect()
    }

    #[test]
    fn timestamp_variants() {
        assert_eq!(parse_timestamp("00:00"), Some(0));
        assert_eq!(parse_timestamp("01:02"), Some(62_000));
        assert_eq!(parse_timestamp("01:02.5"), Some(62_500));
        assert_eq!(parse_timestamp("01:02.50"), Some(62_500));
        assert_eq!(parse_timestamp("01:02.05"), Some(62_050));
        assert_eq!(parse_timestamp("01:02.005"), Some(62_005));
        assert_eq!(parse_timestamp("01:02:34"), Some(62_340));
        assert_eq!(parse_timestamp("123:00.00"), Some(7_380_000));
    }

    #[test]
    fn rejects_non_timestamps() {
        for tag in [
            "ar:Someone",
            "offset:100",
            "",
            "1:2:3:4",
            "aa:bb",
            "00:60.00",
            "00:01.1234",
            "00:01.",
            "-1:00",
        ] {
            assert_eq!(parse_timestamp(tag), None, "{tag}");
        }
    }

    #[test]
    fn parses_basic_file() {
        let lrc = "\u{feff}[ti:Made Up Song]\r\n[ar:Nobody]\r\n\r\n\
                   [00:01.00]first line\r\n[00:04.50] second line \r\n[00:09.00]\r\n";
        let l = SyncedLyrics::parse(lrc);
        assert_eq!(
            starts_and_texts(&l),
            vec![(1_000, "first line"), (4_500, "second line"), (9_000, "")]
        );
        assert!(!l.is_empty());
    }

    #[test]
    fn expands_multi_timestamp_lines_and_sorts() {
        let lrc = "[00:30.00][00:10.00]chorus\n[00:20.00]verse";
        let l = SyncedLyrics::parse(lrc);
        assert_eq!(
            starts_and_texts(&l),
            vec![(10_000, "chorus"), (20_000, "verse"), (30_000, "chorus")]
        );
    }

    #[test]
    fn same_timestamp_keeps_file_order() {
        let lrc = "[00:05.00]original\n[00:05.00]translation";
        let l = SyncedLyrics::parse(lrc);
        assert_eq!(
            starts_and_texts(&l),
            vec![(5_000, "original"), (5_000, "translation")]
        );
    }

    #[test]
    fn applies_offset_tag() {
        // Positive offset: lyrics appear earlier.
        let l = SyncedLyrics::parse("[offset:+500]\n[00:00.20]a\n[00:02.00]b");
        assert_eq!(starts_and_texts(&l), vec![(0, "a"), (1_500, "b")]);
        let l = SyncedLyrics::parse("[00:02.00]b\n[offset:-250]");
        assert_eq!(starts_and_texts(&l), vec![(2_250, "b")]);
    }

    #[test]
    fn strips_word_timings_but_keeps_angle_brackets_in_text() {
        let l = SyncedLyrics::parse("[00:01.00]<00:01.00>one <00:01.40>two <3 me");
        assert_eq!(starts_and_texts(&l), vec![(1_000, "one two <3 me")]);
    }

    #[test]
    fn text_starting_with_bracket_is_kept() {
        let l = SyncedLyrics::parse("[00:01.00][Chorus] la la");
        assert_eq!(starts_and_texts(&l), vec![(1_000, "[Chorus] la la")]);
    }

    #[test]
    fn untimed_lines_are_dropped() {
        let l = SyncedLyrics::parse("just some text\n[Chorus]\nmore text");
        assert!(l.lines().is_empty());
        assert!(l.is_empty());
    }

    #[test]
    fn blank_only_lyrics_count_as_empty() {
        assert!(SyncedLyrics::parse("[00:01.00]\n[00:02.00] ").is_empty());
    }

    #[test]
    fn to_lrc_round_trips() {
        let lrc = "[offset:+100]\n[00:01.23]one\n[00:01.23]one (tr)\n[00:05.00]\n[61:02.005]late";
        let parsed = SyncedLyrics::parse(lrc);
        let written = parsed.to_lrc();
        assert!(written.starts_with("[00:01.130]one\n"));
        assert_eq!(SyncedLyrics::parse(&written), parsed);
    }

    #[test]
    fn line_lookup() {
        let l = SyncedLyrics::parse("[00:01.00]a\n[00:03.00]b\n[00:03.00]b2\n[00:05.00]c");
        assert_eq!(l.line_index_at(0), None);
        assert_eq!(l.line_index_at(999), None);
        assert_eq!(l.line_index_at(1_000), Some(0));
        assert_eq!(l.line_index_at(2_999), Some(0));
        // With equal timestamps the later one (e.g. the translation) wins;
        // callers that want pairs can look one line back.
        assert_eq!(l.line_index_at(3_000), Some(2));
        assert_eq!(l.line_index_at(60_000), Some(3));
        assert_eq!(SyncedLyrics::default().line_index_at(1_000), None);
    }
}
