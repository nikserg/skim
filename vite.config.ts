import { defineConfig, type Plugin } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

const host = process.env.TAURI_DEV_HOST;

// @fontsource ships a legacy `.woff` copy next to every `.woff2`. Skim only ever
// runs inside WebView2 (Chromium), which always prefers `woff2`, so the `.woff`
// files are never fetched — they just bloat the installer. Drop them from the
// bundle and strip their now-dead `url(...) format('woff')` from the emitted CSS.
function dropWoff1(): Plugin {
  return {
    name: "skim-drop-woff1",
    enforce: "post",
    generateBundle(_options, bundle) {
      for (const [fileName, chunk] of Object.entries(bundle)) {
        if (fileName.endsWith(".woff")) {
          delete bundle[fileName];
        } else if (chunk.type === "asset" && fileName.endsWith(".css")) {
          chunk.source = chunk.source
            .toString()
            .replace(/,\s*url\([^)]*\.woff\)\s*format\((['"])woff\1\)/g, "");
        }
      }
    },
  };
}

// https://vitejs.dev/config/
export default defineConfig({
  plugins: [svelte(), dropWoff1()],

  // Vite options tailored for Tauri development
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    watch: {
      // tell vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**"],
    },
  },
  envPrefix: ["VITE_", "TAURI_ENV_*"],
  build: {
    target: "chrome120",
    minify: "esbuild",
    sourcemap: !!process.env.TAURI_ENV_DEBUG,
  },
});
