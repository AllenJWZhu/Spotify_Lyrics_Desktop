//! `lyrics-cli drift`: the M0 tool. Logs every position report from Spotify
//! to CSV so we can measure how well local interpolation tracks playback.

// On non-Windows targets `main` never calls into this module.
#![cfg_attr(not(windows), allow(dead_code))]

use std::fs::File;
use std::io::{self, Write};
use std::thread;
use std::time::Duration;

use anyhow::Context;
use lyrics_core::player::{PlayerSnapshot, PlayerSource, now_unix_ms};

use crate::DriftArgs;
use crate::display::{Screen, status_line};
use crate::monitor::{Event, Monitor};

pub fn run(source: &impl PlayerSource, args: &DriftArgs) -> anyhow::Result<()> {
    let file =
        File::create(&args.csv).with_context(|| format!("cannot create {}", args.csv.display()))?;
    let mut log = DriftLog::new(file)?;
    let mut monitor = Monitor::new(args.seek_threshold_ms);
    let mut screen = Screen::new(io::stdout());

    // Print the absolute path: a relative one depends on where the CLI was
    // launched from, which is easy to lose track of.
    let csv_path = std::path::absolute(&args.csv).unwrap_or_else(|_| args.csv.clone());
    screen.line(&format!(
        "Logging position reports to {} (Ctrl+C to stop)",
        csv_path.display()
    ))?;

    loop {
        let now = now_unix_ms();
        let snapshot = source.snapshot().unwrap_or_else(|e| {
            // Sessions can vanish between calls while Spotify starts or
            // quits; report and keep polling.
            let _ = screen.line(&format!("SMTC error: {e}"));
            None
        });

        if let Some(event) = monitor.observe(snapshot.as_ref()) {
            log.record(now, event, snapshot.as_ref())?;
            if matches!(
                event,
                Event::Start | Event::SessionFound | Event::TrackChange
            ) {
                // Keep the previous track's final status line on screen.
                screen.keep_status()?;
            }
        }

        screen.status(&status_line(snapshot.as_ref(), now))?;
        thread::sleep(Duration::from_millis(args.interval_ms));
    }
}

/// CSV log of every event, flushed per row so Ctrl+C loses nothing.
struct DriftLog<W: Write> {
    writer: csv::Writer<W>,
}

impl<W: Write> DriftLog<W> {
    const HEADER: [&'static str; 10] = [
        "wall_unix_ms",
        "event",
        "status",
        "title",
        "artist",
        "duration_ms",
        "reported_position_ms",
        "anchor_unix_ms",
        "anchor_age_ms",
        "delta_ms",
    ];

    fn new(inner: W) -> anyhow::Result<Self> {
        let mut writer = csv::Writer::from_writer(inner);
        writer.write_record(Self::HEADER)?;
        writer.flush()?;
        Ok(Self { writer })
    }

    fn record(
        &mut self,
        wall_unix_ms: i64,
        event: Event,
        snapshot: Option<&PlayerSnapshot>,
    ) -> anyhow::Result<()> {
        let delta = event.delta_ms().map(|d| d.to_string()).unwrap_or_default();
        let row: [String; 10] = match snapshot {
            Some(s) => [
                wall_unix_ms.to_string(),
                event.name().to_string(),
                format!("{:?}", s.status),
                s.track.title.clone(),
                s.track.artist.clone(),
                s.track.duration_ms.to_string(),
                s.reported_position_ms.to_string(),
                s.position_updated_unix_ms.to_string(),
                (wall_unix_ms - s.position_updated_unix_ms).to_string(),
                delta,
            ],
            None => [
                wall_unix_ms.to_string(),
                event.name().to_string(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                delta,
            ],
        };
        self.writer.write_record(&row)?;
        self.writer.flush()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lyrics_core::player::{PlaybackStatus, TrackInfo};

    #[test]
    fn drift_log_writes_quoted_rows() {
        let mut buf = Vec::new();
        {
            let mut log = DriftLog::new(&mut buf).unwrap();
            let s = PlayerSnapshot {
                track: TrackInfo {
                    title: "Title, with comma".into(),
                    artist: "Artist".into(),
                    album: "Album".into(),
                    duration_ms: 200_000,
                },
                status: PlaybackStatus::Playing,
                reported_position_ms: 5_000,
                position_updated_unix_ms: 1_000,
            };
            log.record(1_250, Event::AnchorUpdate { drift_ms: -40 }, Some(&s))
                .unwrap();
            log.record(2_000, Event::SessionLost, None).unwrap();
        }
        let text = String::from_utf8(buf).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 3);
        assert!(lines[0].starts_with("wall_unix_ms,event,"));
        assert_eq!(
            lines[1],
            "1250,anchor_update,Playing,\"Title, with comma\",Artist,200000,5000,1000,250,-40"
        );
        assert_eq!(lines[2], "2000,session_lost,,,,,,,,");
    }
}
