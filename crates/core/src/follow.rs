//! Follows a player: decides which track is current, asks for its lyrics,
//! and tracks which lyric line is being sung.
//!
//! [`Follower`] makes every decision but performs no I/O. It is fed player
//! snapshots and fetch results, and answers with [`FollowEvent`]s that a
//! front end (terminal, overlay window) renders. Fetching is requested via
//! [`FollowEvent::FetchRequested`] and reported back through
//! [`Follower::on_fetched`], so callers choose their own threading.

use std::collections::HashMap;

use crate::lyrics::{FoundLyrics, Lyrics, SyncedLyrics};
use crate::player::{PlayerSnapshot, SettleEvent, TrackInfo, TrackSettler};

/// Waits before refetching after a failed lookup. A track plays for
/// minutes, so it's worth outlasting an LRCLIB overload (503) that the
/// transport's own short retries didn't.
pub const RETRY_DELAYS_MS: [i64; 4] = [5_000, 15_000, 30_000, 60_000];

/// Backward position corrections smaller than this are treated as noise,
/// not seeks. Spotify's report on pause lands ~200 ms behind playback, which
/// would otherwise flash the previous line.
pub const BACKTRACK_TOLERANCE_MS: i64 = 500;

#[derive(Debug, Clone, PartialEq)]
pub enum FollowEvent {
    /// A track became current (its metadata settled).
    TrackChanged(TrackInfo),
    /// Nothing is playing any more.
    Cleared,
    /// Fetch lyrics for this track and report back with
    /// [`Follower::on_fetched`].
    FetchRequested(TrackInfo),
    /// The lookup for the current track finished, from cache or a fetch.
    /// `None` means no lyrics exist for it.
    LyricsReady(Option<FoundLyrics>),
    /// The lookup for the current track failed. `retry_in_ms` is `None` once
    /// retries are used up.
    LookupFailed {
        error: String,
        retry_in_ms: Option<i64>,
    },
    /// The synced line being sung changed. `None` before the first line.
    LineChanged(Option<usize>),
}

/// Where the lyrics lookup for the current track stands.
#[derive(Debug, Clone, PartialEq)]
pub enum LookupState {
    /// No current track.
    Idle,
    /// `attempt` counts lookups started for the current track, from 1.
    Loading {
        attempt: usize,
    },
    Ready(Option<FoundLyrics>),
    /// The last lookup failed. `retry_at_ms` is when to try again, or
    /// `None` once retries are used up.
    Failed {
        attempt: usize,
        retry_at_ms: Option<i64>,
    },
}

pub struct Follower {
    settler: TrackSettler,
    offset_ms: i64,
    /// Results by track, including "nothing found", so revisiting a track
    /// (or repeat-one) doesn't refetch. Errors are not cached.
    cache: HashMap<TrackInfo, Option<FoundLyrics>>,
    state: LookupState,
    line: Option<usize>,
}

impl Follower {
    /// `settle_ms`: see [`TrackSettler::new`]. `offset_ms`: shifts lyric
    /// timing; positive shows lines earlier.
    pub fn new(settle_ms: i64, offset_ms: i64) -> Self {
        Self {
            settler: TrackSettler::new(settle_ms),
            offset_ms,
            cache: HashMap::new(),
            state: LookupState::Idle,
            line: None,
        }
    }

    /// The current (settled) track.
    pub fn track(&self) -> Option<&TrackInfo> {
        self.settler.settled()
    }

    pub fn state(&self) -> &LookupState {
        &self.state
    }

    /// Synced lyrics of the current track, if that's what was found.
    pub fn synced(&self) -> Option<&SyncedLyrics> {
        match &self.state {
            LookupState::Ready(Some(FoundLyrics {
                lyrics: Lyrics::Synced(s),
                ..
            })) => Some(s),
            _ => None,
        }
    }

    /// Index of the synced line being sung, as last reported.
    pub fn line(&self) -> Option<usize> {
        self.line
    }

    pub fn offset_ms(&self) -> i64 {
        self.offset_ms
    }

    pub fn set_offset_ms(&mut self, offset_ms: i64) {
        self.offset_ms = offset_ms;
    }

    /// Playback position in lyric time (offset applied) at `now_ms`.
    pub fn lyric_position_ms(&self, snapshot: &PlayerSnapshot, now_ms: i64) -> u64 {
        (snapshot.estimated_position_ms(now_ms) as i64 + self.offset_ms).max(0) as u64
    }

    /// Processes one player reading.
    pub fn tick(&mut self, snapshot: Option<&PlayerSnapshot>, now_ms: i64) -> Vec<FollowEvent> {
        let mut events = Vec::new();

        match self.settler.observe(snapshot.map(|s| &s.track), now_ms) {
            Some(SettleEvent::Settled(track)) => {
                self.line = None;
                events.push(FollowEvent::TrackChanged(track.clone()));
                match self.cache.get(&track).cloned() {
                    Some(found) => {
                        self.state = LookupState::Ready(found.clone());
                        events.push(FollowEvent::LyricsReady(found));
                    }
                    None => {
                        self.state = LookupState::Loading { attempt: 1 };
                        events.push(FollowEvent::FetchRequested(track));
                    }
                }
            }
            Some(SettleEvent::Cleared) => {
                self.state = LookupState::Idle;
                self.line = None;
                events.push(FollowEvent::Cleared);
            }
            None => {}
        }

        if let LookupState::Failed {
            attempt,
            retry_at_ms: Some(retry_at),
        } = self.state
        {
            // Failed is only ever set for the settled track.
            if let (true, Some(track)) = (now_ms >= retry_at, self.settler.settled()) {
                events.push(FollowEvent::FetchRequested(track.clone()));
                self.state = LookupState::Loading {
                    attempt: attempt + 1,
                };
            }
        }

        if let Some(snapshot) = snapshot {
            events.extend(self.follow(snapshot, now_ms));
        }
        events
    }

    fn follow(&mut self, snapshot: &PlayerSnapshot, now_ms: i64) -> Option<FollowEvent> {
        // While a track change is in flight the snapshot can mix two
        // tracks; only follow when it is exactly the settled one.
        if self.settler.settled() != Some(&snapshot.track) {
            return None;
        }
        let synced = self.synced()?;

        let position = self.lyric_position_ms(snapshot, now_ms) as i64;
        let mut index = synced.line_index_at(position as u64);
        if let Some(current) = self.line.and_then(|i| synced.lines().get(i)) {
            let small_step_back =
                index < self.line && position + BACKTRACK_TOLERANCE_MS >= current.start_ms as i64;
            if small_step_back {
                index = self.line;
            }
        }
        if index == self.line {
            return None;
        }
        self.line = index;
        Some(FollowEvent::LineChanged(index))
    }

    /// Handles a finished fetch. Results for a track that is no longer
    /// current are cached but not reported; failures for the current track
    /// are retried with backoff.
    pub fn on_fetched(
        &mut self,
        track: TrackInfo,
        result: Result<Option<FoundLyrics>, String>,
        now_ms: i64,
    ) -> Vec<FollowEvent> {
        let current = self.settler.settled() == Some(&track);
        match result {
            Ok(found) => {
                self.cache.insert(track, found.clone());
                if !current {
                    return Vec::new();
                }
                self.state = LookupState::Ready(found.clone());
                vec![FollowEvent::LyricsReady(found)]
            }
            Err(error) if current => {
                let attempt = match self.state {
                    LookupState::Loading { attempt } => attempt,
                    _ => 1,
                };
                let retry_in_ms = RETRY_DELAYS_MS.get(attempt - 1).copied();
                self.state = LookupState::Failed {
                    attempt,
                    retry_at_ms: retry_in_ms.map(|d| now_ms + d),
                };
                vec![FollowEvent::LookupFailed { error, retry_in_ms }]
            }
            Err(_) => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::player::PlaybackStatus;

    const LRC: &str = "[00:01.00]line one\n[00:03.00]line two\n[00:03.00]translation two\n\
                       [00:05.00]\n[00:06.00]line four";

    fn track(title: &str) -> TrackInfo {
        TrackInfo {
            title: title.into(),
            artist: "Artist".into(),
            album: String::new(),
            duration_ms: 200_000,
        }
    }

    /// Playing, with position `pos` reported at time `at`.
    fn playing(t: &TrackInfo, pos: u64, at: i64) -> PlayerSnapshot {
        PlayerSnapshot {
            track: t.clone(),
            status: PlaybackStatus::Playing,
            reported_position_ms: pos,
            position_updated_unix_ms: at,
        }
    }

    fn paused(t: &TrackInfo, pos: u64, at: i64) -> PlayerSnapshot {
        PlayerSnapshot {
            status: PlaybackStatus::Paused,
            ..playing(t, pos, at)
        }
    }

    fn found(lyrics: Lyrics) -> Option<FoundLyrics> {
        Some(FoundLyrics {
            lyrics,
            source: "LRCLIB",
            matched_title: "Song".into(),
            matched_artist: "Artist".into(),
            matched_duration_ms: Some(200_000),
        })
    }

    fn synced() -> Option<FoundLyrics> {
        found(Lyrics::Synced(SyncedLyrics::parse(LRC)))
    }

    fn line(i: usize) -> Vec<FollowEvent> {
        vec![FollowEvent::LineChanged(Some(i))]
    }

    fn fetches(events: &[FollowEvent]) -> usize {
        events
            .iter()
            .filter(|e| matches!(e, FollowEvent::FetchRequested(_)))
            .count()
    }

    /// Settles `t` (playing from position 0 at time 0) and delivers `result`.
    fn settled_with(t: &TrackInfo, result: Option<FoundLyrics>) -> Follower {
        let mut f = Follower::new(1_000, 0);
        assert!(f.tick(Some(&playing(t, 0, 0)), 0).is_empty());
        let events = f.tick(Some(&playing(t, 0, 0)), 1_000);
        assert!(events.contains(&FollowEvent::FetchRequested(t.clone())));
        f.on_fetched(t.clone(), Ok(result), 1_000);
        f
    }

    #[test]
    fn requests_fetch_once_track_settles() {
        let t = track("Song");
        let mut f = Follower::new(1_000, 0);
        assert!(f.tick(Some(&playing(&t, 0, 0)), 0).is_empty());
        assert!(f.tick(Some(&playing(&t, 0, 0)), 500).is_empty());
        assert_eq!(
            f.tick(Some(&playing(&t, 0, 0)), 1_000),
            vec![
                FollowEvent::TrackChanged(t.clone()),
                FollowEvent::FetchRequested(t.clone()),
            ]
        );
        assert_eq!(f.state(), &LookupState::Loading { attempt: 1 });
        assert_eq!(f.track(), Some(&t));
    }

    #[test]
    fn reports_each_line_once_as_playback_reaches_it() {
        let t = track("Song");
        let mut f = settled_with(&t, synced());
        let snap = playing(&t, 0, 0);
        assert_eq!(f.tick(Some(&snap), 1_100), line(0));
        assert!(f.tick(Some(&snap), 2_900).is_empty());
        // Original and translation share 3.0 s; the later one is reported
        // and front ends show the group.
        assert_eq!(f.tick(Some(&snap), 3_000), line(2));
        assert_eq!(f.tick(Some(&snap), 5_100), line(3));
        assert_eq!(f.tick(Some(&snap), 6_000), line(4));
        assert!(f.tick(Some(&snap), 9_000).is_empty());
        assert_eq!(f.line(), Some(4));
    }

    #[test]
    fn seeking_back_moves_to_the_new_line() {
        let t = track("Song");
        let mut f = settled_with(&t, synced());
        f.tick(Some(&playing(&t, 0, 0)), 6_500);
        assert_eq!(f.tick(Some(&playing(&t, 1_200, 7_000)), 7_000), line(0));
        assert_eq!(
            f.tick(Some(&playing(&t, 0, 8_000)), 8_000),
            vec![FollowEvent::LineChanged(None)]
        );
    }

    #[test]
    fn pause_report_slightly_behind_does_not_flash_previous_line() {
        let t = track("Song");
        let mut f = settled_with(&t, synced());
        assert_eq!(f.tick(Some(&playing(&t, 0, 0)), 6_100), line(4));
        // Paused: Spotify reports 5.85 s, before line four's 6.0 s start.
        assert!(f.tick(Some(&paused(&t, 5_850, 6_150)), 6_200).is_empty());
        // A real seek back (beyond the tolerance) still moves.
        assert_eq!(f.tick(Some(&paused(&t, 5_200, 7_000)), 7_000), line(3));
    }

    #[test]
    fn offset_shifts_lines_earlier() {
        let t = track("Song");
        let mut f = Follower::new(1_000, 500);
        f.tick(Some(&playing(&t, 0, 0)), 0);
        f.tick(Some(&playing(&t, 0, 0)), 1_000);
        f.on_fetched(t.clone(), Ok(synced()), 1_000);
        // At 2.5 s playback, +500 ms reaches line two at 3.0 s.
        assert_eq!(f.tick(Some(&playing(&t, 0, 0)), 2_500), line(2));
        assert_eq!(f.lyric_position_ms(&playing(&t, 0, 0), 2_500), 3_000);
    }

    #[test]
    fn ignores_snapshots_that_mix_two_tracks() {
        let t = track("Song");
        let mut f = settled_with(&t, synced());
        f.tick(Some(&playing(&t, 0, 0)), 1_100);
        // Spotify updated the duration before the title.
        let mut hybrid = playing(&t, 0, 2_000);
        hybrid.track.duration_ms = 150_000;
        assert!(f.tick(Some(&hybrid), 4_000).is_empty());
    }

    #[test]
    fn late_result_for_previous_track_is_cached_not_reported() {
        let a = track("A");
        let b = track("B");
        let mut f = Follower::new(1_000, 0);
        f.tick(Some(&playing(&a, 0, 0)), 0);
        f.tick(Some(&playing(&a, 0, 0)), 1_000);
        f.tick(Some(&playing(&b, 0, 2_000)), 2_000);
        f.tick(Some(&playing(&b, 0, 2_000)), 3_000);
        assert!(f.on_fetched(a.clone(), Ok(synced()), 3_000).is_empty());
        assert_eq!(f.state(), &LookupState::Loading { attempt: 1 });

        // Back to A: served from cache, no new fetch.
        f.tick(Some(&playing(&a, 0, 4_000)), 4_000);
        let events = f.tick(Some(&playing(&a, 0, 4_000)), 5_000);
        assert_eq!(fetches(&events), 0);
        assert_eq!(events[1], FollowEvent::LyricsReady(synced()));
    }

    #[test]
    fn non_synced_results_are_reported_but_not_followed() {
        let t = track("Song");
        let mut f = settled_with(&t, found(Lyrics::Instrumental));
        assert!(f.synced().is_none());
        assert!(f.tick(Some(&playing(&t, 0, 0)), 5_000).is_empty());

        let mut f = Follower::new(1_000, 0);
        f.tick(Some(&playing(&t, 0, 0)), 0);
        f.tick(Some(&playing(&t, 0, 0)), 1_000);
        assert_eq!(
            f.on_fetched(t.clone(), Ok(None), 1_000),
            vec![FollowEvent::LyricsReady(None)]
        );
    }

    #[test]
    fn failed_lookups_are_retried_with_backoff_then_given_up() {
        let t = track("Song");
        let snap = playing(&t, 0, 0);
        let mut f = Follower::new(1_000, 0);
        f.tick(Some(&snap), 0);
        assert_eq!(fetches(&f.tick(Some(&snap), 1_000)), 1);

        let mut now = 1_000;
        for delay in RETRY_DELAYS_MS {
            assert_eq!(
                f.on_fetched(t.clone(), Err("http status: 503".into()), now),
                vec![FollowEvent::LookupFailed {
                    error: "http status: 503".into(),
                    retry_in_ms: Some(delay),
                }]
            );
            assert_eq!(fetches(&f.tick(Some(&snap), now + delay - 1)), 0);
            now += delay;
            assert_eq!(fetches(&f.tick(Some(&snap), now)), 1);
        }

        // The fifth lookup fails too: stop.
        let events = f.on_fetched(t.clone(), Err("http status: 503".into()), now);
        assert_eq!(
            events,
            vec![FollowEvent::LookupFailed {
                error: "http status: 503".into(),
                retry_in_ms: None,
            }]
        );
        assert_eq!(fetches(&f.tick(Some(&snap), now + 600_000)), 0);
    }

    #[test]
    fn retry_can_succeed() {
        let t = track("Song");
        let snap = playing(&t, 0, 0);
        let mut f = Follower::new(1_000, 0);
        f.tick(Some(&snap), 0);
        f.tick(Some(&snap), 1_000);
        f.on_fetched(t.clone(), Err("503".into()), 1_000);
        assert_eq!(fetches(&f.tick(Some(&snap), 6_000)), 1);
        assert_eq!(f.state(), &LookupState::Loading { attempt: 2 });
        assert_eq!(
            f.on_fetched(t.clone(), Ok(synced()), 6_500),
            vec![FollowEvent::LyricsReady(synced())]
        );
    }

    #[test]
    fn no_retry_after_switching_tracks() {
        let a = track("A");
        let b = track("B");
        let mut f = Follower::new(1_000, 0);
        f.tick(Some(&playing(&a, 0, 0)), 0);
        f.tick(Some(&playing(&a, 0, 0)), 1_000);
        f.on_fetched(a.clone(), Err("503".into()), 1_000);
        f.tick(Some(&playing(&b, 0, 2_000)), 2_000);
        let settle_b = f.tick(Some(&playing(&b, 0, 2_000)), 3_000);
        assert_eq!(
            settle_b.last(),
            Some(&FollowEvent::FetchRequested(b.clone()))
        );
        assert_eq!(fetches(&f.tick(Some(&playing(&b, 0, 2_000)), 10_000)), 0);
        // A's late failure is ignored.
        assert!(
            f.on_fetched(a.clone(), Err("503".into()), 10_000)
                .is_empty()
        );
    }

    #[test]
    fn player_closing_clears_state() {
        let t = track("Song");
        let mut f = settled_with(&t, synced());
        f.tick(None, 2_000);
        assert_eq!(f.tick(None, 3_000), vec![FollowEvent::Cleared]);
        assert_eq!(f.state(), &LookupState::Idle);
        assert_eq!(f.track(), None);
    }
}
