import { api } from "../api";
import type { Theme } from "../types";

const media = window.matchMedia("(prefers-color-scheme: dark)");

/** AI actions the reading pane exposes to the global keyboard handler. */
type ReadingAiActions = { draftReply: () => void; summarize: () => void; ask: () => void };
let readingAi: ReadingAiActions | null = null;

const state = $state({
  theme: "system" as Theme,
  /** Resolved light/dark after applying the system preference. */
  effective: "light" as "light" | "dark",
  settingsOpen: false,
  /** AI Recap panel occupies the reading pane while open. */
  recapOpen: false,
  /** Keyboard-shortcuts cheat sheet overlay. */
  shortcutsOpen: false,
});

function effectiveTheme(): "light" | "dark" {
  if (state.theme === "system") return media.matches ? "dark" : "light";
  return state.theme;
}

function applyTheme() {
  state.effective = effectiveTheme();
  document.documentElement.dataset.theme = state.effective;
}

media.addEventListener("change", applyTheme);
applyTheme();

export const ui = {
  get theme() {
    return state.theme;
  },
  get effective() {
    return state.effective;
  },
  setTheme(theme: Theme) {
    state.theme = theme;
    applyTheme();
  },
  cycleTheme() {
    const order: Theme[] = ["light", "dark", "system"];
    const next = order[(order.indexOf(state.theme) + 1) % order.length];
    this.setTheme(next);
    void api.setSetting("theme", next).catch(() => {});
  },
  get settingsOpen() {
    return state.settingsOpen;
  },
  openSettings() {
    state.settingsOpen = true;
  },
  closeSettings() {
    state.settingsOpen = false;
  },
  get recapOpen() {
    return state.recapOpen;
  },
  openRecap() {
    state.recapOpen = true;
  },
  closeRecap() {
    state.recapOpen = false;
  },
  get shortcutsOpen() {
    return state.shortcutsOpen;
  },
  openShortcuts() {
    state.shortcutsOpen = true;
  },
  closeShortcuts() {
    state.shortcutsOpen = false;
  },
  get readingAi() {
    return readingAi;
  },
  setReadingAi(actions: ReadingAiActions | null) {
    readingAi = actions;
  },
};
