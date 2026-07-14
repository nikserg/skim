import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";
import { resolve } from "node:path";

// Demo build: identical to the app, but every Tauri IPC module is swapped for
// a browser-side mock (see ./mock). Run with `vite --config demo/vite.demo.config.ts`.
const dir = import.meta.dirname;
const root = resolve(dir, "..");
const mock = (f) => resolve(dir, "mock", f);

export default defineConfig({
  root,
  plugins: [svelte()],
  clearScreen: false,
  resolve: {
    alias: [
      { find: "@tauri-apps/api/core", replacement: mock("tauri-core.ts") },
      { find: "@tauri-apps/api/window", replacement: mock("tauri-window.ts") },
      { find: "@tauri-apps/api/event", replacement: mock("tauri-event.ts") },
      { find: "@tauri-apps/api/app", replacement: mock("tauri-app.ts") },
      { find: "@tauri-apps/plugin-opener", replacement: mock("plugin-opener.ts") },
      { find: "@tauri-apps/plugin-autostart", replacement: mock("plugin-autostart.ts") },
    ],
  },
  server: { port: 1421, strictPort: true, host: false },
  build: { outDir: resolve(dir, "dist-demo"), emptyOutDir: true, target: "chrome120" },
});
