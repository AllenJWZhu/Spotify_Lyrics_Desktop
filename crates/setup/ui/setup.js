// The setup window: one lyric line as the headline, one button, two options.
// States: ready → installing → done (or error → installing again).

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const STRINGS = {
  en: {
    headline: "Lyrics, floating on your desktop",
    pitch: "In time with Spotify, above every window. Lock it and clicks pass straight through.",
    install: "Install",
    update: "Update",
    reinstall: "Reinstall",
    close: "Close",
    options: "Options",
    desktopShortcut: "Desktop shortcut",
    startWithWindows: "Start with Windows",
    footer: (v) => `Version ${v}. Installs for your account only, no administrator rights needed.`,
    older: (old) => `Version ${old} is installed. Updating keeps your settings and saved lyrics.`,
    same: "This version is already installed. Reinstalling keeps your settings.",
    newer: (installed) => `Version ${installed} is already installed, which is newer than this one.`,
    installing: "Installing…",
    installingOver: "Installing… If Lyrics Overlay is running, it closes for a moment.",
    doneHeadline: "Lyrics Overlay is installed",
    done: "Play a song in Spotify and the lyrics appear above the taskbar.",
    open: "Open Lyrics Overlay",
    failed: (detail) => `Installation didn't finish. ${detail}`,
    retry: "Try again",
  },
  zh: {
    headline: "让歌词浮在桌面上",
    pitch: "跟着 Spotify 同步滚动，悬浮在所有窗口之上。锁定后鼠标可以直接穿透。",
    install: "安装",
    update: "更新",
    reinstall: "重新安装",
    close: "关闭",
    options: "选项",
    desktopShortcut: "桌面快捷方式",
    startWithWindows: "开机自动启动",
    footer: (v) => `版本 ${v}。仅为当前用户安装，不需要管理员权限。`,
    older: (old) => `已安装版本 ${old}。更新会保留你的设置和已保存的歌词。`,
    same: "已安装此版本。重新安装会保留你的设置。",
    newer: (installed) => `已安装更新的版本 ${installed}，不需要再安装这个版本。`,
    installing: "正在安装…",
    installingOver: "正在安装… 如果 Lyrics Overlay 正在运行，会短暂关闭。",
    doneHeadline: "Lyrics Overlay 已安装",
    done: "在 Spotify 里播放歌曲，歌词就会出现在任务栏上方。",
    open: "打开 Lyrics Overlay",
    failed: (detail) => `安装没有完成。${detail}`,
    retry: "重试",
  },
};

const t = navigator.language?.toLowerCase().startsWith("zh") ? STRINGS.zh : STRINGS.en;
document.documentElement.lang = t === STRINGS.zh ? "zh-CN" : "en";

const $ = (id) => document.getElementById(id);
const els = {
  headline: $("headline"),
  headlineText: $("headline-text"),
  headlineSung: $("headline-sung"),
  message: $("message"),
  primary: $("primary"),
  secondary: $("secondary"),
  optionsToggle: $("options-toggle"),
  options: $("options"),
  desktopShortcut: $("desktop-shortcut"),
  startWithWindows: $("start-with-windows"),
  footer: $("footer"),
};

let info;
let onPrimary = () => {};
let onSecondary = () => {};

function setHeadline(text, mode) {
  els.headlineText.textContent = text;
  els.headlineSung.textContent = text;
  els.headline.classList.remove("intro", "working", "done");
  // Restart the animation even when the mode is the same as before.
  void els.headline.offsetWidth;
  if (mode) els.headline.classList.add(mode);
}

function setMessage(text, isError = false) {
  els.message.textContent = text;
  els.message.classList.toggle("error", isError);
}

function setButtons(primary, secondary, { options = false } = {}) {
  els.primary.textContent = primary.label;
  onPrimary = primary.action;
  els.secondary.hidden = !secondary;
  if (secondary) {
    els.secondary.textContent = secondary.label;
    onSecondary = secondary.action;
  }
  els.optionsToggle.hidden = !options;
  if (!options) showOptions(false);
}

function showOptions(open) {
  els.options.hidden = !open;
  els.optionsToggle.setAttribute("aria-expanded", String(open));
}

function ready() {
  document.body.classList.remove("busy");
  setHeadline(t.headline, "intro");
  const close = { label: t.close, action: quit };
  switch (info.relation) {
    case "older":
      setMessage(t.older(info.installed));
      setButtons({ label: t.update, action: install }, null, { options: true });
      break;
    case "same":
      setMessage(t.same);
      setButtons({ label: t.reinstall, action: install }, null, { options: true });
      break;
    case "newer":
      setMessage(t.newer(info.installed));
      setButtons(close, null);
      break;
    default:
      setMessage(t.pitch);
      setButtons({ label: t.install, action: install }, null, { options: true });
  }
}

async function install() {
  document.body.classList.add("busy");
  setHeadline(t.headline, "working");
  setMessage(info.relation === "none" ? t.installing : t.installingOver);
  await invoke("install", {
    options: {
      desktopShortcut: els.desktopShortcut.checked,
      startWithWindows: els.startWithWindows.checked,
    },
  });
}

function finished({ ok, detail }) {
  document.body.classList.remove("busy");
  if (ok) {
    setHeadline(t.doneHeadline, "done");
    setMessage(t.done);
    setButtons({ label: t.open, action: open }, { label: t.close, action: quit });
  } else {
    setHeadline(t.headline, "intro");
    setMessage(t.failed(detail ?? ""), true);
    setButtons({ label: t.retry, action: install }, { label: t.close, action: quit }, { options: true });
  }
  els.primary.focus({ focusVisible: false });
}

async function open() {
  try {
    await invoke("launch");
  } catch (e) {
    setMessage(String(e), true);
  }
}

function quit() {
  invoke("quit");
}

els.primary.addEventListener("click", () => onPrimary());
els.secondary.addEventListener("click", () => onSecondary());
$("close").addEventListener("click", quit);
els.optionsToggle.addEventListener("click", () => showOptions(els.options.hidden));
document.addEventListener("keydown", (e) => {
  if (e.key === "Escape" && !document.body.classList.contains("busy")) quit();
});

$("options-label").textContent = t.options;
$("desktop-shortcut-label").textContent = t.desktopShortcut;
$("start-with-windows-label").textContent = t.startWithWindows;
document.title = t === STRINGS.zh ? "Lyrics Overlay 安装" : "Lyrics Overlay Setup";

await listen("setup-finished", (event) => finished(event.payload));
info = await invoke("info");
els.desktopShortcut.checked = info.defaults.desktopShortcut;
els.startWithWindows.checked = info.defaults.startWithWindows;
els.footer.textContent = t.footer(info.version);
ready();
await document.fonts.ready;
// Enter installs, without drawing a focus ring nobody asked for yet.
els.primary.focus({ focusVisible: false });
invoke("ready");
