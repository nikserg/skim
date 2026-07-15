import { openUrl } from "@tauri-apps/plugin-opener";

// Links in AI answers are rendered as <a class="md-link"> in the main webview
// (not a sandboxed iframe like mail), so a real navigation would replace the
// app. Intercept clicks and hand the href to the system browser instead.
function onAiLinkClick(ev: MouseEvent) {
  const a = (ev.target as HTMLElement | null)?.closest("a.md-link");
  if (!a) return;
  ev.preventDefault();
  const href = a.getAttribute("href");
  if (href && /^https?:/i.test(href)) void openUrl(href);
}

// Svelte action for any container that renders mdLite() output. Delegated
// imperatively (not a template onclick) so the container div stays free of
// the click-needs-keyboard a11y rule — the anchors are the targets.
export function aiLinks(node: HTMLElement) {
  node.addEventListener("click", onAiLinkClick);
  return { destroy: () => node.removeEventListener("click", onAiLinkClick) };
}
