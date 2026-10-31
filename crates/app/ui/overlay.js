// Wires the backend (Tauri events/commands) to the DOM. Display logic for
// the lyrics is in view.js; settings are owned by the app and only
// reflected here.
import { computeView, formatOffset } from "./view.js";

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

/** Smallest font scale before long lines are cut with an ellipsis. */
const MIN_FIT = 0.55;

let state = null;
let playback = null;
let settings = null;
let lastKey = null;

const root = document.documentElement;
const primary = document.getElementById("primary");
const secondary = document.getElementById("secondary");
const playPause = document.getElementById("play-pause");
const offsetLabel = document.getElementById("offset");
const unlockButton = document.getElementById("unlock");

// ---- Lyrics ---------------------------------------------------------------

function setLine(el, text, muted) {
  if (el.dataset.text !== text) {
    el.dataset.text = text;
    for (const layer of el.querySelectorAll(".layer")) layer.textContent = text;
    fitWidth(el);
  }
  el.classList.toggle("muted", muted);
}

/** Shrinks the font until the line fits the window, down to MIN_FIT. */
function fitWidth(el) {
  el.style.setProperty("--scale", "1");
  const available = el.parentElement.clientWidth - 64;
  const needed = el.querySelector(".base").scrollWidth;
  if (needed > available) {
    el.style.setProperty("--scale", String(Math.max(MIN_FIT, available / needed)));
  }
}

function render(view) {
  setLine(primary, view.primary.text, view.primary.muted);
  primary.style.setProperty("--fill", String(view.primary.fill ?? 0));
  primary.querySelector(".sung").hidden = view.primary.fill === null;

  secondary.hidden = view.secondary === null;
  if (view.secondary) setLine(secondary, view.secondary.text, view.secondary.muted);

  if (view.key !== lastKey) {
    lastKey = view.key;
    primary.classList.remove("enter");
    void primary.offsetWidth; // restart the animation
    primary.classList.add("enter");
  }
  playPause.classList.toggle("playing", Boolean(playback?.playing));
}

function frame() {
  render(computeView(state, playback, Date.now()));
  requestAnimationFrame(frame);
}

// ---- Settings ---------------------------------------------------------------

function applySettings(next) {
  settings = next;
  root.dataset.theme = next.theme;
  root.style.setProperty("--font-scale", String(next.fontScale));
  document.body.classList.toggle("locked", next.locked);
  offsetLabel.textContent = formatOffset(next.offsetMs);
  // Layout changed: re-fit lines and re-report the unlock button.
  for (const el of [primary, secondary]) if (el.dataset.text) fitWidth(el);
  reportUnlockButton();
}

/** Tells the app where the unlock button is, so it can let clicks through
 * everywhere else while locked. */
function reportUnlockButton() {
  if (!settings?.locked) {
    invoke("set_unlock_hotspot", { rect: null });
    return;
  }
  const r = unlockButton.getBoundingClientRect();
  invoke("set_unlock_hotspot", { rect: { x: r.x, y: r.y, width: r.width, height: r.height } });
}

// ---- Controls ---------------------------------------------------------------

const ACTIONS = {
  previous: () => invoke("player_control", { action: "previous" }),
  playPause: () => invoke("player_control", { action: "playPause" }),
  next: () => invoke("player_control", { action: "next" }),
  fontDown: () => invoke("step_font", { steps: -1 }),
  fontUp: () => invoke("step_font", { steps: 1 }),
  theme: () => invoke("cycle_theme"),
  offsetDown: () => invoke("step_offset", { steps: -1 }),
  offsetUp: () => invoke("step_offset", { steps: 1 }),
  offsetReset: () => invoke("reset_offset"),
  lock: () => invoke("set_locked", { locked: true }),
  unlock: () => invoke("set_locked", { locked: false }),
  close: () => invoke("quit_app"),
};

document.addEventListener("click", (event) => {
  const button = event.target.closest("button[data-action]");
  if (button) ACTIONS[button.dataset.action]?.();
});

// ---- Startup ----------------------------------------------------------------

async function start() {
  await listen("overlay-state", (event) => {
    state = event.payload;
  });
  await listen("overlay-playback", (event) => {
    playback = event.payload;
  });
  await listen("overlay-settings", (event) => applySettings(event.payload));
  await listen("overlay-pointer", (event) => {
    document.body.classList.toggle("pointer-inside", event.payload.inside);
  });

  // Fetched after subscribing, so nothing sent in between is lost.
  const initial = await invoke("get_state");
  state ??= initial.state;
  playback ??= initial.playback;
  if (!settings) applySettings(initial.settings);

  new ResizeObserver(() => applySettings(settings)).observe(document.body);
  requestAnimationFrame(frame);
}

start();
