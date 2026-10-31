//! On-disk lyrics cache, so a track seen before shows its lyrics instantly,
//! offline, and without asking LRCLIB again.
//!
//! - Found lyrics are kept indefinitely; a wrong match can be cleared with
//!   [`LyricsCache::remove`] ("look up lyrics again").
//! - "Nothing found" is kept for [`MISSING_TTL_SECS`], since LRCLIB keeps
//!   growing and a later lookup may succeed.
//! - Lookup errors are never cached.
//!
//! The cache is disposable: an unreadable or outdated file is recreated,
//! and [`CachedProvider`] falls back to the network on any cache error.

use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, params};

use super::{FoundLyrics, Lyrics, LyricsError, LyricsProvider, SyncedLyrics};
use crate::player::TrackInfo;

/// Bump when the table layout changes; older caches are dropped.
const SCHEMA_VERSION: i32 = 1;

/// How long a "no lyrics found" answer is trusted.
pub const MISSING_TTL_SECS: i64 = 3 * 24 * 60 * 60;

#[derive(Debug, thiserror::Error)]
#[error("lyrics cache: {0}")]
pub struct CacheError(#[from] rusqlite::Error);

pub struct LyricsCache {
    conn: Mutex<Connection>,
}

/// A cached answer: `Some(found)` or a cached "nothing found" (`None`).
pub type CachedAnswer = Option<FoundLyrics>;

impl LyricsCache {
    /// Opens (or creates) the cache at `path`, creating parent folders.
    pub fn open(path: &Path) -> Result<Self, CacheError> {
        if let Some(dir) = path.parent() {
            // A missing folder surfaces as an open error below.
            let _ = std::fs::create_dir_all(dir);
        }
        Self::init(Connection::open(path)?)
    }

    pub fn in_memory() -> Result<Self, CacheError> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self, CacheError> {
        let version: i32 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if version != SCHEMA_VERSION {
            conn.execute_batch(&format!(
                "DROP TABLE IF EXISTS lyrics;
                 CREATE TABLE lyrics (
                     title       TEXT    NOT NULL,
                     artist      TEXT    NOT NULL,
                     album       TEXT    NOT NULL,
                     duration_ms INTEGER NOT NULL,
                     kind        TEXT    NOT NULL,  -- synced | plain | instrumental | missing
                     body        TEXT,              -- LRC or plain text
                     source      TEXT,
                     matched_title    TEXT,
                     matched_artist   TEXT,
                     matched_duration_ms INTEGER,
                     fetched_at  INTEGER NOT NULL,  -- unix seconds
                     PRIMARY KEY (title, artist, album, duration_ms)
                 );
                 PRAGMA user_version = {SCHEMA_VERSION};"
            ))?;
        }
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn conn(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The cached answer for `track`, or `None` if there is none (or only
    /// an expired "nothing found").
    pub fn get(
        &self,
        track: &TrackInfo,
        now_unix_s: i64,
    ) -> Result<Option<CachedAnswer>, CacheError> {
        let row = self
            .conn()
            .query_row(
                "SELECT kind, body, source, matched_title, matched_artist,
                        matched_duration_ms, fetched_at
                 FROM lyrics
                 WHERE title = ?1 AND artist = ?2 AND album = ?3 AND duration_ms = ?4",
                params![
                    track.title,
                    track.artist,
                    track.album,
                    track.duration_ms as i64
                ],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, Option<i64>>(5)?,
                        row.get::<_, i64>(6)?,
                    ))
                },
            )
            .optional()?;

        let Some((kind, body, source, title, artist, duration, fetched_at)) = row else {
            return Ok(None);
        };
        let lyrics = match (kind.as_str(), body) {
            ("missing", _) if now_unix_s - fetched_at < MISSING_TTL_SECS => return Ok(Some(None)),
            ("missing", _) => return Ok(None),
            ("synced", Some(lrc)) => Lyrics::Synced(SyncedLyrics::parse(&lrc)),
            ("plain", Some(text)) => Lyrics::Plain(text),
            ("instrumental", _) => Lyrics::Instrumental,
            // Unknown kind or missing body: treat as not cached.
            _ => return Ok(None),
        };
        Ok(Some(Some(FoundLyrics {
            lyrics,
            source: source.unwrap_or_default(),
            matched_title: title.unwrap_or_default(),
            matched_artist: artist.unwrap_or_default(),
            matched_duration_ms: duration.map(|d| d as u64),
        })))
    }

    pub fn put(
        &self,
        track: &TrackInfo,
        answer: &CachedAnswer,
        now_unix_s: i64,
    ) -> Result<(), CacheError> {
        let (kind, body) = match answer.as_ref().map(|f| &f.lyrics) {
            None => ("missing", None),
            Some(Lyrics::Synced(s)) => ("synced", Some(s.to_lrc())),
            Some(Lyrics::Plain(text)) => ("plain", Some(text.clone())),
            Some(Lyrics::Instrumental) => ("instrumental", None),
        };
        self.conn().execute(
            "INSERT OR REPLACE INTO lyrics
                 (title, artist, album, duration_ms, kind, body, source,
                  matched_title, matched_artist, matched_duration_ms, fetched_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                track.title,
                track.artist,
                track.album,
                track.duration_ms as i64,
                kind,
                body,
                answer.as_ref().map(|f| &f.source),
                answer.as_ref().map(|f| &f.matched_title),
                answer.as_ref().map(|f| &f.matched_artist),
                answer
                    .as_ref()
                    .and_then(|f| f.matched_duration_ms)
                    .map(|d| d as i64),
                now_unix_s,
            ],
        )?;
        Ok(())
    }

    pub fn remove(&self, track: &TrackInfo) -> Result<(), CacheError> {
        self.conn().execute(
            "DELETE FROM lyrics WHERE title = ?1 AND artist = ?2 AND album = ?3 AND duration_ms = ?4",
            params![track.title, track.artist, track.album, track.duration_ms as i64],
        )?;
        Ok(())
    }
}

fn now_unix_s() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Wraps a provider with the cache: answers from the cache when it can,
/// otherwise asks `inner` and remembers the answer.
pub struct CachedProvider<P> {
    inner: P,
    cache: Arc<LyricsCache>,
}

impl<P> CachedProvider<P> {
    pub fn new(inner: P, cache: Arc<LyricsCache>) -> Self {
        Self { inner, cache }
    }
}

impl<P: LyricsProvider> LyricsProvider for CachedProvider<P> {
    fn fetch(&self, track: &TrackInfo) -> Result<Option<FoundLyrics>, LyricsError> {
        match self.cache.get(track, now_unix_s()) {
            Ok(Some(answer)) => return Ok(answer),
            Ok(None) => {}
            Err(e) => log::warn!("{e}; asking the provider"),
        }
        let answer = self.inner.fetch(track)?;
        if let Err(e) = self.cache.put(track, &answer, now_unix_s()) {
            log::warn!("{e}; not cached");
        }
        Ok(answer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn track(title: &str) -> TrackInfo {
        TrackInfo {
            title: title.into(),
            artist: "Artist".into(),
            album: "Album".into(),
            duration_ms: 200_000,
        }
    }

    fn found(lyrics: Lyrics) -> CachedAnswer {
        Some(FoundLyrics {
            lyrics,
            source: "LRCLIB".into(),
            matched_title: "Song".into(),
            matched_artist: "Artist".into(),
            matched_duration_ms: Some(199_000),
        })
    }

    fn synced() -> CachedAnswer {
        found(Lyrics::Synced(SyncedLyrics::parse(
            "[00:01.00]made up one\n[00:01.00]translation\n[00:03.50]made up two",
        )))
    }

    #[test]
    fn stores_every_kind_of_answer() {
        let cache = LyricsCache::in_memory().unwrap();
        let cases = [
            ("synced", synced()),
            ("plain", found(Lyrics::Plain("a\nb".into()))),
            ("instrumental", found(Lyrics::Instrumental)),
            ("missing", None),
        ];
        for (title, answer) in &cases {
            cache.put(&track(title), answer, 1_000).unwrap();
        }
        for (title, answer) in &cases {
            assert_eq!(
                cache.get(&track(title), 1_000).unwrap(),
                Some(answer.clone()),
                "{title}"
            );
        }
        assert_eq!(cache.get(&track("never seen"), 1_000).unwrap(), None);
    }

    #[test]
    fn key_includes_album_and_duration() {
        let cache = LyricsCache::in_memory().unwrap();
        cache.put(&track("Song"), &synced(), 0).unwrap();
        let mut other = track("Song");
        other.duration_ms += 1;
        assert_eq!(cache.get(&other, 0).unwrap(), None);
        other = track("Song");
        other.album = "Live".into();
        assert_eq!(cache.get(&other, 0).unwrap(), None);
    }

    #[test]
    fn missing_answers_expire_found_ones_dont() {
        let cache = LyricsCache::in_memory().unwrap();
        cache.put(&track("Missing"), &None, 0).unwrap();
        cache.put(&track("Found"), &synced(), 0).unwrap();
        let later = MISSING_TTL_SECS;
        assert_eq!(cache.get(&track("Missing"), later - 1).unwrap(), Some(None));
        assert_eq!(cache.get(&track("Missing"), later).unwrap(), None);
        assert_eq!(
            cache.get(&track("Found"), later * 100).unwrap(),
            Some(synced())
        );
    }

    #[test]
    fn remove_and_replace() {
        let cache = LyricsCache::in_memory().unwrap();
        cache.put(&track("Song"), &None, 0).unwrap();
        cache.put(&track("Song"), &synced(), 1).unwrap();
        assert_eq!(cache.get(&track("Song"), 1).unwrap(), Some(synced()));
        cache.remove(&track("Song")).unwrap();
        assert_eq!(cache.get(&track("Song"), 1).unwrap(), None);
    }

    #[test]
    fn persists_and_recreates_outdated_files() {
        let dir = std::env::temp_dir().join(format!("lyrics-cache-test-{}", std::process::id()));
        let path = dir.join("nested").join("lyrics.sqlite3");
        {
            let cache = LyricsCache::open(&path).unwrap();
            cache.put(&track("Song"), &synced(), 0).unwrap();
        }
        assert_eq!(
            LyricsCache::open(&path)
                .unwrap()
                .get(&track("Song"), 0)
                .unwrap(),
            Some(synced())
        );

        // A cache from another schema version is dropped, not misread.
        Connection::open(&path)
            .unwrap()
            .execute_batch("PRAGMA user_version = 999;")
            .unwrap();
        assert_eq!(
            LyricsCache::open(&path)
                .unwrap()
                .get(&track("Song"), 0)
                .unwrap(),
            None
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    struct Counting {
        calls: AtomicUsize,
        answer: Result<CachedAnswer, ()>,
    }

    impl LyricsProvider for Counting {
        fn fetch(&self, _: &TrackInfo) -> Result<Option<FoundLyrics>, LyricsError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.answer
                .clone()
                .map_err(|()| LyricsError::Other("offline".into()))
        }
    }

    #[test]
    fn provider_is_asked_once_per_track() {
        let cache = Arc::new(LyricsCache::in_memory().unwrap());
        let provider = CachedProvider::new(
            Counting {
                calls: AtomicUsize::new(0),
                answer: Ok(synced()),
            },
            Arc::clone(&cache),
        );
        assert_eq!(provider.fetch(&track("Song")).unwrap(), synced());
        assert_eq!(provider.fetch(&track("Song")).unwrap(), synced());
        assert_eq!(provider.inner.calls.load(Ordering::SeqCst), 1);

        // "Look up again": clearing the entry makes the next fetch ask.
        cache.remove(&track("Song")).unwrap();
        provider.fetch(&track("Song")).unwrap();
        assert_eq!(provider.inner.calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn errors_are_not_cached() {
        let cache = Arc::new(LyricsCache::in_memory().unwrap());
        let provider = CachedProvider::new(
            Counting {
                calls: AtomicUsize::new(0),
                answer: Err(()),
            },
            Arc::clone(&cache),
        );
        assert!(provider.fetch(&track("Song")).is_err());
        assert!(provider.fetch(&track("Song")).is_err());
        assert_eq!(provider.inner.calls.load(Ordering::SeqCst), 2);
        assert_eq!(cache.get(&track("Song"), now_unix_s()).unwrap(), None);
    }
}
