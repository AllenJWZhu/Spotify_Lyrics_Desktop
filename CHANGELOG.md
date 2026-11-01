# Changelog

## [0.1.7] - Unreleased

**Lyrics**
- Songs that only had plain (untimed) lyrics are looked up again after a few days, so synced lyrics that turn up later replace them.
- A backup source saying a song is instrumental no longer hides lyrics the main source has.
- One of the backup sources answers faster: its two search methods now run at the same time.
- The log says which entry matched (title, artist and length), to make a wrong version easy to spot.

## [0.1.6] - 2026-10-31

**Overlay**
- The sung part of a line no longer has a coloured glow spilling outside the letters; the colour stays inside the outline.

## [0.1.5] - 2026-10-31

**Lyrics**
- Another backup lyrics source, with much better K-pop coverage: the original recordings of most K-pop songs, synced. It's asked at the same time as the others.
- The tray switch is now **Backup lyrics sources** and covers all the backups.

## [0.1.4] - 2026-10-31

**Lyrics**
- Lyrics sources are now asked at the same time, and the first one with synced lyrics wins. A source that is down no longer holds up the others, so lyrics show as soon as anyone has them.
- When a source is down, the message says so plainly, without an HTTP code.

## [0.1.3] - 2026-10-31

**Lyrics**
- A backup lyrics source: when LRCLIB has no synced lyrics for a song or can't be reached, the overlay now looks elsewhere. It helps most with Chinese, Japanese and Korean songs, and often brings a Chinese translation, shown under each line. A match needs the same artist, title and length, so covers and other versions are skipped. It can be turned off from the tray menu.
- Error messages name the service that failed.

## [0.1.2] - 2026-10-31

**Installer**
- A new setup window in the app's own style replaces the Windows wizard: one **Install** button, with a desktop shortcut and Start with Windows under **Options**, then **Open Lyrics Overlay**. In English or Chinese, following Windows.
- The classic wizard is still published (`-classic-setup.exe`), now with the app's icon and artwork; it opens by itself on PCs without the WebView2 runtime.
- Updates install silently in the background (from 0.1.2 on); before, a progress window showed.

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
