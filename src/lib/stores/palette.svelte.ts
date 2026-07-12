const state = $state({
  open: false,
});

export const palette = {
  get open() {
    return state.open;
  },
  show() {
    state.open = true;
  },
  hide() {
    state.open = false;
  },
  toggle() {
    state.open = !state.open;
  },
};
