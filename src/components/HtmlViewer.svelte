<script lang="ts">
  // Renders sanitized email HTML inside a sandboxed iframe (no scripts) with
  // a strict CSP. Links open in the system browser; height tracks content.
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { ui } from "../lib/stores/ui.svelte";

  let { html }: { html: string } = $props();

  let iframe: HTMLIFrameElement | undefined = $state();
  let height = $state(120);

  const srcdoc = $derived(buildDoc(html, ui.effective === "dark"));

  function buildDoc(body: string, dark: boolean): string {
    // The default canvas follows the app theme; emails that bring their own
    // (inline) colors keep them — the sanitizer already limits what CSS
    // survives.
    const colors = dark
      ? {
          scheme: "dark",
          bg: "#141418",
          text: "#ececef",
          link: "#8ab4f8",
          quoteBorder: "#3a3a42",
          quoteText: "#a3a3ab",
        }
      : {
          scheme: "light",
          bg: "#ffffff",
          text: "#17171b",
          link: "#1a56c4",
          quoteBorder: "#dddddd",
          quoteText: "#555555",
        };
    return `<!doctype html><html><head>
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; img-src http://skim-cid.localhost data: https: http:; style-src 'unsafe-inline'">
<style>
  :root { color-scheme: ${colors.scheme}; }
  html, body { margin: 0; padding: 0; }
  body {
    font-family: 'Hanken Grotesk', 'Segoe UI', sans-serif;
    font-size: 14px; line-height: 1.6; color: ${colors.text}; background: ${colors.bg};
    word-wrap: break-word; overflow-wrap: break-word;
  }
  img { max-width: 100%; height: auto; }
  img[src=""] { display: none; }
  a { color: ${colors.link}; }
  table { max-width: 100%; }
  blockquote { margin: 8px 0 8px 2px; padding-left: 12px; border-left: 2px solid ${colors.quoteBorder}; color: ${colors.quoteText}; }
  pre.skim-plain { white-space: pre-wrap; font: inherit; margin: 0; }
</style></head><body>${body}</body></html>`;
  }

  function onLoad() {
    const doc = iframe?.contentDocument;
    if (!doc) return;
    const measure = () => {
      height = Math.min(Math.max(doc.documentElement.scrollHeight, 40) + 8, 20000);
    };
    measure();
    // Images loading later change the height.
    for (const img of Array.from(doc.images)) {
      img.addEventListener("load", measure);
      img.addEventListener("error", measure);
    }
    doc.addEventListener("click", (e) => {
      const target = (e.target as HTMLElement | null)?.closest("a");
      if (target) {
        e.preventDefault();
        const href = target.getAttribute("href");
        if (href && /^https?:/i.test(href)) void openUrl(href);
      }
    });
  }
</script>

<iframe
  bind:this={iframe}
  title="Message"
  sandbox="allow-same-origin"
  srcdoc={srcdoc}
  onload={onLoad}
  style="height: {height}px"
></iframe>

<style>
  iframe {
    width: 100%;
    border: none;
    display: block;
    background: var(--surface);
    border-radius: var(--radius-m);
  }
</style>
