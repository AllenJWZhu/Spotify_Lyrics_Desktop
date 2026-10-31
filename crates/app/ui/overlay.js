// Wires the backend (Tauri events/commands) to the DOM. All display logic
// is in view.js.
import { computeView } from "./view.js";

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

/** Smallest font scale before long lines are cut with an ellipsis. */
const MIN_SCALE = 0.55;

let state = null;
let playback = null;
let lastKey = null;

const primary = document.getElementById("primary");
const secondary = document.getElementById("secondary");

function setLine(el, text, muted) {
  if (el.dataset.text !== text) {
    el.dataset.text = text;
    for (const layer of el.querySelectorAll(".layer")) layer.textContent = text;
    fitWidth(el);
  }
  el.classList.toggle("muted", muted);
}

/** Shrinks the font until the line fits the window, down to MIN_SCALE. */
function fitWidth(el) {
  el.style.setProperty("--scale", "1");
  const available = el.parentElement.clientWidth - 64;
  const needed = el.querySelector(".base").scrollWidth;
  if (needed > available) {
    el.style.setProperty("--scale", String(Math.max(MIN_SCALE, available / needed)));
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
}

function frame() {
  render(computeView(state, playback, Date.now()));
  requestAnimationFrame(frame);
}

async function start() {
  document.getElementById("close").addEventListener("click", () => invoke("quit"));

  await listen("overlay-state", (event) => {
    state = event.payload;
  });
  await listen("overlay-playback", (event) => {
    playback = event.payload;
  });
  // Fetched after subscribing, so nothing sent in between is lost.
  const initial = await invoke("get_state");
  state ??= initial.state;
  playback ??= initial.playback;

  requestAnimationFrame(frame);
}

start();
