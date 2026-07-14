// Mock of `@tauri-apps/api/event`. No backend fires events in the demo, so
// listeners simply never receive anything (and unlisten is a no-op).
export async function listen<T = unknown>(
  _event: string,
  _handler: (e: { payload: T }) => void,
): Promise<() => void> {
  return () => {};
}

export async function once<T = unknown>(
  _event: string,
  _handler: (e: { payload: T }) => void,
): Promise<() => void> {
  return () => {};
}

export async function emit(_event: string, _payload?: unknown): Promise<void> {}
export async function emitTo(_target: string, _event: string, _payload?: unknown): Promise<void> {}
