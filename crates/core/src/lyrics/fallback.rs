//! A main lyrics source with a backup.
//!
//! The backup is asked when the main source has nothing synced: no entry,
//! only plain text, or a failure (it being down is the main reason to have
//! a backup). The main source's answer wins whenever the backup can't do
//! better, and its error is the one reported, so a lookup that failed for
//! the main source is still retried later.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use super::{FoundLyrics, Lyrics, LyricsError, LyricsProvider};
use crate::player::TrackInfo;

pub struct Fallback {
    main: Box<dyn LyricsProvider>,
    backup: Box<dyn LyricsProvider>,
    /// Off: only the main source is used. Shared with the settings.
    enabled: Arc<AtomicBool>,
}

impl Fallback {
    pub fn new(
        main: Box<dyn LyricsProvider>,
        backup: Box<dyn LyricsProvider>,
        enabled: Arc<AtomicBool>,
    ) -> Self {
        Self {
            main,
            backup,
            enabled,
        }
    }

    fn ask_backup(&self, track: &TrackInfo) -> Option<FoundLyrics> {
        match self.backup.fetch(track) {
            Ok(found) => found,
            Err(e) => {
                log::warn!("backup lookup for {:?} failed: {e}", track.title);
                None
            }
        }
    }
}

impl LyricsProvider for Fallback {
    fn fetch(&self, track: &TrackInfo) -> Result<Option<FoundLyrics>, LyricsError> {
        let main = self.main.fetch(track);
        if !self.enabled.load(Ordering::Relaxed) {
            return main;
        }
        match main {
            Ok(Some(found)) if !matches!(found.lyrics, Lyrics::Plain(_)) => Ok(Some(found)),
            Ok(Some(plain)) => Ok(Some(
                self.ask_backup(track)
                    .filter(|b| b.lyrics.is_synced())
                    .unwrap_or(plain),
            )),
            Ok(None) => self.backup.fetch(track),
            Err(e) => match self.ask_backup(track) {
                Some(found) => {
                    log::info!("main source failed ({e}); using the backup's lyrics");
                    Ok(Some(found))
                }
                None => Err(e),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lyrics::SyncedLyrics;
    use std::sync::atomic::AtomicUsize;

    #[derive(Clone, Copy)]
    enum Answer {
        Synced,
        Plain,
        Nothing,
        Fails,
    }

    struct Fake {
        answer: Answer,
        name: &'static str,
        calls: Arc<AtomicUsize>,
    }

    impl LyricsProvider for Fake {
        fn fetch(&self, _: &TrackInfo) -> Result<Option<FoundLyrics>, LyricsError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let lyrics = match self.answer {
                Answer::Synced => Lyrics::Synced(SyncedLyrics::parse("[00:01.00]x")),
                Answer::Plain => Lyrics::Plain("x".into()),
                Answer::Nothing => return Ok(None),
                Answer::Fails => return Err(LyricsError::Other(format!("{} down", self.name))),
            };
            Ok(Some(FoundLyrics {
                lyrics,
                source: self.name.into(),
                matched_title: String::new(),
                matched_artist: String::new(),
                matched_duration_ms: None,
            }))
        }
    }

    fn run(main: Answer, backup: Answer, enabled: bool) -> (Result<Option<String>, String>, usize) {
        let calls = Arc::new(AtomicUsize::new(0));
        let chain = Fallback::new(
            Box::new(Fake {
                answer: main,
                name: "main",
                calls: Arc::new(AtomicUsize::new(0)),
            }),
            Box::new(Fake {
                answer: backup,
                name: "backup",
                calls: Arc::clone(&calls),
            }),
            Arc::new(AtomicBool::new(enabled)),
        );
        let track = TrackInfo {
            title: "t".into(),
            artist: "a".into(),
            album: String::new(),
            duration_ms: 1,
        };
        let result = chain
            .fetch(&track)
            .map(|f| f.map(|f| f.source))
            .map_err(|e| e.to_string());
        (result, calls.load(Ordering::SeqCst))
    }

    use Answer::*;

    #[test]
    fn main_synced_lyrics_win_without_asking_the_backup() {
        assert_eq!(run(Synced, Synced, true), (Ok(Some("main".into())), 0));
    }

    #[test]
    fn backup_fills_in_when_main_has_nothing_or_fails() {
        assert_eq!(run(Nothing, Synced, true).0, Ok(Some("backup".into())));
        assert_eq!(run(Fails, Synced, true).0, Ok(Some("backup".into())));
        assert_eq!(run(Nothing, Nothing, true).0, Ok(None));
    }

    #[test]
    fn backup_only_replaces_plain_text_with_synced() {
        assert_eq!(run(Plain, Synced, true).0, Ok(Some("backup".into())));
        assert_eq!(run(Plain, Plain, true).0, Ok(Some("main".into())));
        assert_eq!(run(Plain, Fails, true).0, Ok(Some("main".into())));
    }

    #[test]
    fn the_main_error_is_reported_so_the_lookup_is_retried() {
        assert_eq!(run(Fails, Nothing, true).0, Err("main down".into()));
        assert_eq!(run(Fails, Fails, true).0, Err("main down".into()));
    }

    #[test]
    fn turned_off_means_main_only() {
        assert_eq!(run(Nothing, Synced, false), (Ok(None), 0));
        assert_eq!(run(Fails, Synced, false).0, Err("main down".into()));
    }
}
