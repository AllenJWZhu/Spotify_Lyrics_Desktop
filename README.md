# Spotify Lyrics Desktop

Floating, always-on-top desktop lyrics for the Spotify desktop app on Windows, in the style of NetEase Cloud Music's desktop lyrics.

> **Status: M0 prototype.** Right now this is a command-line tool that reads what Spotify is playing and measures how accurately the playback position can be tracked. There is no overlay window yet.

## How it works

- **No Spotify login, no API key.** Spotify reports its playback state to Windows through the System Media Transport Controls (SMTC), the same mechanism behind the media flyout in the taskbar. This app reads that, so it works with a free Spotify account and needs no Client ID.
- **Position tracking.** SMTC reports position as an anchor ("position P at time T"), not a continuous stream. Between reports the app extrapolates from the wall clock and re-syncs on every new report, play/pause, or seek.
- **Lyrics** (coming in M1) are fetched at runtime from [LRCLIB](https://lrclib.net), an open database of time-synced lyrics. Lyrics are never bundled with or stored in this repository.

## Roadmap

- [x] **M0** Read Spotify via SMTC, interpolate position, log drift to CSV
- [ ] **M1** Fetch and parse synced lyrics, print the current line in the terminal
- [ ] **M2** Transparent always-on-top overlay window (Tauri)
- [ ] **M3** Lock / click-through mode, drag to move, tray icon, settings
- [ ] **M4** Translation line, per-track offset, extra lyrics providers, cache
- [ ] **M5** Packaged releases

## Running the M0 prototype

Requires Windows 10 or 11, the Spotify desktop app, and a [Rust toolchain](https://rustup.rs).

```powershell
cargo run --release -p lyrics-cli
```

Play something in Spotify. The terminal shows the estimated position, and every position report from Spotify is written to `drift.csv`:

| column | meaning |
| --- | --- |
| `event` | `start`, `track_change`, `status_change`, `anchor_update`, `seek`, `session_found`, `session_lost` |
| `reported_position_ms` / `anchor_unix_ms` | the anchor Spotify reported |
| `anchor_age_ms` | how old that anchor was when we read it |
| `delta_ms` | for `anchor_update`: reported position minus what the previous anchor predicted (drift). For `seek`: the jump distance |

Options: `--interval-ms` (poll interval, default 200), `--csv` (output path), `--seek-threshold-ms` (jumps above this count as seeks, default 1500).

A prebuilt `lyrics-cli.exe` from the latest CI run is available under the **Actions** tab (artifact `lyrics-cli-windows`).

## Project layout

```
crates/core   platform-independent logic: player model, position estimation, SMTC source (Windows only)
crates/cli    the M0 command-line prototype
```

## Development

```sh
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

The core logic is platform-independent and tests run on any OS. The SMTC source only compiles on Windows; CI builds and tests on both `windows-latest` and `ubuntu-latest`.

## License

[MIT](LICENSE). This project is not affiliated with or endorsed by Spotify.
