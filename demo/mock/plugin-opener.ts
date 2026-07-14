// Mock of `@tauri-apps/plugin-opener`. Links in the reading pane are inert in
// the demo (we don't want to pop the system browser mid-recording).
export async function openUrl(_url: string): Promise<void> {}
export async function openPath(_path: string): Promise<void> {}
export async function revealItemInDir(_path: string): Promise<void> {}
