//! M0 prototype: watch Spotify's media session, print the interpolated
//! playback position, and log every position report to CSV so we can
//! measure how well local interpolation tracks real playback.

mod display;
mod monitor;
mod run;

use std::path::PathBuf;

use clap::Parser;

#[derive(Parser, Debug)]
#[command(version, about)]
pub struct Args {
    /// Poll interval in milliseconds.
    #[arg(long, default_value_t = 200)]
    pub interval_ms: u64,

    /// Where to write the drift log.
    #[arg(long, default_value = "drift.csv")]
    pub csv: PathBuf,

    /// Anchor jumps larger than this (ms) are logged as seeks, not drift.
    #[arg(long, default_value_t = 1_500)]
    pub seek_threshold_ms: i64,
}

#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let source = lyrics_core::player::smtc::SmtcSource::new()?;
    run::run(&source, &args)
}

#[cfg(not(windows))]
fn main() -> anyhow::Result<()> {
    // Still parse so `--help` and `--version` work everywhere.
    let _args = Args::parse();
    anyhow::bail!("lyrics-cli reads Spotify through Windows SMTC and only runs on Windows for now")
}
