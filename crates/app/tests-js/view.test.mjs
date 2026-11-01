// Run with: node --test "crates/app/tests-js/*.test.mjs"
import { test } from "node:test";
import assert from "node:assert/strict";

import {
  MAX_FILL_MS,
  computeView,
  extrapolate,
  formatOffset,
  groupBounds,
  lineAt,
} from "../ui/view.js";

const track = { title: "Song", artist: "Artist" };
const lines = [
  { startMs: 1000, text: "line one" },
  { startMs: 3000, text: "line two" },
  { startMs: 3000, text: "translation two" },
  { startMs: 5000, text: "" },
  { startMs: 6000, text: "line four" },
];
const synced = { track, lookup: "synced", retrying: false, lines, source: "LRCLIB", message: null };

const playing = (positionMs, atUnixMs, line) => ({ positionMs, atUnixMs, playing: true, line });
const paused = (positionMs, atUnixMs, line) => ({ positionMs, atUnixMs, playing: false, line });

test("extrapolate advances only while playing", () => {
  assert.equal(extrapolate(null, 5), null);
  assert.equal(extrapolate(playing(1000, 100, null), 600), 1500);
  assert.equal(extrapolate(playing(1000, 100, null), 50), 1000);
  assert.equal(extrapolate(paused(1000, 100, null), 600), 1000);
});

test("lineAt and groupBounds", () => {
  assert.equal(lineAt(lines, 0), -1);
  assert.equal(lineAt(lines, 1000), 0);
  assert.equal(lineAt(lines, 3500), 2);
  assert.equal(lineAt(lines, 99999), 4);
  assert.equal(lineAt([], 5), -1);
  assert.deepEqual(groupBounds(lines, 2), { first: 1, last: 2 });
  assert.deepEqual(groupBounds(lines, 1), { first: 1, last: 2 });
  assert.deepEqual(groupBounds(lines, 0), { first: 0, last: 0 });
});

test("no track shows the waiting message or the backend's message", () => {
  assert.equal(computeView(null, null, 0).primary.text, "Play something in Spotify");
  const v = computeView({ track: null, message: "Can't reach media controls" }, null, 0);
  assert.equal(v.primary.text, "Can't reach media controls");
  assert.equal(v.primary.muted, true);
});

test("non-synced lookups show the track and a status", () => {
  const cases = [
    [{ lookup: "loading" }, "Looking up lyrics…"],
    [{ lookup: "missing" }, "No lyrics found for this song"],
    [{ lookup: "plain" }, "Only unsynced lyrics exist for this song"],
    [{ lookup: "instrumental" }, "Instrumental"],
    [
      { lookup: "error", retrying: true, error: "LRCLIB is down right now" },
      "LRCLIB is down right now. Trying again automatically",
    ],
    [
      { lookup: "error", retrying: false, error: null },
      "Couldn't get lyrics from LRCLIB. Use “Look up lyrics again” in the tray menu to retry",
    ],
  ];
  for (const [extra, text] of cases) {
    const v = computeView({ ...synced, lines: [], ...extra }, null, 0);
    assert.equal(v.primary.text, "Song");
    assert.equal(v.primary.fill, null);
    assert.equal(v.secondary.text, text);
  }
});

test("before the first line: title over artist", () => {
  const v = computeView(synced, playing(200, 0, null), 300);
  assert.equal(v.primary.text, "Song");
  assert.equal(v.primary.fill, null);
  assert.equal(v.secondary.text, "Artist");
});

test("karaoke fill follows the extrapolated position", () => {
  // Line one runs 1.0 s → 3.0 s. Anchor: 1.0 s at t=0; at t=500 → 1.5 s.
  const v = computeView(synced, playing(1000, 0, 0), 500);
  assert.equal(v.primary.text, "line one");
  assert.equal(v.primary.fill, 0.25);
  assert.equal(v.secondary.text, "line two");
  assert.equal(v.secondary.muted, true);
  assert.equal(computeView(synced, playing(1000, 0, 0), 1999).primary.fill, 0.9995);
});

test("moves to the next line on time without waiting for the backend", () => {
  // Backend still says line 0, but 3.0 s has passed.
  const v = computeView(synced, playing(2900, 0, 0), 150);
  assert.equal(v.primary.text, "line two");
  assert.equal(v.secondary.text, "translation two");
  assert.equal(v.secondary.muted, false);
});

test("backend line wins when paused (small backward corrections)", () => {
  // Backend keeps line 4 even though the paused position is just before it.
  const v = computeView(synced, paused(5850, 0, 4), 500);
  assert.equal(v.primary.text, "line four");
  assert.equal(v.primary.fill, 0);
});

test("instrumental gap shows a note and what comes next", () => {
  const v = computeView(synced, playing(5200, 0, 3), 0);
  assert.equal(v.primary.text, "♪");
  assert.equal(v.primary.fill, null);
  assert.equal(v.secondary.text, "line four");
});

test("fill duration is capped for lines followed by a long break", () => {
  const last = computeView(synced, playing(6000 + MAX_FILL_MS / 2, 0, 4), 0);
  assert.equal(last.primary.fill, 0.5);
  assert.equal(last.secondary, null);
});

test("key changes with the displayed line", () => {
  const a = computeView(synced, playing(1100, 0, 0), 0).key;
  const b = computeView(synced, playing(1900, 0, 0), 0).key;
  const c = computeView(synced, playing(3100, 0, 2), 0).key;
  assert.equal(a, b);
  assert.notEqual(a, c);
});

test("offsets are formatted with a sign and one decimal", () => {
  assert.equal(formatOffset(0), "0.0s");
  assert.equal(formatOffset(500), "+0.5s");
  assert.equal(formatOffset(-1500), "−1.5s");
});
