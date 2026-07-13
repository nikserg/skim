import { aiApi, type AiProvider } from "../api";

const state = $state({
  keyPresent: false,
  checked: false,
  provider: "anthropic" as AiProvider,
  anthropic: false,
  openrouter: false,
});

export const ai = {
  /** Whether the ACTIVE provider has a key — the gate for AI features. */
  get keyPresent() {
    return state.keyPresent;
  },
  get checked() {
    return state.checked;
  },
  get provider() {
    return state.provider;
  },
  get anthropic() {
    return state.anthropic;
  },
  get openrouter() {
    return state.openrouter;
  },
  async refresh() {
    try {
      const s = await aiApi.keyStatus();
      state.provider = s.provider;
      state.anthropic = s.anthropic;
      state.openrouter = s.openrouter;
      state.keyPresent = s.provider === "openrouter" ? s.openrouter : s.anthropic;
    } catch {
      state.keyPresent = false;
    }
    state.checked = true;
  },
};
