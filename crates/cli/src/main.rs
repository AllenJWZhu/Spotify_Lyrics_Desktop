//! Command-line prototype: follows the Spotify desktop app through Windows
//! SMTC and prints lyrics line by line in time with playback.
//!
//! `lyrics-cli drift` keeps the M0 tool that logs position reports to CSV.

mod display;
mod drift;
mod monitor;
mod watch;

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(version, about, args_conflicts_with_subcommands = true)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    #[command(flatten)]
    watch: WatchArgs,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Log Spotify's position reports to CSV to measure sync drift.
    Drift(DriftArgs),
}

/// Options for the default mode (following playback with lyrics).
#[derive(Args, Debug)]
pub struct WatchArgs {
    /// Shift lyric timing in milliseconds. Positive shows lines earlier.
    #[arg(long, default_value_t = 0, allow_negative_numbers = true)]
    pub offset_ms: i64,

    /// Poll interval in milliseconds.
    #[arg(long, default_value_t = 100)]
    pub interval_ms: u64,

    /// How long track metadata must stay unchanged before lyrics are fetched.
    #[arg(long, default_value_t = 1_000)]
    pub settle_ms: i64,

    /// Don't fall back to NetEase when LRCLIB has nothing synced.
    #[arg(long)]
    pub no_netease: bool,
}

#[derive(Args, Debug)]
pub struct DriftArgs {
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
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    use lyrics_core::lyrics::fallback::Racing;
    use lyrics_core::player::smtc::SmtcSource;

    let cli = Cli::parse();
    let source = SmtcSource::new()?;
    match cli.command {
        Some(Command::Drift(args)) => drift::run(&source, &args),
        None => {
            let backup = Arc::new(AtomicBool::new(!cli.watch.no_netease));
            let provider = Arc::new(Racing::lrclib_and_netease(backup));
            watch::run(&source, provider, &cli.watch)
        }
    }
}

#[cfg(not(windows))]
fn main() -> anyhow::Result<()> {
    // Still parse so `--help` and `--version` work everywhere.
    let _cli = Cli::parse();
    anyhow::bail!("lyrics-cli reads Spotify through Windows SMTC and only runs on Windows for now")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_mode_takes_watch_options() {
        let cli = Cli::try_parse_from(["lyrics-cli", "--offset-ms", "-300"]).unwrap();
        assert!(cli.command.is_none());
        assert_eq!(cli.watch.offset_ms, -300);
        assert_eq!(cli.watch.interval_ms, 100);
    }

    #[test]
    fn drift_subcommand_parses() {
        let cli = Cli::try_parse_from(["lyrics-cli", "drift", "--csv", "out.csv"]).unwrap();
        match cli.command {
            Some(Command::Drift(args)) => assert_eq!(args.csv, PathBuf::from("out.csv")),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn verifies_clap_definition() {
        use clap::CommandFactory;
        Cli::command().debug_assert();
    }
}
