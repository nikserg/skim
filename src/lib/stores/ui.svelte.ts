import type { Theme } from "../types";

const media = window.matchMedia("(prefers-color-scheme: dark)");

const state = $state({
  theme: (localStorage.getItem("skim.theme") as Theme) || "system",
  selectedThreadId: null as number | null,
  selectedFolderId: 1,
});

function effectiveTheme(): "light" | "dark" {
  if (state.theme === "system") return media.matches ? "dark" : "light";
  return state.theme;
}

function applyTheme() {
  document.documentElement.dataset.theme = effectiveTheme();
}

media.addEventListener("change", applyTheme);
applyTheme();

export const ui = {
  get theme() {
    return state.theme;
  },
  setTheme(theme: Theme) {
    state.theme = theme;
    localStorage.setItem("skim.theme", theme);
    applyTheme();
  },
  cycleTheme() {
    const order: Theme[] = ["light", "dark", "system"];
    this.setTheme(order[(order.indexOf(state.theme) + 1) % order.length]);
  },
  get selectedThreadId() {
    return state.selectedThreadId;
  },
  set selectedThreadId(id: number | null) {
    state.selectedThreadId = id;
  },
  get selectedFolderId() {
    return state.selectedFolderId;
  },
  set selectedFolderId(id: number) {
    state.selectedFolderId = id;
  },
};
