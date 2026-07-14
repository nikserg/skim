// Mock of `@tauri-apps/api/window`. Window chrome (minimize/maximize/close)
// is inert in the browser demo — the buttons stay visible but do nothing.
let maximized = false;

export function getCurrentWindow() {
  return {
    async minimize() {},
    async maximize() {
      maximized = true;
    },
    async unmaximize() {
      maximized = false;
    },
    async toggleMaximize() {
      maximized = !maximized;
    },
    async isMaximized() {
      return maximized;
    },
    async close() {},
    async setTitle() {},
    async listen() {
      return () => {};
    },
  };
}

export const appWindow = getCurrentWindow();
