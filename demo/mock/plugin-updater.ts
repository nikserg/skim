// Mock of `@tauri-apps/plugin-updater`. The demo never offers updates, unless
// localStorage "skimdemo.update" = "on" — then a fake release walks the banner
// through available → downloading → ready → installing without touching the
// machine. `installs` counts the real installs a run would have started, which
// is what makes "Restart" firing twice visible in a browser.

export type DownloadEvent =
  | { event: "Started"; data: { contentLength?: number } }
  | { event: "Progress"; data: { chunkLength: number } }
  | { event: "Finished" };

export interface Update {
  version: string;
  download(onEvent?: (event: DownloadEvent) => void): Promise<void>;
  install(): Promise<void>;
  close(): Promise<void>;
}

const offered = () => {
  try {
    return (globalThis as any).localStorage?.getItem("skimdemo.update") === "on";
  } catch {
    return false;
  }
};

const wait = (ms: number) => new Promise((done) => setTimeout(done, ms));

/** Read from the console to check the banner started exactly one install. */
export const installs = { count: 0 };

export async function check(): Promise<Update | null> {
  if (!offered()) return null;
  return {
    version: "9.9.9",
    async download(onEvent) {
      const total = 4_000_000;
      onEvent?.({ event: "Started", data: { contentLength: total } });
      for (let got = 0; got < total; got += total / 10) {
        await wait(120);
        onEvent?.({ event: "Progress", data: { chunkLength: total / 10 } });
      }
      onEvent?.({ event: "Finished" });
    },
    async install() {
      installs.count += 1;
      (globalThis as any).console?.log(`demo: install #${installs.count}`);
      // The real thing never returns — the installer kills the process.
      await new Promise(() => {});
    },
    async close() {},
  };
}
