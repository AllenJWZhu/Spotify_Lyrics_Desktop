//! Several lyrics sources asked at once; the first good answer wins.
//!
//! Every enabled source is asked in parallel. The first one to come back
//! with synced lyrics (or to say the track is instrumental) is used straight
//! away, without waiting for the others: a source that is down and retrying
//! no longer holds up one that answers. Only when no source has synced
//! lyrics does the lookup wait for all of them, then takes plain text if
//! anyone had it, else reports "not found" or, if any source failed, the
//! failure, so the lookup is retried later.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Instant;

use super::{FoundLyrics, Lyrics, LyricsError, LyricsProvider};
use crate::player::TrackInfo;

struct Source {
    provider: Arc<dyn LyricsProvider>,
    /// `None`: always on. Otherwise a switch shared with the settings.
    enabled: Option<Arc<AtomicBool>>,
}

pub struct Racing {
    /// In order of preference: on a tie for "no synced lyrics", the
    /// earlier source's plain text or error is the one used.
    sources: Vec<Source>,
}

impl Racing {
    pub fn new() -> Self {
        Self {
            sources: Vec::new(),
        }
    }

    /// Adds a source that is always asked.
    pub fn with(mut self, provider: impl LyricsProvider + 'static) -> Self {
        self.sources.push(Source {
            provider: Arc::new(provider),
            enabled: None,
        });
        self
    }

    /// Adds a source that is asked only while `enabled` is set.
    pub fn with_switch(
        mut self,
        provider: impl LyricsProvider + 'static,
        enabled: Arc<AtomicBool>,
    ) -> Self {
        self.sources.push(Source {
            provider: Arc::new(provider),
            enabled: Some(enabled),
        });
        self
    }

    /// LRCLIB, and NetEase while `netease` is on.
    pub fn lrclib_and_netease(netease: Arc<AtomicBool>) -> Self {
        Self::new()
            .with(super::lrclib::Lrclib::new())
            .with_switch(super::netease::NetEase::new(), netease)
    }
}

impl Default for Racing {
    fn default() -> Self {
        Self::new()
    }
}

/// Synced lyrics or a definite "instrumental" end the race.
fn is_final(found: &FoundLyrics) -> bool {
    !matches!(found.lyrics, Lyrics::Plain(_))
}

impl LyricsProvider for Racing {
    fn fetch(&self, track: &TrackInfo) -> Result<Option<FoundLyrics>, LyricsError> {
        let started = Instant::now();
        let (tx, rx) = mpsc::channel();
        let mut asked = 0;
        for (rank, source) in self.sources.iter().enumerate() {
            if source
                .enabled
                .as_ref()
                .is_some_and(|on| !on.load(Ordering::Relaxed))
            {
                continue;
            }
            asked += 1;
            let provider = Arc::clone(&source.provider);
            let track = track.clone();
            let tx = tx.clone();
            // A source still retrying after the race is decided finishes in
            // the background; its answer goes nowhere.
            thread::spawn(move || {
                let _ = tx.send((rank, provider.fetch(&track)));
            });
        }
        drop(tx);

        // Best non-final answers so far, by source preference.
        let mut plain: Option<(usize, FoundLyrics)> = None;
        let mut error: Option<(usize, LyricsError)> = None;
        for (rank, result) in rx.iter().take(asked) {
            match result {
                Ok(Some(found)) if is_final(&found) => {
                    log::info!(
                        "{} answered first, after {} ms",
                        found.source,
                        started.elapsed().as_millis()
                    );
                    return Ok(Some(found));
                }
                Ok(Some(found)) => {
                    if plain.as_ref().is_none_or(|(r, _)| rank < *r) {
                        plain = Some((rank, found));
                    }
                }
                Ok(None) => {}
                Err(e) => {
                    log::warn!("lyrics lookup for {:?}: {e}", track.title);
                    if error.as_ref().is_none_or(|(r, _)| rank < *r) {
                        error = Some((rank, e));
                    }
                }
            }
        }
        match (plain, error) {
            (Some((_, found)), _) => Ok(Some(found)),
            (None, Some((_, e))) => Err(e),
            (None, None) => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lyrics::SyncedLyrics;
    use std::time::Duration;

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
        delay_ms: u64,
    }

    impl LyricsProvider for Fake {
        fn fetch(&self, _: &TrackInfo) -> Result<Option<FoundLyrics>, LyricsError> {
            thread::sleep(Duration::from_millis(self.delay_ms));
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

    fn fake(name: &'static str, answer: Answer, delay_ms: u64) -> Fake {
        Fake {
            answer,
            name,
            delay_ms,
        }
    }

    fn track() -> TrackInfo {
        TrackInfo {
            title: "t".into(),
            artist: "a".into(),
            album: String::new(),
            duration_ms: 1,
        }
    }

    /// (main, its delay), (backup, its delay), backup on → result, ms taken.
    fn race(
        main: (Answer, u64),
        backup: (Answer, u64),
        backup_on: bool,
    ) -> (Result<Option<String>, String>, u128) {
        let racing = Racing::new()
            .with(fake("main", main.0, main.1))
            .with_switch(
                fake("backup", backup.0, backup.1),
                Arc::new(AtomicBool::new(backup_on)),
            );
        let started = Instant::now();
        let result = racing
            .fetch(&track())
            .map(|f| f.map(|f| f.source))
            .map_err(|e| e.to_string());
        (result, started.elapsed().as_millis())
    }

    use Answer::*;

    #[test]
    fn the_first_synced_answer_wins_without_waiting_for_the_rest() {
        let (result, ms) = race((Fails, 600), (Synced, 20), true);
        assert_eq!(result, Ok(Some("backup".into())));
        assert!(ms < 400, "waited {ms} ms for the slow source");

        let (result, ms) = race((Synced, 20), (Synced, 600), true);
        assert_eq!(result, Ok(Some("main".into())));
        assert!(ms < 400, "waited {ms} ms for the slow source");
    }

    #[test]
    fn plain_text_waits_for_a_possible_synced_answer() {
        assert_eq!(
            race((Plain, 10), (Synced, 80), true).0,
            Ok(Some("backup".into()))
        );
        // Both plain: the preferred source's.
        assert_eq!(
            race((Plain, 80), (Plain, 10), true).0,
            Ok(Some("main".into()))
        );
        // Plain beats a failure.
        assert_eq!(
            race((Fails, 10), (Plain, 50), true).0,
            Ok(Some("backup".into()))
        );
    }

    #[test]
    fn nothing_anywhere_is_not_found_but_a_failure_is_retried() {
        assert_eq!(race((Nothing, 10), (Nothing, 10), true).0, Ok(None));
        assert_eq!(
            race((Fails, 10), (Nothing, 10), true).0,
            Err("main down".into())
        );
        assert_eq!(
            race((Nothing, 10), (Fails, 10), true).0,
            Err("backup down".into())
        );
        assert_eq!(
            race((Fails, 50), (Fails, 10), true).0,
            Err("main down".into())
        );
    }

    #[test]
    fn a_switched_off_source_is_not_asked() {
        assert_eq!(race((Nothing, 10), (Synced, 10), false).0, Ok(None));
        assert_eq!(
            race((Fails, 10), (Synced, 10), false).0,
            Err("main down".into())
        );
    }
}
