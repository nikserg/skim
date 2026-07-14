// Mock of `@tauri-apps/plugin-autostart` (referenced from settings).
export async function enable(): Promise<void> {}
export async function disable(): Promise<void> {}
export async function isEnabled(): Promise<boolean> {
  return false;
}
