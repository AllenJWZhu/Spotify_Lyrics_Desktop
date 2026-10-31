//! Turns a stream of player snapshots into discrete events, so the CLI only
//! logs when something actually changed.

use lyrics_core::player::{PlayerSnapshot, anchor_drift_ms};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// First time we saw a Spotify session.
    Start,
    /// Spotify came back after being gone.
    SessionFound,
    /// Spotify closed or stopped publishing a session.
    SessionLost,
    TrackChange,
    /// Play / pause / stop.
    StatusChange,
    /// Same track and status, new position anchor. `drift_ms` is how far it
    /// landed from where the previous anchor predicted.
    AnchorUpdate {
        drift_ms: i64,
    },
    /// An anchor jump too large to be clock drift: the user seeked.
    Seek {
        jump_ms: i64,
    },
}

impl Event {
    pub fn name(&self) -> &'static str {
        match self {
            Event::Start => "start",
            Event::SessionFound => "session_found",
            Event::SessionLost => "session_lost",
            Event::TrackChange => "track_change",
            Event::StatusChange => "status_change",
            Event::AnchorUpdate { .. } => "anchor_update",
            Event::Seek { .. } => "seek",
        }
    }

    /// Drift (or seek distance) in ms, if the event carries one.
    pub fn delta_ms(&self) -> Option<i64> {
        match *self {
            Event::AnchorUpdate { drift_ms } => Some(drift_ms),
            Event::Seek { jump_ms } => Some(jump_ms),
            _ => None,
        }
    }
}

pub struct Monitor {
    last: Option<PlayerSnapshot>,
    seen_any: bool,
    seek_threshold_ms: i64,
}

impl Monitor {
    pub fn new(seek_threshold_ms: i64) -> Self {
        Self {
            last: None,
            seen_any: false,
            seek_threshold_ms,
        }
    }

    /// Feeds the latest reading and returns what changed since the previous one.
    pub fn observe(&mut self, next: Option<&PlayerSnapshot>) -> Option<Event> {
        let event = match (self.last.as_ref(), next) {
            (None, None) => None,
            (None, Some(_)) if self.seen_any => Some(Event::SessionFound),
            (None, Some(_)) => Some(Event::Start),
            (Some(_), None) => Some(Event::SessionLost),
            (Some(prev), Some(next)) => self.compare(prev, next),
        };
        if next.is_some() {
            self.seen_any = true;
        }
        self.last = next.cloned();
        event
    }

    fn compare(&self, prev: &PlayerSnapshot, next: &PlayerSnapshot) -> Option<Event> {
        if prev.track != next.track {
            return Some(Event::TrackChange);
        }
        if prev.status != next.status {
            return Some(Event::StatusChange);
        }
        if !prev.anchor_differs(next) {
            return None;
        }
        // Both playing: compare against the extrapolation. Otherwise (e.g. a
        // seek while paused) the raw position change is the jump.
        let delta = anchor_drift_ms(prev, next)
            .unwrap_or(next.reported_position_ms as i64 - prev.reported_position_ms as i64);
        if delta.abs() > self.seek_threshold_ms {
            Some(Event::Seek { jump_ms: delta })
        } else {
            Some(Event::AnchorUpdate { drift_ms: delta })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lyrics_core::player::{PlaybackStatus, TrackInfo};

    fn snap(title: &str, status: PlaybackStatus, pos: u64, at: i64) -> PlayerSnapshot {
        PlayerSnapshot {
            track: TrackInfo {
                title: title.into(),
                artist: "Artist".into(),
                album: "Album".into(),
                duration_ms: 240_000,
            },
            status,
            reported_position_ms: pos,
            position_updated_unix_ms: at,
        }
    }

    use PlaybackStatus::{Paused, Playing};

    #[test]
    fn session_lifecycle() {
        let mut m = Monitor::new(1_500);
        let a = snap("A", Playing, 0, 0);
        assert_eq!(m.observe(None), None);
        assert_eq!(m.observe(Some(&a)), Some(Event::Start));
        assert_eq!(m.observe(Some(&a)), None);
        assert_eq!(m.observe(None), Some(Event::SessionLost));
        assert_eq!(m.observe(None), None);
        assert_eq!(m.observe(Some(&a)), Some(Event::SessionFound));
    }

    #[test]
    fn track_and_status_changes() {
        let mut m = Monitor::new(1_500);
        m.observe(Some(&snap("A", Playing, 0, 0)));
        assert_eq!(
            m.observe(Some(&snap("A", Paused, 5_000, 5_000))),
            Some(Event::StatusChange)
        );
        assert_eq!(
            m.observe(Some(&snap("B", Paused, 0, 6_000))),
            Some(Event::TrackChange)
        );
    }

    #[test]
    fn small_anchor_moves_are_drift() {
        let mut m = Monitor::new(1_500);
        m.observe(Some(&snap("A", Playing, 10_000, 1_000)));
        assert_eq!(
            m.observe(Some(&snap("A", Playing, 15_120, 6_000))),
            Some(Event::AnchorUpdate { drift_ms: 120 })
        );
    }

    #[test]
    fn large_anchor_moves_are_seeks() {
        let mut m = Monitor::new(1_500);
        m.observe(Some(&snap("A", Playing, 10_000, 1_000)));
        assert_eq!(
            m.observe(Some(&snap("A", Playing, 90_000, 2_000))),
            Some(Event::Seek { jump_ms: 79_000 })
        );
    }

    #[test]
    fn seek_while_paused_uses_raw_jump() {
        let mut m = Monitor::new(1_500);
        m.observe(Some(&snap("A", Paused, 10_000, 1_000)));
        assert_eq!(
            m.observe(Some(&snap("A", Paused, 4_000, 9_000))),
            Some(Event::Seek { jump_ms: -6_000 })
        );
    }
}
