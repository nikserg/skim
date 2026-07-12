import { aiApi } from "../api";

const state = $state({
  keyPresent: false,
  checked: false,
});

export const ai = {
  get keyPresent() {
    return state.keyPresent;
  },
  get checked() {
    return state.checked;
  },
  async refresh() {
    try {
      state.keyPresent = await aiApi.keyStatus();
    } catch {
      state.keyPresent = false;
    }
    state.checked = true;
  },
};
