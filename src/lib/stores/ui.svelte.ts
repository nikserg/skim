import { api } from "../api";
import type { Lightness, Temperature, Theme } from "../types";

const media = window.matchMedia("(prefers-color-scheme: dark)");

/** AI actions the reading pane exposes to the global keyboard handler. */
type ReadingAiActions = { ask: () => void };
let readingAi: ReadingAiActions | null = null;

/** Message currently open in the reading pane, for AI context. Non-reactive —
 * the palette chat reads it once when a session starts. */
let openMessageId: number | null = null;

const state = $state({
  /** Warm (quiet-zine) vs cold (classic) neutrals. */
  temperature: "warm" as Temperature,
  lightness: "light" as Lightness,
  settingsOpen: false,
  /** AI Recap panel occupies the reading pane while open. */
  recapOpen: false,
  /** Keyboard-shortcuts cheat sheet overlay. */
  shortcutsOpen: false,
  /** Left menu collapsed to an icon-only rail. */
  sidebarCollapsed: false,
});

/** Parse a persisted theme string into the two axes, migrating legacy values.
 *  Legacy (single-axis) values map onto the cold palette so existing users keep
 *  their look; "system" is resolved once against the OS (auto-switching is gone).
 *  Anything unknown/empty falls back to the new default, warm-light. */
function parseTheme(raw: string | undefined): { temperature: Temperature; lightness: Lightness } {
  switch (raw) {
    case "cold-light":
    case "cold-dark":
    case "warm-light":
    case "warm-dark": {
      const [temperature, lightness] = raw.split("-") as [Temperature, Lightness];
      return { temperature, lightness };
    }
    case "light":
      return { temperature: "cold", lightness: "light" };
    case "dark":
      return { temperature: "cold", lightness: "dark" };
    case "system":
      return { temperature: "cold", lightness: media.matches ? "dark" : "light" };
    default:
      return { temperature: "warm", lightness: "light" };
  }
}

function serialize(): Theme {
  return `${state.temperature}-${state.lightness}`;
}

function applyTheme() {
  // One attribute carries both axes, e.g. "warm-dark". Exactly one token block
  // in tokens.css matches at a time, so there is no specificity/order guessing.
  document.documentElement.dataset.theme = serialize();
}

applyTheme();

export const ui = {
  get temperature() {
    return state.temperature;
  },
  get lightness() {
    return state.lightness;
  },
  /** Resolved light/dark. Consumed by HtmlViewer for email-body rendering. */
  get effective() {
    return state.lightness;
  },
  /** Full persisted value, e.g. "warm-light". */
  get theme(): Theme {
    return serialize();
  },
  /** Apply both axes. Does not persist — callers persist (mirrors old contract). */
  setTheme(temperature: Temperature, lightness: Lightness) {
    state.temperature = temperature;
    state.lightness = lightness;
    applyTheme();
  },
  /** Boot path: apply a persisted (possibly legacy) value, return the normalized
   *  string so the caller can write it back only when it actually changed. */
  hydrate(raw: string | undefined): Theme {
    const { temperature, lightness } = parseTheme(raw);
    state.temperature = temperature;
    state.lightness = lightness;
    applyTheme();
    return serialize();
  },
  /** Command-palette "Toggle theme": flip lightness, keep temperature. */
  cycleTheme() {
    this.setTheme(state.temperature, state.lightness === "light" ? "dark" : "light");
    void api.setSetting("theme", serialize()).catch(() => {});
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
  get sidebarCollapsed() {
    return state.sidebarCollapsed;
  },
  /** Apply the collapsed state without persisting (boot path). */
  setSidebarCollapsed(collapsed: boolean) {
    state.sidebarCollapsed = collapsed;
  },
  /** Toggle the left rail and persist the choice. */
  toggleSidebar() {
    state.sidebarCollapsed = !state.sidebarCollapsed;
    void api.setSetting("sidebar_collapsed", state.sidebarCollapsed ? "on" : "off").catch(() => {});
  },
  get readingAi() {
    return readingAi;
  },
  setReadingAi(actions: ReadingAiActions | null) {
    readingAi = actions;
  },
  get openMessageId() {
    return openMessageId;
  },
  setOpenMessage(id: number | null) {
    openMessageId = id;
  },
};
