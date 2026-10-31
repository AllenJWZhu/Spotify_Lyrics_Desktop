# Changelog

## [0.1.0] - Unreleased

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
