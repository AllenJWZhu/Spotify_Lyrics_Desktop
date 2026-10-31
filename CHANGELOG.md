# Changelog

## [0.1.2] - Unreleased

**Installer**
- The installer and uninstaller carry the app's icon instead of the generic NSIS one.
- New artwork on the installer's pages, in the app's colours.

## [0.1.1] - 2026-10-31

**Updates**
- The overlay checks for a new version shortly after it starts and every few hours, and offers to install it: one click downloads it, installs it and restarts the overlay. **Check for updates** in the tray checks right away.
- Updates are signed, and the overlay only installs ones signed with this project's key.
- 0.1.0 can't update itself: install 0.1.1 by hand once, and later versions arrive in the app.

## [0.1.0] - 2026-10-31

First release: floating desktop lyrics for the Spotify desktop app on Windows.

**Lyrics**
- Follows Spotify through the Windows media controls: no Spotify login, works with free accounts.
- Synced lyrics from [LRCLIB](https://lrclib.net), matched by title, artist and length; lines light up as they are sung, with translations shown underneath when the lyrics include them.
- Lyrics you've seen are cached on your computer, so replays show instantly and work offline. **Look up lyrics again** in the tray fixes a wrong match.
- If LRCLIB is down or unreachable, the overlay says why and keeps retrying while the song plays.

**Overlay**
- Transparent, always on top, no taskbar button. Drag to move; hover for playback controls, text size, colour, and lyric timing.
- Appears when Spotify starts playing, stays up through pauses, and hides when Spotify quits.
- Lock (`Ctrl+Alt+L`) so clicks pass through; hover the lyrics to unlock. Show or hide with `Ctrl+Alt+D`.
- Tray menu: show/hide, lock, look up lyrics again, start with Windows, open log folder, quit.
- Remembers position, size, colour, timing and lock state.

**Known limitations**
- Windows only. The installer isn't code-signed yet, so Windows SmartScreen asks for confirmation the first time.
- Lyrics depend on LRCLIB's coverage; some songs only have unsynced lyrics or none.
