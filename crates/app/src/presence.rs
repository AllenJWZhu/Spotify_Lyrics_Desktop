//! When the overlay is on screen.
//!
//! The window stays out of the way until Spotify actually plays something,
//! like NetEase's desktop lyrics that only exist while its player runs. Once
//! Spotify has played, the lyrics stay up through pauses and go away when
//! Spotify quits. The user's own show/hide choice (`Ctrl+Alt+D`, tray) still
//! applies on top.
//!
//! While Spotify isn't playing, the window can "peek" for a few seconds (on a
//! manual launch, or when the user asks to show it) so starting the app or
//! pressing the shortcut visibly does something: it shows "Play something
//! in Spotify" and then gets out of the way again.

use std::time::{Duration, Instant};

/// How long the window peeks after a manual launch or a show request.
pub const PEEK: Duration = Duration::from_secs(6);
/// While peeking, the window stays up this long after the pointer leaves it,
/// so it doesn't vanish while being dragged or read.
pub const HOVER_LINGER: Duration = Duration::from_secs(2);
/// Spotify's media session can blink out while it starts, quits or changes
/// device; only treat it as gone after this long.
pub const GONE_GRACE: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, Default)]
pub struct Presence {
    /// Spotify has played since its media session appeared.
    active: bool,
    /// When the session was first seen missing, while `active`.
    missing_since: Option<Instant>,
    peek_until: Option<Instant>,
}

/// What the player looked like on one poll.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayerSeen {
    /// No Spotify media session (not running, or it hasn't registered yet).
    Absent,
    /// A session that isn't playing: paused, stopped, nothing loaded.
    Idle,
    Playing,
}

impl Presence {
    /// Updates from one player poll. Returns `Some(active)` when that changed.
    pub fn observe(&mut self, seen: PlayerSeen, now: Instant) -> Option<bool> {
        let before = self.active;
        match seen {
            PlayerSeen::Playing => {
                self.active = true;
                self.missing_since = None;
            }
            PlayerSeen::Idle => self.missing_since = None,
            PlayerSeen::Absent if self.active => {
                let since = *self.missing_since.get_or_insert(now);
                if now.duration_since(since) >= GONE_GRACE {
                    self.active = false;
                    self.missing_since = None;
                }
            }
            PlayerSeen::Absent => {}
        }
        (self.active != before).then_some(self.active)
    }

    /// Shows the window for `PEEK` even though Spotify isn't playing.
    pub fn peek(&mut self, now: Instant) {
        self.peek_until = Some(now + PEEK);
    }

    pub fn peeking(&self, now: Instant) -> bool {
        self.peek_until.is_some_and(|until| now < until)
    }

    /// The pointer is on the window: keep a peek going a little longer.
    pub fn hovered(&mut self, now: Instant) {
        if let Some(until) = &mut self.peek_until {
            if now < *until {
                *until = (*until).max(now + HOVER_LINGER);
            }
        }
    }

    pub fn end_peek(&mut self) {
        self.peek_until = None;
    }

    /// Whether the window should be on screen. `visible` is the user's
    /// show/hide setting.
    pub fn shown(&self, visible: bool, now: Instant) -> bool {
        visible && (self.active || self.peeking(now))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(t0: Instant, ms: u64) -> Instant {
        t0 + Duration::from_millis(ms)
    }

    #[test]
    fn hidden_until_spotify_plays() {
        let t0 = Instant::now();
        let mut p = Presence::default();
        assert!(!p.shown(true, t0));
        // Spotify open with a track loaded but paused: still hidden.
        assert_eq!(p.observe(PlayerSeen::Idle, t0), None);
        assert!(!p.shown(true, t0));
        assert_eq!(p.observe(PlayerSeen::Playing, at(t0, 100)), Some(true));
        assert!(p.shown(true, at(t0, 100)));
        // The user's hide setting still wins.
        assert!(!p.shown(false, at(t0, 100)));
    }

    #[test]
    fn stays_up_through_pauses() {
        let t0 = Instant::now();
        let mut p = Presence::default();
        p.observe(PlayerSeen::Playing, t0);
        assert_eq!(p.observe(PlayerSeen::Idle, at(t0, 60_000)), None);
        assert!(p.shown(true, at(t0, 600_000)));
    }

    #[test]
    fn hides_once_spotify_has_been_gone_a_while() {
        let t0 = Instant::now();
        let mut p = Presence::default();
        p.observe(PlayerSeen::Playing, t0);
        assert_eq!(p.observe(PlayerSeen::Absent, at(t0, 1_000)), None);
        assert_eq!(p.observe(PlayerSeen::Absent, at(t0, 3_900)), None);
        assert!(p.shown(true, at(t0, 3_900)));
        assert_eq!(p.observe(PlayerSeen::Absent, at(t0, 4_000)), Some(false));
        assert!(!p.shown(true, at(t0, 4_000)));
        // Reopened Spotify, track restored but paused: stays hidden.
        p.observe(PlayerSeen::Idle, at(t0, 10_000));
        assert!(!p.shown(true, at(t0, 10_000)));
    }

    #[test]
    fn a_session_blip_does_not_hide_it() {
        let t0 = Instant::now();
        let mut p = Presence::default();
        p.observe(PlayerSeen::Playing, t0);
        p.observe(PlayerSeen::Absent, at(t0, 1_000));
        p.observe(PlayerSeen::Idle, at(t0, 2_000));
        // The earlier absence doesn't count toward a later one.
        p.observe(PlayerSeen::Absent, at(t0, 3_000));
        assert_eq!(p.observe(PlayerSeen::Absent, at(t0, 5_500)), None);
        assert!(p.shown(true, at(t0, 5_500)));
    }

    #[test]
    fn peeks_for_a_few_seconds() {
        let t0 = Instant::now();
        let mut p = Presence::default();
        p.peek(t0);
        assert!(p.shown(true, at(t0, 5_900)));
        assert!(!p.shown(true, at(t0, 6_000)));
        assert!(!p.shown(false, t0));
    }

    #[test]
    fn hovering_keeps_a_peek_up() {
        let t0 = Instant::now();
        let mut p = Presence::default();
        p.peek(t0);
        p.hovered(at(t0, 5_500));
        assert!(p.shown(true, at(t0, 7_400)));
        assert!(!p.shown(true, at(t0, 7_500)));
        // Hovering after a peek has ended doesn't bring it back.
        p.hovered(at(t0, 9_000));
        assert!(!p.shown(true, at(t0, 9_000)));
    }

    #[test]
    fn ending_a_peek_hides_at_once() {
        let t0 = Instant::now();
        let mut p = Presence::default();
        p.peek(t0);
        p.end_peek();
        assert!(!p.shown(true, t0));
    }
}
