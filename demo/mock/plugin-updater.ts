// Mock of `@tauri-apps/plugin-updater` — the demo never offers updates.

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

export async function check(): Promise<Update | null> {
  return null;
}
