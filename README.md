# Spotify Lyrics Desktop

Floating, always-on-top desktop lyrics for the Spotify desktop app on Windows, in the style of NetEase Cloud Music's desktop lyrics.

> **Status: M3.** A transparent, always-on-top window shows the current line with a karaoke fill and the next line (or translation) below it. It can be locked so clicks pass through, and is controlled from a hover toolbar, the tray icon or global shortcuts.

## How it works

- **No Spotify login, no API key.** Spotify reports its playback state to Windows through the System Media Transport Controls (SMTC), the same mechanism behind the media flyout in the taskbar. This app reads that, so it works with a free Spotify account and needs no Client ID.
- **Position tracking.** SMTC reports position as an anchor ("position P at time T") about every 4.5 s. Between reports the app extrapolates from the wall clock; measured error between reports is 0–1 ms, and every report, play/pause or seek re-syncs it.
- **Track changes.** Spotify updates a track's duration 200–800 ms before its title and artist. The app waits until all three have stopped changing for 1 s before looking up lyrics, so it never searches for "old title + new duration".
- **Overlay.** A [Tauri](https://tauri.app) window (WebView2 on Windows). The backend sends the lyrics once per track plus a position anchor on line changes, play/pause and seeks; the page animates the fill every frame from that anchor, so it stays smooth without a stream of updates.
- **Lyrics** are fetched at runtime from [LRCLIB](https://lrclib.net), an open database of time-synced lyrics: exact match first (with and without the album, then with cleaned-up title and main artist), then a search filtered to ±2 s of the track length. Lyrics are never bundled with or stored in this repository.

## Roadmap

- [x] **M0** Read Spotify via SMTC, interpolate position, log drift to CSV
- [x] **M1** Fetch and parse synced lyrics, print the current line in the terminal
- [x] **M2** Transparent always-on-top overlay window (Tauri)
- [x] **M3** Lock / click-through mode, drag to move, tray icon, settings
- [ ] **M4** Translation line, per-track offset, extra lyrics providers, persistent cache, word-level timing
- [ ] **M5** Packaged releases

## Running the overlay

Requires Windows 10 or 11 and the Spotify desktop app. Either download `lyrics-overlay.exe` from the latest CI run (**Actions** tab, artifact `lyrics-overlay-windows`) or build it with a [Rust toolchain](https://rustup.rs):

```powershell
cargo run --release -p lyrics-overlay
```

The window appears above the taskbar. Drag it anywhere to move it.

| | |
| --- | --- |
| **Hover** | toolbar: previous / play-pause / next, text size A− A+, colour, lyric timing − 0.0s + (click the value to reset), lock, quit |
| **Lock** | clicks pass through to whatever is underneath. Hover the lyrics and an unlock button appears at the top |
| **Tray icon** | show/hide, lock/unlock, quit |
| `Ctrl+Alt+L` | lock / unlock |
| `Ctrl+Alt+D` | show / hide |

Position, size, colour, timing offset and lock state are remembered in `%APPDATA%\io.github.allenjwzhu.lyrics-overlay\settings.json`.

To try it without Spotify (or on macOS/Linux, where this is the only mode), run the built-in demo track:

```powershell
cargo run -p lyrics-overlay -- --demo
```

## Command-line prototype

`lyrics-cli` follows Spotify in the terminal instead (artifact `lyrics-cli-windows`):

```powershell
cargo run --release -p lyrics-cli
```

Play something in Spotify:

```
♪ Some Song — Some Artist
  synced lyrics from LRCLIB (42 lines, matched “Some Song” by Some Artist)
  [00:12.3] first line
  [00:15.8] second line
▶ 00:16.4 / 03:21.0  lyrics: synced  Some Song — Some Artist
```

| option | default | |
| --- | --- | --- |
| `--offset-ms` | `0` | shift lyric timing; positive shows lines earlier, e.g. `--offset-ms=-300` |
| `--interval-ms` | `100` | poll interval |
| `--settle-ms` | `1000` | how long metadata must be stable before lyrics are fetched |

### Measuring sync drift

`lyrics-cli drift` writes every position report from Spotify to `drift.csv` (path printed at startup):

| column | meaning |
| --- | --- |
| `event` | `start`, `track_change`, `status_change`, `anchor_update`, `seek`, `session_found`, `session_lost` |
| `reported_position_ms` / `anchor_unix_ms` | the anchor Spotify reported |
| `anchor_age_ms` | how old that anchor was when we read it |
| `delta_ms` | for `anchor_update`: reported position minus what the previous anchor predicted (drift). For `seek`: the jump distance |

Options: `--interval-ms` (default 200), `--csv` (output path), `--seek-threshold-ms` (jumps above this count as seeks, default 1500).

## Project layout

```
crates/core
  player/    snapshot model, position estimation, track settling, SMTC source (Windows only)
  lyrics/    LRC parser, title/artist cleanup, LRCLIB client
  follow.rs  Follower: current track, lyrics lookup with retries, current line
crates/cli   command-line front end
crates/app   overlay window (Tauri): settings, controller (lock, tray, shortcuts), worker; ui/ is plain HTML/CSS/JS, no build step
```

## Development

```sh
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
node --test "crates/app/tests-js/*.test.mjs"   # overlay view logic
```

On Linux, building the overlay needs WebKitGTK (`libwebkit2gtk-4.1-dev` and friends, see the [Tauri prerequisites](https://tauri.app/start/prerequisites/)).

Everything except the SMTC source is platform-independent, and tests need no network (the HTTP client is tested against a local mock server). CI builds and tests on `windows-latest` and `ubuntu-latest`.

Live checks against the real LRCLIB API are opt-in:

```sh
cargo test -p lyrics-core --test lrclib_live -- --ignored --nocapture
```

## License

[MIT](LICENSE). Lyrics are provided by [LRCLIB](https://lrclib.net) at runtime. This project is not affiliated with or endorsed by Spotify.
