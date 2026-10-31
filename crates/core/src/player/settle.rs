//! Decides when the player's track metadata can be trusted.
//!
//! On a track change Spotify updates its SMTC timeline (duration, position)
//! first and its media properties (title, artist) 200–800 ms later. In
//! between, a snapshot pairs the *old* title with the *new* duration. Looking
//! up lyrics for that hybrid finds nothing or the wrong song, so we only
//! treat a track as current once its metadata has stopped changing for a
//! while.

use super::TrackInfo;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettleEvent {
    /// A track's metadata has been stable long enough to act on.
    Settled(TrackInfo),
    /// No usable track for long enough: the player stopped or closed.
    Cleared,
}

pub struct TrackSettler {
    settle_ms: i64,
    candidate: Option<TrackInfo>,
    candidate_since_ms: i64,
    settled: Option<TrackInfo>,
}

impl TrackSettler {
    /// `settle_ms` must exceed the gap between Spotify's timeline and
    /// metadata updates; we observed up to ~850 ms.
    pub fn new(settle_ms: i64) -> Self {
        Self {
            settle_ms,
            candidate: None,
            candidate_since_ms: 0,
            settled: None,
        }
    }

    /// The track most recently reported as settled, if any.
    pub fn settled(&self) -> Option<&TrackInfo> {
        self.settled.as_ref()
    }

    /// Feeds the latest metadata reading. Returns an event when the settled
    /// track changes.
    pub fn observe(&mut self, track: Option<&TrackInfo>, now_ms: i64) -> Option<SettleEvent> {
        // Spotify reports an empty title while shutting down.
        let track = track.filter(|t| !t.title.trim().is_empty());

        if track != self.candidate.as_ref() {
            self.candidate = track.cloned();
            self.candidate_since_ms = now_ms;
        }

        let stable = now_ms - self.candidate_since_ms >= self.settle_ms;
        if !stable || self.candidate == self.settled {
            return None;
        }

        self.settled = self.candidate.clone();
        Some(match &self.settled {
            Some(track) => SettleEvent::Settled(track.clone()),
            None => SettleEvent::Cleared,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(title: &str, artist: &str, duration_ms: u64) -> TrackInfo {
        TrackInfo {
            title: title.into(),
            artist: artist.into(),
            album: String::new(),
            duration_ms,
        }
    }

    fn settled(track: &TrackInfo) -> Option<SettleEvent> {
        Some(SettleEvent::Settled(track.clone()))
    }

    #[test]
    fn settles_after_quiet_period() {
        let mut s = TrackSettler::new(1_000);
        let a = t("A", "X", 100_000);
        assert_eq!(s.observe(Some(&a), 0), None);
        assert_eq!(s.observe(Some(&a), 999), None);
        assert_eq!(s.observe(Some(&a), 1_000), settled(&a));
        // Only reported once.
        assert_eq!(s.observe(Some(&a), 5_000), None);
        assert_eq!(s.settled(), Some(&a));
    }

    /// Replays the metadata sequence recorded in a real `drift.csv`
    /// (timestamps relative to the first track change).
    #[test]
    fn ignores_hybrid_metadata_during_track_change() {
        let mut s = TrackSettler::new(1_000);
        let timebomb = t("TIMEBOMB", "izna", 171_232);
        let hybrid_1 = t("TIMEBOMB", "izna", 151_373); // old title, new duration
        let eta_short = t("ETA", "NewJeans", 151_373);
        let hybrid_2 = t("ETA", "NewJeans", 152_400); // user skipped again
        let plot_twist = t("plot twist", "TWS", 152_400);

        s.observe(Some(&timebomb), -10_000);
        assert_eq!(s.observe(Some(&timebomb), -1), settled(&timebomb));

        let mut events = Vec::new();
        let timeline = [
            (0, &hybrid_1),
            (203, &eta_short),
            (2_218, &hybrid_2),
            (3_026, &plot_twist),
        ];
        let mut i = 0;
        for now in (0..6_000).step_by(100) {
            while i + 1 < timeline.len() && timeline[i + 1].0 <= now {
                i += 1;
            }
            let reading = if now >= timeline[i].0 {
                timeline[i].1
            } else {
                &timebomb
            };
            if let Some(e) = s.observe(Some(reading), now) {
                events.push((now, e));
            }
        }

        // The hybrids never settle; the two real tracks do.
        assert_eq!(
            events,
            vec![
                (1_300, SettleEvent::Settled(eta_short.clone())),
                (4_100, SettleEvent::Settled(plot_twist.clone())),
            ]
        );
    }

    #[test]
    fn empty_title_counts_as_no_track() {
        let mut s = TrackSettler::new(1_000);
        let a = t("A", "X", 100_000);
        s.observe(Some(&a), 0);
        s.observe(Some(&a), 1_000);
        let blank = t("", "", 0);
        assert_eq!(s.observe(Some(&blank), 2_000), None);
        assert_eq!(s.observe(None, 2_500), None);
        assert_eq!(s.observe(None, 3_000), Some(SettleEvent::Cleared));
        assert_eq!(s.settled(), None);
    }

    #[test]
    fn brief_dropout_does_not_clear() {
        let mut s = TrackSettler::new(1_000);
        let a = t("A", "X", 100_000);
        s.observe(Some(&a), 0);
        s.observe(Some(&a), 1_000);
        assert_eq!(s.observe(None, 1_100), None);
        assert_eq!(s.observe(Some(&a), 1_500), None);
        assert_eq!(s.observe(Some(&a), 3_000), None);
        assert_eq!(s.settled(), Some(&a));
    }

    #[test]
    fn nothing_before_first_track() {
        let mut s = TrackSettler::new(1_000);
        assert_eq!(s.observe(None, 0), None);
        assert_eq!(s.observe(None, 5_000), None);
    }
}
