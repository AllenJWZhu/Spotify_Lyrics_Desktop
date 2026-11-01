//! Lyrics model, parsing and providers.

pub mod cache;
pub mod fallback;
mod lrc;
pub mod lrclib;
pub mod matching;
pub mod netease;
pub mod qqmusic;
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
    pub source: String,
    pub matched_title: String,
    pub matched_artist: String,
    /// Length of the matched entry in milliseconds, if the provider said.
    pub matched_duration_ms: Option<u64>,
}

#[derive(Debug, thiserror::Error)]
pub enum LyricsError {
    /// From LRCLIB, the main source.
    #[error("request failed: {0}")]
    Http(#[from] ureq::Error),

    /// From another service, named so messages blame the right one.
    #[error("{service}: request failed: {error}")]
    Service {
        service: Service,
        error: ureq::Error,
    },

    #[error("{0}")]
    Other(String),
}

/// A lyrics service, for messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Service {
    Lrclib,
    NetEase,
    QqMusic,
}

impl Service {
    pub fn name(self) -> &'static str {
        match self {
            Service::Lrclib => "LRCLIB",
            Service::NetEase => "NetEase",
            Service::QqMusic => "QQ Music",
        }
    }

    pub fn host(self) -> &'static str {
        match self {
            Service::Lrclib => "lrclib.net",
            Service::NetEase => "music.163.com",
            Service::QqMusic => "y.qq.com",
        }
    }
}

impl std::fmt::Display for Service {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

/// Why a lookup failed, in terms that tell the user whose problem it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureKind {
    /// The service answered with a server error (5xx): it is down or
    /// overloaded. Nothing to fix locally.
    ServiceDown(u16),
    /// The service asked us to slow down (429).
    RateLimited,
    /// No answer in time.
    TimedOut,
    /// This computer couldn't reach the service: offline, DNS, refused.
    NoConnection,
    /// The system proxy couldn't be used.
    Proxy,
    /// The secure connection failed (certificates, TLS).
    Secure,
    /// Anything else, e.g. an unexpected response. Not worth retrying.
    Other,
}

impl FailureKind {
    /// Whether trying again later could help.
    pub fn is_retryable(self) -> bool {
        self != FailureKind::Other
    }

    /// One sentence for the overlay, about LRCLIB.
    pub fn message(self) -> String {
        self.message_for(Service::Lrclib)
    }

    /// One sentence for the overlay.
    pub fn message_for(self, service: Service) -> String {
        let (name, host) = (service.name(), service.host());
        match self {
            // The status code is in the log; on screen it's just noise.
            FailureKind::ServiceDown(_) => format!("{name} is down right now"),
            FailureKind::RateLimited => format!("{name} is limiting requests right now"),
            FailureKind::TimedOut => format!("{name} isn't responding"),
            FailureKind::NoConnection => {
                format!("Can't connect to {host}. Check your internet connection")
            }
            FailureKind::Proxy => "Can't connect through your system proxy".into(),
            FailureKind::Secure => format!("Couldn't make a secure connection to {host}"),
            FailureKind::Other => format!("Couldn't get lyrics from {name}"),
        }
    }
}

impl LyricsError {
    /// The service the failure came from.
    pub fn service(&self) -> Service {
        match self {
            LyricsError::Service { service, .. } => *service,
            _ => Service::Lrclib,
        }
    }

    /// One sentence for the overlay, naming the right service.
    pub fn message(&self) -> String {
        self.kind().message_for(self.service())
    }

    pub fn kind(&self) -> FailureKind {
        use std::io::ErrorKind;
        let e = match self {
            LyricsError::Http(e) | LyricsError::Service { error: e, .. } => e,
            LyricsError::Other(_) => return FailureKind::Other,
        };
        match e {
            ureq::Error::StatusCode(429) => FailureKind::RateLimited,
            ureq::Error::StatusCode(s) if *s >= 500 => FailureKind::ServiceDown(*s),
            ureq::Error::Timeout(_) => FailureKind::TimedOut,
            ureq::Error::Io(io) if io.kind() == ErrorKind::TimedOut => FailureKind::TimedOut,
            ureq::Error::HostNotFound | ureq::Error::ConnectionFailed | ureq::Error::Io(_) => {
                FailureKind::NoConnection
            }
            ureq::Error::ConnectProxyFailed(_) | ureq::Error::InvalidProxyUrl => FailureKind::Proxy,
            ureq::Error::Tls(_) | ureq::Error::Rustls(_) => FailureKind::Secure,
            _ => FailureKind::Other,
        }
    }
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
    fn failures_are_classified_by_whose_problem_it_is() {
        use std::io;
        let http = |e: ureq::Error| LyricsError::Http(e).kind();
        assert_eq!(
            http(ureq::Error::StatusCode(502)),
            FailureKind::ServiceDown(502)
        );
        assert_eq!(
            http(ureq::Error::StatusCode(503)),
            FailureKind::ServiceDown(503)
        );
        assert_eq!(http(ureq::Error::StatusCode(429)), FailureKind::RateLimited);
        assert_eq!(http(ureq::Error::StatusCode(400)), FailureKind::Other);
        assert_eq!(http(ureq::Error::HostNotFound), FailureKind::NoConnection);
        assert_eq!(
            http(ureq::Error::ConnectionFailed),
            FailureKind::NoConnection
        );
        assert_eq!(
            http(ureq::Error::Io(io::Error::from(
                io::ErrorKind::ConnectionRefused
            ))),
            FailureKind::NoConnection
        );
        assert_eq!(
            http(ureq::Error::Io(io::Error::from(io::ErrorKind::TimedOut))),
            FailureKind::TimedOut
        );
        assert_eq!(
            http(ureq::Error::ConnectProxyFailed("x".into())),
            FailureKind::Proxy
        );
        assert_eq!(LyricsError::Other("x".into()).kind(), FailureKind::Other);

        assert!(FailureKind::ServiceDown(502).is_retryable());
        assert!(!FailureKind::Other.is_retryable());
        assert_eq!(
            FailureKind::ServiceDown(502).message(),
            "LRCLIB is down right now"
        );
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
