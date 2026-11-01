//! Background thread: polls the player, drives the [`Follower`], fetches
//! lyrics, and pushes [`OverlayState`] / [`PlaybackView`] to the page.

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use lyrics_core::follow::{FetchError, FollowEvent, Follower};
use lyrics_core::lyrics::cache::LyricsCache;
use lyrics_core::lyrics::{FoundLyrics, LyricsProvider};
use lyrics_core::player::{PlaybackStatus, PlayerAction, PlayerSource, TrackInfo, now_unix_ms};

use crate::controller::Controller;
use crate::demo::{DemoProvider, DemoSource};
use crate::presence::PlayerSeen;
use crate::view::{OverlayState, PLAYBACK_EVENT, PlaybackThrottle, PlaybackView, STATE_EVENT};

const POLL_MS: u64 = 100;
const SETTLE_MS: i64 = 1_000;

/// Requests from the rest of the app to the worker thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerCommand {
    SetOffset(i64),
    Control(PlayerAction),
    /// Drop the current track's lyrics (and cache entry) and look them up again.
    Refresh,
}

/// Where lyrics come from and, if caching, the cache behind them.
struct Parts {
    source: Box<dyn PlayerSource>,
    provider: Arc<dyn LyricsProvider>,
    cache: Option<Arc<LyricsCache>>,
}

/// Everything the page needs on (re)load, before events start arriving.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Initial {
    pub state: OverlayState,
    pub playback: Option<PlaybackView>,
}

/// Latest values sent to the page, for the `get_state` command.
#[derive(Default)]
pub struct Shared(Mutex<Initial>);

impl Shared {
    pub fn current(&self) -> Initial {
        self.lock().clone()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Initial> {
        // A panic while holding the lock leaves plain data behind; keep going.
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// `cache`: where to keep fetched lyrics; `None` (or a cache that fails to
/// open) means always fetch. `backup`: whether to ask the backup sources when LRCLIB
/// has nothing synced. Neither is used in demo mode.
pub fn spawn(
    app: AppHandle,
    demo: bool,
    cache: Option<std::path::PathBuf>,
    backup: Arc<AtomicBool>,
    offset_ms: i64,
    commands: mpsc::Receiver<WorkerCommand>,
) -> std::io::Result<()> {
    thread::Builder::new()
        .name("lyrics-worker".into())
        .spawn(move || run(app, demo, cache, backup, offset_ms, commands))?;
    Ok(())
}

type FetchResult = (TrackInfo, Result<Option<FoundLyrics>, FetchError>);

fn run(
    app: AppHandle,
    demo: bool,
    cache_path: Option<std::path::PathBuf>,
    backup: Arc<AtomicBool>,
    offset_ms: i64,
    commands: mpsc::Receiver<WorkerCommand>,
) {
    let Parts {
        source,
        provider,
        cache,
    } = if demo {
        demo_parts()
    } else {
        real_parts(&app, cache_path.as_deref(), backup)
    };

    let mut follower = Follower::new(SETTLE_MS, offset_ms);
    let mut throttle = PlaybackThrottle::default();
    let (tx, rx) = mpsc::channel::<FetchResult>();

    loop {
        let mut events = Vec::new();
        while let Ok(command) = commands.try_recv() {
            match command {
                WorkerCommand::SetOffset(ms) => {
                    follower.set_offset_ms(ms);
                    // The position the page extrapolates from just jumped.
                    throttle.reset();
                }
                WorkerCommand::Control(action) => {
                    if let Err(e) = source.control(action) {
                        log::warn!("player control failed: {e}");
                    }
                }
                WorkerCommand::Refresh => {
                    if let (Some(cache), Some(track)) = (&cache, follower.track()) {
                        if let Err(e) = cache.remove(track) {
                            log::warn!("{e}");
                        }
                    }
                    events.extend(follower.refresh());
                    throttle.reset();
                }
            }
        }

        let now = now_unix_ms();
        // A failed read (session vanishing while Spotify starts or quits)
        // looks the same as "not running" to the follower.
        let snapshot = source.snapshot().unwrap_or(None);
        app.state::<Controller>().observe_player(match &snapshot {
            None => PlayerSeen::Absent,
            Some(s) if s.status == PlaybackStatus::Playing => PlayerSeen::Playing,
            Some(_) => PlayerSeen::Idle,
        });

        events.extend(follower.tick(snapshot.as_ref(), now));
        while let Ok((track, result)) = rx.try_recv() {
            events.extend(follower.on_fetched(track, result, now));
        }

        let mut state_changed = false;
        for event in &events {
            match event {
                FollowEvent::LineChanged(_) => {}
                FollowEvent::FetchRequested(track) => {
                    spawn_fetch(&provider, track.clone(), tx.clone());
                    state_changed = true;
                }
                FollowEvent::TrackChanged(_) | FollowEvent::Cleared => {
                    throttle.reset();
                    state_changed = true;
                }
                FollowEvent::LyricsReady(_) | FollowEvent::LookupFailed { .. } => {
                    state_changed = true;
                }
            }
        }

        if state_changed {
            let state = OverlayState::from_follower(&follower);
            app.state::<Shared>().lock().state = state.clone();
            let _ = app.emit(STATE_EVENT, state);
        }

        let playback = match (&snapshot, follower.track()) {
            (Some(s), Some(_)) => Some(PlaybackView::from_follower(&follower, s, now)),
            _ => None,
        };
        if let Some(view) = playback.and_then(|p| throttle.update(p)) {
            app.state::<Shared>().lock().playback = Some(view.clone());
            let _ = app.emit(PLAYBACK_EVENT, view);
        }

        thread::sleep(Duration::from_millis(POLL_MS));
    }
}

fn spawn_fetch(
    provider: &Arc<dyn LyricsProvider>,
    track: TrackInfo,
    tx: mpsc::Sender<FetchResult>,
) {
    let provider = Arc::clone(provider);
    thread::spawn(move || {
        let result = provider.fetch(&track);
        match &result {
            Ok(Some(found)) => log::info!(
                "lyrics for {:?} by {:?}: {} from {}",
                track.title,
                track.artist,
                kind_name(&found.lyrics),
                found.source
            ),
            Ok(None) => log::info!("no lyrics for {:?} by {:?}", track.title, track.artist),
            // The full error (status, proxy, TLS…) goes to the log; the page
            // gets a sentence about whose problem it is.
            Err(e) => log::warn!(
                "lyrics lookup for {:?} by {:?} failed: {e}",
                track.title,
                track.artist
            ),
        }
        let result = result.map_err(|e| FetchError::from(&e));
        // The receiver lives as long as the worker, i.e. the app.
        let _ = tx.send((track, result));
    });
}

fn kind_name(lyrics: &lyrics_core::lyrics::Lyrics) -> &'static str {
    match lyrics {
        lyrics_core::lyrics::Lyrics::Synced(_) => "synced",
        lyrics_core::lyrics::Lyrics::Plain(_) => "plain",
        lyrics_core::lyrics::Lyrics::Instrumental => "instrumental",
    }
}

fn demo_parts() -> Parts {
    Parts {
        source: Box::new(DemoSource::new()),
        provider: Arc::new(DemoProvider),
        cache: None,
    }
}

/// LRCLIB and the backup sources, behind the on-disk cache when it opens.
#[cfg_attr(not(windows), allow(dead_code))]
fn lyrics_provider(
    cache_path: Option<&std::path::Path>,
    backup: Arc<AtomicBool>,
) -> (Arc<dyn LyricsProvider>, Option<Arc<LyricsCache>>) {
    use lyrics_core::lyrics::cache::CachedProvider;
    use lyrics_core::lyrics::fallback::Racing;

    let sources = Racing::standard(backup);
    match cache_path.map(LyricsCache::open) {
        Some(Ok(cache)) => {
            let cache = Arc::new(cache);
            let provider = CachedProvider::new(sources, Arc::clone(&cache));
            (Arc::new(provider), Some(cache))
        }
        Some(Err(e)) => {
            log::warn!("{e}; lyrics won't be cached");
            (Arc::new(sources), None)
        }
        None => (Arc::new(sources), None),
    }
}

#[cfg(windows)]
fn real_parts(
    app: &AppHandle,
    cache_path: Option<&std::path::Path>,
    backup: Arc<AtomicBool>,
) -> Parts {
    use lyrics_core::player::smtc::{SmtcSource, init_worker_thread};

    init_worker_thread();
    let source = loop {
        match SmtcSource::new() {
            Ok(source) => break source,
            Err(e) => {
                let state = OverlayState {
                    message: Some(format!("Can't reach Windows media controls: {e}")),
                    ..OverlayState::default()
                };
                app.state::<Shared>().lock().state = state.clone();
                let _ = app.emit(STATE_EVENT, state);
                thread::sleep(Duration::from_secs(2));
            }
        }
    };
    let (provider, cache) = lyrics_provider(cache_path, backup);
    Parts {
        source: Box::new(source),
        provider,
        cache,
    }
}

#[cfg(not(windows))]
fn real_parts(
    _app: &AppHandle,
    _cache_path: Option<&std::path::Path>,
    _backup: Arc<AtomicBool>,
) -> Parts {
    // `main` always picks demo mode off Windows.
    demo_parts()
}
