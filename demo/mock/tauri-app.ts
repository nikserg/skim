// Mock of `@tauri-apps/api/app`.
export async function getVersion(): Promise<string> {
  return "demo";
}
export async function getName(): Promise<string> {
  return "Skim";
}
export async function getTauriVersion(): Promise<string> {
  return "2.0.0";
}
