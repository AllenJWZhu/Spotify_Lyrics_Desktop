//! Windows System Media Transport Controls (SMTC) source.
//!
//! Spotify publishes its playback state to Windows like any other media app,
//! so we can read it through `GlobalSystemMediaTransportControlsSessionManager`
//! without the Spotify Web API, OAuth, or a Client ID.
//!
//! All calls here block (`join()` waits on the WinRT async operation), so
//! use this from a worker thread, never from a UI thread.

use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession as Session,
    GlobalSystemMediaTransportControlsSessionManager as SessionManager,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus as SmtcStatus,
};

use super::{
    PlaybackStatus, PlayerError, PlayerSnapshot, PlayerSource, TrackInfo, is_spotify_app_id,
    now_unix_ms, winrt_datetime_to_unix_ms, winrt_timespan_to_ms,
};

pub struct SmtcSource {
    manager: SessionManager,
}

impl SmtcSource {
    pub fn new() -> Result<Self, PlayerError> {
        let manager = SessionManager::RequestAsync()?.join()?;
        Ok(Self { manager })
    }

    /// Finds Spotify's session among all active media sessions.
    ///
    /// Deliberately not `GetCurrentSession()`: Windows' idea of the
    /// "current" session may be a browser tab or another player.
    fn find_spotify(&self) -> Result<Option<Session>, PlayerError> {
        let sessions = self.manager.GetSessions()?;
        for i in 0..sessions.Size()? {
            let session = sessions.GetAt(i)?;
            if is_spotify_app_id(&session.SourceAppUserModelId()?.to_string()) {
                return Ok(Some(session));
            }
        }
        Ok(None)
    }
}

impl PlayerSource for SmtcSource {
    fn snapshot(&self) -> Result<Option<PlayerSnapshot>, PlayerError> {
        let Some(session) = self.find_spotify()? else {
            return Ok(None);
        };

        // TODO(M1): cache metadata and refresh it on `MediaPropertiesChanged`
        // instead of fetching it on every poll.
        let props = session.TryGetMediaPropertiesAsync()?.join()?;
        let timeline = session.GetTimelineProperties()?;
        let status = session.GetPlaybackInfo()?.PlaybackStatus()?;

        let start_ms = winrt_timespan_to_ms(timeline.StartTime()?.Duration);
        let end_ms = winrt_timespan_to_ms(timeline.EndTime()?.Duration);

        // An unset LastUpdatedTime comes back as 0; treat the reading as fresh.
        let updated_ticks = timeline.LastUpdatedTime()?.UniversalTime;
        let position_updated_unix_ms = if updated_ticks > 0 {
            winrt_datetime_to_unix_ms(updated_ticks)
        } else {
            now_unix_ms()
        };

        Ok(Some(PlayerSnapshot {
            track: TrackInfo {
                title: props.Title()?.to_string(),
                artist: props.Artist()?.to_string(),
                album: props.AlbumTitle()?.to_string(),
                duration_ms: end_ms.saturating_sub(start_ms),
            },
            status: map_status(status),
            reported_position_ms: winrt_timespan_to_ms(timeline.Position()?.Duration),
            position_updated_unix_ms,
        }))
    }
}

fn map_status(status: SmtcStatus) -> PlaybackStatus {
    if status == SmtcStatus::Playing {
        PlaybackStatus::Playing
    } else if status == SmtcStatus::Paused {
        PlaybackStatus::Paused
    } else if status == SmtcStatus::Stopped
        || status == SmtcStatus::Closed
        || status == SmtcStatus::Opened
    {
        PlaybackStatus::Stopped
    } else if status == SmtcStatus::Changing {
        PlaybackStatus::Changing
    } else {
        PlaybackStatus::Unknown
    }
}
