// Writes text to the system clipboard. Returns false if the browser refused —
// the caller decides whether to surface anything. WebView2 supports the
// Clipboard API, so no execCommand fallback is needed.
export async function copyText(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    return false;
  }
}
