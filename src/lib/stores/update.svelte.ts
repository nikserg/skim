import { check, type Update } from "@tauri-apps/plugin-updater";
import { api } from "../api";

/** Wall-clock gate between real checks against GitHub. The app lives in the
 * tray for days, so the timestamp persists across restarts — a fresh boot
 * doesn't re-poll if yesterday's tick already did. */
const CHECK_EVERY_MS = 24 * 60 * 60 * 1000;
/** Let the boot mail sync win the first minute of network and CPU. */
const FIRST_TICK_MS = 60_000;
/** How often the gate is re-evaluated while the app keeps running. */
const TICK_MS = 60 * 60 * 1000;

type UpdateStatus = "idle" | "available" | "downloading" | "ready" | "error";

const state = $state({
  status: "idle" as UpdateStatus,
  version: null as string | null,
  /** 0..1 while downloading; -1 when the total size is unknown. */
  progress: 0,
});

/** Plugin handle for the offered release (a Rust-side resource). Non-reactive —
 * only the banner state machine touches it. */
let update: Update | null = null;
let dismissed = "";
let lastCheck = 0;
let started = false;

async function checkNow() {
  if (state.status !== "idle") return;
  try {
    const found = await check();
    lastCheck = Date.now();
    void api.setSetting("update_last_check", String(lastCheck)).catch(() => {});
    if (found && found.version !== dismissed) {
      update = found;
      state.version = found.version;
      state.status = "available";
    }
  } catch {
    // Offline or a GitHub hiccup — stay quiet; lastCheck was not advanced,
    // so a later hourly tick simply retries.
  }
}

export const updater = {
  get status() {
    return state.status;
  },
  get version() {
    return state.version;
  },
  get progress() {
    return state.progress;
  },

  /** Boot path. `settings` is the get_settings map (may be empty when the
   * boot fetch failed — checks then start from a clean slate). */
  init(settings: Record<string, string>) {
    if (started) return;
    started = true;
    dismissed = settings.update_dismissed ?? "";
    lastCheck = Number(settings.update_last_check) || 0;
    const tick = () => {
      if (Date.now() - lastCheck >= CHECK_EVERY_MS) void checkNow();
    };
    setTimeout(() => {
      tick();
      setInterval(tick, TICK_MS);
    }, FIRST_TICK_MS);
  },

  /** "Update": stream the installer into memory (nothing touches disk, so a
   * quit before restarting leaves no junk — the banner just comes back). */
  async download() {
    if (!update || state.status === "downloading") return;
    state.status = "downloading";
    state.progress = 0;
    let total = 0;
    let got = 0;
    try {
      await update.download((event) => {
        if (event.event === "Started") {
          total = event.data.contentLength ?? 0;
        } else if (event.event === "Progress") {
          got += event.data.chunkLength;
          state.progress = total > 0 ? got / total : -1;
        }
      });
      state.status = "ready";
    } catch {
      state.status = "error";
    }
  },

  /** "Restart": passive NSIS install. The plugin exits this process and the
   * installer relaunches Skim — code after install() never runs on Windows. */
  async restart() {
    if (!update) return;
    // One-shot flag for the Rust side: the installer relaunches with the old
    // process args (possibly autostart's --minimized), but the user clicked
    // "Restart" and expects the window back.
    await api.setSetting("update_relaunch", "1").catch(() => {});
    try {
      await update.install();
    } catch {
      state.status = "error";
    }
  },

  /** "×": never offer this version again; a newer release shows up anew. */
  dismiss() {
    if (!state.version) return;
    dismissed = state.version;
    void api.setSetting("update_dismissed", dismissed).catch(() => {});
    void update?.close();
    update = null;
    state.version = null;
    state.status = "idle";
  },
};
