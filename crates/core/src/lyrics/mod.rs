//! Lyrics model, parsing and providers.

mod lrc;
pub mod lrclib;
pub mod query;

pub use lrc::{LyricLine, SyncedLyrics};

use crate::player::TrackInfo;

/// Lyrics for one track, best form first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Lyrics {
    Synced(SyncedLyrics),
    /// Text without timing; can be shown but not followed.
    Plain(String),
    /// The track is known to have no vocals.
    Instrumental,
}

impl Lyrics {
    /// Builds the best available form from raw provider fields. `None` when
    /// none of them carries anything usable.
    pub fn from_parts(
        synced: Option<&str>,
        plain: Option<&str>,
        instrumental: bool,
    ) -> Option<Self> {
        if let Some(synced) = synced.map(SyncedLyrics::parse).filter(|s| !s.is_empty()) {
            return Some(Lyrics::Synced(synced));
        }
        if let Some(plain) = plain.map(str::trim).filter(|p| !p.is_empty()) {
            return Some(Lyrics::Plain(plain.to_string()));
        }
        instrumental.then_some(Lyrics::Instrumental)
    }

    pub fn is_synced(&self) -> bool {
        matches!(self, Lyrics::Synced(_))
    }
}

/// Lyrics plus where they came from, so the UI can credit the source and
/// show which entry matched.
#[derive(Debug, Clone, PartialEq)]
pub struct FoundLyrics {
    pub lyrics: Lyrics,
    /// Provider name, e.g. "LRCLIB".
    pub source: &'static str,
    pub matched_title: String,
    pub matched_artist: String,
    /// Length of the matched entry in milliseconds, if the provider said.
    pub matched_duration_ms: Option<u64>,
}

#[derive(Debug, thiserror::Error)]
pub enum LyricsError {
    #[error("request failed: {0}")]
    Http(#[from] ureq::Error),

    #[error("{0}")]
    Other(String),
}

/// A source of lyrics for a track.
pub trait LyricsProvider: Send + Sync {
    /// `Ok(None)` means the provider has nothing for this track. Errors are
    /// for transport failures that may succeed on retry.
    fn fetch(&self, track: &TrackInfo) -> Result<Option<FoundLyrics>, LyricsError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefers_synced_then_plain_then_instrumental() {
        let synced = "[00:01.00]line";
        assert!(matches!(
            Lyrics::from_parts(Some(synced), Some("line"), false),
            Some(Lyrics::Synced(_))
        ));
        assert_eq!(
            Lyrics::from_parts(None, Some("  line \n"), false),
            Some(Lyrics::Plain("line".into()))
        );
        assert_eq!(
            Lyrics::from_parts(None, None, true),
            Some(Lyrics::Instrumental)
        );
        assert_eq!(Lyrics::from_parts(None, None, false), None);
    }

    #[test]
    fn unusable_synced_falls_back_to_plain() {
        assert_eq!(
            Lyrics::from_parts(Some("no timestamps here"), Some("line"), false),
            Some(Lyrics::Plain("line".into()))
        );
        assert_eq!(Lyrics::from_parts(Some(""), Some(" "), false), None);
    }
}
