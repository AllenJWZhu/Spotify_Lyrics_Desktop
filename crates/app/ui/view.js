// What the overlay shows, computed from the backend's state + playback
// anchor and the current time. Pure functions only, so they can be tested
// with `node --test` (see crates/app/tests-js).

/** A line's fill never takes longer than this, so a line followed by a long
 * instrumental break doesn't crawl. */
export const MAX_FILL_MS = 8000;

/** Playback position (lyric time, ms) at `nowMs`, or null without an anchor. */
export function extrapolate(playback, nowMs) {
  if (!playback) return null;
  const elapsed = playback.playing ? Math.max(0, nowMs - playback.atUnixMs) : 0;
  return playback.positionMs + elapsed;
}

/** Index of the last line that has started at `pos`; -1 before the first. */
export function lineAt(lines, pos) {
  let lo = 0;
  let hi = lines.length;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if (lines[mid].startMs <= pos) lo = mid + 1;
    else hi = mid;
  }
  return lo - 1;
}

/** First and last index of the lines sharing line `i`'s timestamp
 * (an original line followed by its translation). */
export function groupBounds(lines, i) {
  const start = lines[i].startMs;
  let first = i;
  while (first > 0 && lines[first - 1].startMs === start) first--;
  let last = i;
  while (last + 1 < lines.length && lines[last + 1].startMs === start) last++;
  return { first, last };
}

function nextText(lines, after) {
  for (let i = after + 1; i < lines.length; i++) {
    if (lines[i].text) return lines[i].text;
  }
  return null;
}

const STATUS_TEXT = {
  loading: "Looking up lyrics…",
  missing: "No lyrics found",
  plain: "Only unsynced lyrics available",
  instrumental: "Instrumental",
};

/**
 * @returns {{
 *   key: string,
 *   primary: { text: string, fill: number | null, muted: boolean },
 *   secondary: { text: string, muted: boolean } | null,
 * }}
 * `fill` is the sung fraction (0..1) of the primary line, or null when it
 * isn't a lyric. `key` changes whenever the displayed line changes, to
 * trigger the enter animation.
 */
export function computeView(state, playback, nowMs) {
  const track = state?.track;
  if (!track) {
    return {
      key: "idle",
      primary: { text: state?.message ?? "Waiting for Spotify…", fill: null, muted: true },
      secondary: null,
    };
  }

  const header = `${track.title} — ${track.artist}`;
  if (state.lookup !== "synced") {
    const status =
      state.lookup === "error"
        ? state.retrying
          ? "Lyrics lookup failed, retrying…"
          : "Lyrics lookup failed"
        : STATUS_TEXT[state.lookup] ?? "";
    return {
      key: `info:${header}`,
      primary: { text: header, fill: null, muted: false },
      secondary: status ? { text: status, muted: true } : null,
    };
  }

  const lines = state.lines;
  const pos = extrapolate(playback, nowMs);
  let line = playback?.line ?? -1;
  // The backend reports line changes every ~100 ms and owns backward moves
  // (seeks). Moving forward at the exact moment is done here.
  if (pos !== null && playback.playing) line = Math.max(line, lineAt(lines, pos));

  if (line < 0) {
    const upcoming = nextText(lines, -1);
    return {
      key: `intro:${header}`,
      primary: { text: header, fill: null, muted: true },
      secondary: upcoming ? { text: upcoming, muted: true } : null,
    };
  }

  const { first, last } = groupBounds(lines, line);
  const current = lines[first];
  const key = `line:${header}:${first}`;

  if (!current.text) {
    const upcoming = nextText(lines, last);
    return {
      key,
      primary: { text: "♪", fill: null, muted: true },
      secondary: upcoming ? { text: upcoming, muted: true } : null,
    };
  }

  const nextStart = lines[last + 1]?.startMs ?? current.startMs + MAX_FILL_MS;
  const duration = Math.max(1, Math.min(nextStart - current.startMs, MAX_FILL_MS));
  const fill = pos === null ? 0 : Math.min(1, Math.max(0, (pos - current.startMs) / duration));

  const translation = lines
    .slice(first + 1, last + 1)
    .map((l) => l.text)
    .filter(Boolean)
    .join(" / ");
  const secondaryText = translation || nextText(lines, last);

  return {
    key,
    primary: { text: current.text, fill, muted: false },
    secondary: secondaryText
      ? { text: secondaryText, muted: !translation }
      : null,
  };
}

/** Lyric timing offset for display: "+0.5s", "−1.0s", "0.0s". */
export function formatOffset(ms) {
  const sign = ms > 0 ? "+" : ms < 0 ? "−" : "";
  return `${sign}${(Math.abs(ms) / 1000).toFixed(1)}s`;
}
