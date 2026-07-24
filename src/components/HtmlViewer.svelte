<script lang="ts">
  // Renders sanitized email HTML inside a sandboxed iframe (no scripts) with
  // a strict CSP. Links open in the system browser; height tracks content.
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { ui } from "../lib/stores/ui.svelte";
  import { t } from "../lib/i18n/index.svelte";
  import type { LinkFlag } from "../lib/types";

  let {
    html,
    security = null,
    onHoverUrl,
  }: {
    html: string;
    /** Per-link phishing verdicts from the backend, keyed by raw href. */
    security?: LinkFlag[] | null;
    /** Reports the real destination under the cursor (null = left links). */
    onHoverUrl?: (url: string | null) => void;
  } = $props();

  let iframe: HTMLIFrameElement | undefined = $state();
  let height = $state(120);
  let resizeObs: ResizeObserver | null = null;
  let observedDoc: Document | null = null;

  // The backend matched verdicts against the raw href attribute, so lookups
  // here must use getAttribute("href") too — never .href, which normalizes.
  const flags = $derived(new Map((security ?? []).map((f) => [f.href, f])));

  // Click-time gate for a flagged link: tiny popover at the click point with
  // the real host and the reason; nothing opens until "Open anyway".
  let gate = $state<{ flag: LinkFlag; x: number; y: number } | null>(null);
  let gateEl: HTMLDivElement | undefined = $state();

  $effect(() => {
    if (!gate) return;
    gateEl?.focus();
    const dismiss = () => (gate = null);
    // Capture phase + stopPropagation: while the gate is open, Escape closes
    // only the gate — the app's own window-level Escape (which deselects the
    // whole message) must not see the event.
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        dismiss();
      }
    };
    // The reading pane scrolls in the parent document; a fixed popover would
    // detach from its link, so any scroll closes it. Clicks land either in
    // the parent (pointerdown here) or inside the iframe (its own handler
    // replaces or reopens the gate).
    window.addEventListener("keydown", onKey, true);
    window.addEventListener("pointerdown", dismiss);
    window.addEventListener("scroll", dismiss, true);
    return () => {
      window.removeEventListener("keydown", onKey, true);
      window.removeEventListener("pointerdown", dismiss);
      window.removeEventListener("scroll", dismiss, true);
    };
  });

  function openGateLink() {
    const href = gate?.flag.href;
    gate = null;
    if (href) void openUrl(href);
  }

  // Attach measurement as soon as the srcdoc document's DOM exists — do NOT
  // wait for the iframe's `load` event. `load` fires only once every remote
  // image *and* tracking pixel has settled, so a single slow or stalled pixel
  // (common in marketing mail) would leave the message frozen at its initial
  // height with an inner scrollbar. The ResizeObserver set up here then tracks
  // later reflow (images arriving, fonts settling) and grows the frame to fit.
  $effect(() => {
    void srcdoc; // re-run when the rendered document is replaced
    const el = iframe;
    if (!el) return;
    let raf = 0;
    let tries = 0;
    const poll = () => {
      const doc = el.contentDocument;
      // contentDocument is briefly the initial empty about:blank before the
      // srcdoc document swaps in; wait for the real, populated one.
      if (doc && doc !== observedDoc && doc.body && doc.body.childElementCount > 0) {
        setupDoc(doc);
        return;
      }
      if (tries++ < 300) raf = requestAnimationFrame(poll);
    };
    poll();
    return () => cancelAnimationFrame(raf);
  });

  $effect(() => () => resizeObs?.disconnect());

  // Emails that carry their own colors (inline color/background, bgcolor,
  // <font color>) assume a light page background they never declare. Honor
  // that by rendering them on an explicit white canvas even in dark theme —
  // forcing the dark canvas produced dark-on-dark, invisible text. Plain-text
  // and colorless emails still follow the app theme.
  const ownColors = $derived(hasOwnColors(html));
  // The iframe document can't see the app's CSS variables, so resolve --surface
  // here and paint it in. Transparency looks like the obvious answer but isn't:
  // under `color-scheme: dark` the UA paints its own opaque canvas and ignores a
  // transparent root, so dark themes would keep the mismatched block.
  const surface = $derived.by(() => {
    void ui.temperature; // re-resolve when the palette changes
    void ui.lightness;
    return (
      getComputedStyle(document.documentElement).getPropertyValue("--surface").trim() ||
      "#ffffff"
    );
  });
  const srcdoc = $derived(
    buildDoc(html, ui.effective === "dark" && !ownColors, ownColors, surface),
  );

  function hasOwnColors(body: string): boolean {
    return (
      /(?:^|[;\s"'])(?:background-color|background|color)\s*:/i.test(body) ||
      /\bbgcolor\s*=/i.test(body) ||
      /<font\b[^>]*\bcolor\s*=/i.test(body)
    );
  }

  function buildDoc(body: string, dark: boolean, ownColors: boolean, surface: string): string {
    // The default canvas follows the app theme: it's painted with the live
    // --surface, so the message blends into the pane in every palette. It used
    // to be hardcoded (#ffffff / #141418) — those happened to equal cold-light
    // and cold-dark's surface, so the drift only became visible once the warm
    // palette landed and every email turned into a mismatched block.
    //
    // Emails that bring their own (inline) colors still get an explicit white
    // page, since that's the background they were written against — the
    // sanitizer already limits what CSS survives.
    const colors = dark
      ? {
          scheme: "dark",
          bg: surface,
          text: "#ececef",
          link: "#8ab4f8",
          quoteBorder: "#3a3a42",
          quoteText: "#a3a3ab",
        }
      : {
          scheme: "light",
          bg: ownColors ? "#ffffff" : surface,
          text: "#17171b",
          link: "#1a56c4",
          quoteBorder: "#dddddd",
          quoteText: "#555555",
        };
    return `<!doctype html><html><head>
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; img-src http://skim-cid.localhost data: https: http:; style-src 'unsafe-inline'">
<style>
  :root { color-scheme: ${colors.scheme}; }
  /* The background goes on the root too, not just body: the canvas takes its
     colour from html first, and color-scheme makes the UA paint an opaque
     default there — which would defeat a transparent body. */
  html, body { margin: 0; padding: 0; background: ${colors.bg}; }
  body {
    font-family: 'Hanken Grotesk', 'Segoe UI', sans-serif;
    font-size: 14px; line-height: 1.6; color: ${colors.text};
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

  function setupDoc(doc: Document) {
    observedDoc = doc;
    const measure = () => {
      const h = Math.min(Math.max(doc.documentElement.scrollHeight, 40) + 8, 20000);
      // Guard against feedback loops with percentage-height emails.
      if (Math.abs(h - height) > 1) height = h;
    };
    measure();
    // Content reflows as it settles — web fonts, table-based layouts relaxing,
    // async images. Track every layout change instead of measuring once, so the
    // iframe never gets its own scrollbar.
    resizeObs?.disconnect();
    const obs = new ResizeObserver(measure);
    obs.observe(doc.documentElement);
    if (doc.body) obs.observe(doc.body);
    resizeObs = obs;
    doc.fonts?.ready.then(measure).catch(() => {});
    // Images loading later change the height.
    for (const img of Array.from(doc.images)) {
      img.addEventListener("load", measure);
      img.addEventListener("error", measure);
    }
    // The message body is never editable, so kill the WebView2 context menu
    // here too — the parent document's handler can't reach into the iframe.
    doc.addEventListener("contextmenu", (e) => e.preventDefault());
    doc.addEventListener("click", (e) => {
      const target = (e.target as HTMLElement | null)?.closest("a");
      if (!target) {
        gate = null;
        return;
      }
      e.preventDefault();
      const href = target.getAttribute("href");
      if (!href || !/^https?:/i.test(href)) return;
      const flag = flags.get(href);
      if (!flag) {
        gate = null;
        void openUrl(href);
        return;
      }
      // Click coords are iframe-local; the iframe never scrolls internally
      // (its height tracks content), so iframe rect + coords = viewport.
      const rect = iframe?.getBoundingClientRect();
      const x = (rect?.left ?? 0) + e.clientX;
      const y = (rect?.top ?? 0) + e.clientY;
      gate = {
        flag,
        x: Math.max(8, Math.min(x, window.innerWidth - 336)),
        y: Math.max(8, Math.min(y + 8, window.innerHeight - 180)),
      };
    });
    // Hover preview: report the real destination to the status bar. Entering
    // anything that isn't a link (or leaving the document) clears it.
    doc.addEventListener("mouseover", (e) => {
      const a = (e.target as HTMLElement | null)?.closest("a");
      const href = a?.getAttribute("href");
      onHoverUrl?.(href && /^(https?|mailto):/i.test(href) ? href : null);
    });
    doc.addEventListener("mouseout", (e) => {
      if (!e.relatedTarget) onHoverUrl?.(null);
    });
  }

  // Backstop for the poll above: a body with no element children (an empty
  // message) never trips the childElementCount check, but such messages carry
  // no slow resources, so `load` fires promptly and wires things up here.
  function onLoad() {
    const doc = iframe?.contentDocument;
    if (doc && doc !== observedDoc) setupDoc(doc);
  }
</script>

<iframe
  bind:this={iframe}
  title={t("a11y.message")}
  sandbox="allow-same-origin"
  srcdoc={srcdoc}
  onload={onLoad}
  style="height: {height}px"
></iframe>

{#if gate}
  <div
    bind:this={gateEl}
    class="gate"
    role="alertdialog"
    aria-label={t("security.link_gate_title", { host: gate.flag.host })}
    tabindex="-1"
    style="left: {gate.x}px; top: {gate.y}px"
    onpointerdown={(e) => e.stopPropagation()}
  >
    <div class="gate-title">{t("security.link_gate_title", { host: gate.flag.host })}</div>
    {#each gate.flag.reasons as r (r.code)}
      <div class="gate-reason">{t(`security.link.${r.code}`, { param: r.param ?? "" })}</div>
    {/each}
    <div class="gate-actions">
      <button class="gate-open" onclick={openGateLink}>{t("security.open_anyway")}</button>
    </div>
  </div>
{/if}

<style>
  iframe {
    width: 100%;
    border: none;
    display: block;
    background: var(--surface);
    border-radius: var(--radius-m);
  }

  /* Link-safety gate. Neutral surface; danger tone only where it matters —
     this is a security affordance, not an AI feature, so no accent. */
  .gate {
    position: fixed;
    z-index: 40;
    max-width: 328px;
    padding: 10px 12px;
    background: var(--surface-raised);
    border: 1px solid var(--hairline-strong);
    border-radius: var(--radius-m);
    box-shadow: 0 8px 28px rgba(0, 0, 0, 0.18);
    outline: none;
  }
  .gate-title {
    font-size: 13px;
    font-weight: 600;
    color: var(--danger);
    overflow-wrap: anywhere;
  }
  .gate-reason {
    margin-top: 4px;
    font-size: 12.5px;
    color: var(--text-dim);
  }
  .gate-actions {
    margin-top: 8px;
    display: flex;
    justify-content: flex-end;
  }
  .gate-open {
    font-size: 12.5px;
    padding: 4px 10px;
    border: 1px solid var(--hairline-strong);
    border-radius: var(--radius-s);
    background: none;
    color: var(--text-dim);
    cursor: pointer;
  }
  .gate-open:hover {
    color: var(--text);
    border-color: var(--danger);
  }
</style>
