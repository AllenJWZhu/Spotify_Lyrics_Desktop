# Spotify Lyrics Desktop

Floating, always-on-top desktop lyrics for the Spotify desktop app on Windows, in the style of NetEase Cloud Music's desktop lyrics.

![The overlay showing a Chinese line half sung, with its English translation underneath](docs/screenshot.png)

A transparent, always-on-top window shows the current line with a karaoke fill and the next line (or translation) below it. Lock it so clicks pass through to whatever is underneath; hover it for playback controls, text size, colour and timing.

![Hover toolbar: previous, play, next, text size, colour, timing offset, lock, quit](docs/screenshot-toolbar.png)

## Install

Requires Windows 10 or 11 and the [Spotify desktop app](https://www.spotify.com/download/) (the Microsoft Store version is recognised too). A free Spotify account is fine.

1. Download **`LyricsOverlay-<version>-setup.exe`** from the [latest release](https://github.com/AllenJWZhu/Spotify_Lyrics_Desktop/releases/latest).
2. Run it. The installer isn't code-signed yet, so Windows SmartScreen shows **"Windows protected your PC"**: click **More info → Run anyway**. It installs for your user only and needs no administrator rights.
3. It shows itself above the taskbar for a few seconds, then waits out of sight. Play something in Spotify and the lyrics appear.

Prefer not to install? **`LyricsOverlay-<version>-portable.zip`** contains a single `LyricsOverlay.exe`; unzip and run it. It needs the Microsoft Edge WebView2 runtime, which Windows 11 and up-to-date Windows 10 already have (the installer adds it if missing).

Each release lists SHA-256 checksums in `SHA256SUMS.txt`. To check a download in PowerShell: `Get-FileHash .\LyricsOverlay-0.1.0-setup.exe`.

**Updates** come to you: the overlay checks for a new version when it starts and every few hours, and asks before installing it (one click, then it restarts by itself). **Check for updates** in the tray checks right away. Updates are signed, so the overlay only installs ones built by this project. (0.1.0 predates this; install 0.1.1 or later by hand once.) The portable version is offered updates too; accepting one installs the regular version.

To uninstall, use **Settings → Apps → Installed apps → Lyrics Overlay**. It also removes the start-with-Windows entry; tick **Delete the application data** to remove settings and cached lyrics too. For the portable version, turn off **Start with Windows** in the tray before deleting the exe.

## How it works

- **No Spotify login, no API key.** Spotify reports its playback state to Windows through the System Media Transport Controls (SMTC), the same mechanism behind the media flyout in the taskbar. This app reads that, so it works with a free Spotify account and needs no Client ID.
- **Position tracking.** SMTC reports position as an anchor ("position P at time T") about every 4.5 s. Between reports the app extrapolates from the wall clock; measured error between reports is 0–1 ms, and every report, play/pause or seek re-syncs it.
- **Track changes.** Spotify updates a track's duration 200–800 ms before its title and artist. The app waits until all three have stopped changing for 1 s before looking up lyrics, so it never searches for "old title + new duration".
- **Overlay.** A [Tauri](https://tauri.app) window (WebView2 on Windows). The backend sends the lyrics once per track plus a position anchor on line changes, play/pause and seeks; the page animates the fill every frame from that anchor, so it stays smooth without a stream of updates.
- **Lyrics** are fetched at runtime from [LRCLIB](https://lrclib.net), an open database of time-synced lyrics: exact match first (with and without the album, then with cleaned-up title and main artist), then a search filtered to ±2 s of the track length. Lyrics are never bundled with or stored in this repository.
- **Cache.** Lyrics you've seen are kept in a local SQLite file, so replays show instantly and work offline. "No lyrics found" is re-checked after 3 days. If a song matched the wrong version, use **Look up lyrics again** in the tray menu.

## Roadmap

- [x] **M0** Read Spotify via SMTC, interpolate position, log drift to CSV
- [x] **M1** Fetch and parse synced lyrics, print the current line in the terminal
- [x] **M2** Transparent always-on-top overlay window (Tauri)
- [x] **M3** Lock / click-through mode, drag to move, tray icon, settings
- [x] **M4** Local lyrics cache, visual refresh
- [x] **M5** Packaged releases (installer, portable zip, start with Windows)
- [ ] Later: per-track timing offset, word-by-word timing

## Using it

The lyrics appear when Spotify starts playing, stay up while it's paused, and go away when you quit Spotify. Drag them anywhere to move them.

| | |
| --- | --- |
| **Hover** | toolbar: previous / play-pause / next, text size A− A+, colour, lyric timing − 0.0s + (click the value to reset), lock, quit |
| **Lock** | clicks pass through to whatever is underneath. Hover the lyrics and an unlock button appears at the top |
| **Tray icon** | show/hide, lock/unlock, look up lyrics again, start with Windows, check for updates, open log folder, quit |
| `Ctrl+Alt+L` | lock / unlock |
| `Ctrl+Alt+D` | show / hide. While Spotify isn't playing, shows the window for a few seconds so you can move it |

**Start with Windows** in the tray menu is off by default. Turn it on and the overlay starts when you sign in, out of sight until Spotify plays.

Where things are kept:

| | |
| --- | --- |
| Settings (position, size, colour, timing, lock) | `%APPDATA%\io.github.allenjwzhu.lyrics-overlay\settings.json` |
| Cached lyrics (safe to delete) | `%LOCALAPPDATA%\io.github.allenjwzhu.lyrics-overlay\lyrics.sqlite3` |
| Log (attach it to bug reports) | `%LOCALAPPDATA%\io.github.allenjwzhu.lyrics-overlay\logs\overlay.log`, or **Open log folder** in the tray |

## Command-line prototype

`lyrics-cli` follows Spotify in the terminal instead (from source, or the `lyrics-cli-windows` artifact of any CI run):

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
  lyrics/    LRC parser, title/artist cleanup, LRCLIB client, SQLite cache
  follow.rs  Follower: current track, lyrics lookup with retries, current line
crates/cli   command-line front end
crates/app   overlay window (Tauri): settings, controller (lock, tray, shortcuts), worker; ui/ is plain HTML/CSS/JS, no build step
```

## Development

Build and run the overlay from source with a [Rust toolchain](https://rustup.rs):

```powershell
cargo run --release -p lyrics-overlay
cargo run -p lyrics-overlay -- --demo   # a built-in demo track, no Spotify needed (the only mode on macOS/Linux)
```

Checks CI runs:

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

### Making a release

1. Bump `version` in the root `Cargo.toml` and add a section for it to `CHANGELOG.md`.
2. Tag and push, e.g. `git tag -a v0.1.1 -m "Lyrics Overlay 0.1.1" && git push origin v0.1.1`.
3. The **Release** workflow builds the installer, the portable zip, `SHA256SUMS.txt` and the updater's `latest.json` on Windows and creates a **draft** release with the changelog section as its notes. Check it, then publish: installed copies only see published releases, so publishing is what ships the update.

The installer is signed for the in-app updater with the key in the repository secrets `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`; its public half is `plugins.updater.pubkey` in `crates/app/tauri.conf.json`. The workflow refuses to release if the signature doesn't verify against that key. Don't lose the private key: installed copies only accept updates signed with it.

To build the installer locally (Windows, needs Node.js): `cd crates/app && npx @tauri-apps/cli@2.12.4 build`. It lands in `target/release/bundle/nsis/`.

## License

[MIT](LICENSE). The bundled [Instrument Sans](https://github.com/Instrument/instrument-sans) font is under the SIL Open Font License (`crates/app/ui/fonts/OFL.txt`). Lyrics are provided by [LRCLIB](https://lrclib.net) at runtime. This project is not affiliated with or endorsed by Spotify.
