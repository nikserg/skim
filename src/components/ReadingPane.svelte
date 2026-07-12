<script lang="ts">
  import { t } from "../lib/i18n/index.svelte";
  import { mockAiSummary, mockMessage } from "../lib/mock";
  import { ui } from "../lib/stores/ui.svelte";

  const message = $derived(ui.selectedThreadId === 1 ? mockMessage : null);

  function initial(name: string | null): string {
    return (name ?? "?").charAt(0).toUpperCase();
  }

  function formatFull(unix: number): string {
    return new Date(unix * 1000).toLocaleString(undefined, {
      hour: "numeric",
      minute: "2-digit",
    });
  }
</script>

<section class="pane">
  {#if !message}
    <div class="placeholder">
      <div class="ghost">✉</div>
      {t("reading.no_selection")}
    </div>
  {:else}
    <div class="scroll">
      <h1 class="subject">{message.subject}</h1>

      <div class="meta">
        <span class="avatar">{initial(message.from.name)}</span>
        <div class="who">
          <div class="from">
            {message.from.name}
            <span class="addr">&lt;{message.from.addr}&gt;</span>
          </div>
          <div class="microlabel">
            {t("reading.to_me")} · {formatFull(message.date)}
          </div>
        </div>
        <div class="quick">
          <button title={t("reading.archive")}>
            <svg width="15" height="15" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.2"><path d="M2 3h12v3H2V3zm1 3v7h10V6M6.5 9h3" /></svg>
          </button>
          <button title={t("reading.delete")}>
            <svg width="15" height="15" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.2"><path d="M3 4h10M6.5 4V2.5h3V4M4.5 4l.5 9.5h6l.5-9.5M6.7 6.5v5M9.3 6.5v5" /></svg>
          </button>
          <button title={message.isStarred ? t("reading.unstar") : t("reading.star")}>
            <svg width="15" height="15" viewBox="0 0 16 16" fill={message.isStarred ? "currentColor" : "none"} stroke="currentColor" stroke-width="1.2"><path d="M8 1.5l2 4.1 4.5.6-3.3 3.2.8 4.5L8 11.8l-4 2.1.8-4.5L1.5 6.2 6 5.6 8 1.5z" /></svg>
          </button>
        </div>
      </div>

      <div class="ai-summary">
        <div class="ai-label microlabel">{t("ai.summary")}</div>
        {mockAiSummary}
      </div>

      <div class="body" style="user-select: text; cursor: text;">
        {message.bodyText}
      </div>
    </div>

    <footer class="actions">
      <div class="ai-actions">
        <button class="ai-btn">✦ {t("ai.draft_reply")}</button>
        <button class="ai-btn">{t("ai.summarize")}</button>
        <button class="ai-btn">{t("ai.ask_about")}</button>
      </div>
      <div class="mail-actions">
        <button class="plain">{t("reading.reply")}</button>
        <span class="sep">·</span>
        <button class="plain">{t("reading.forward")}</button>
      </div>
    </footer>
  {/if}
</section>

<style>
  .pane {
    flex: 1;
    display: flex;
    flex-direction: column;
    background: var(--surface);
    min-width: 0;
  }

  .placeholder {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 12px;
    color: var(--text-faint);
    font-size: 13px;
  }
  .ghost {
    font-size: 28px;
    opacity: 0.4;
  }

  .scroll {
    flex: 1;
    overflow-y: auto;
    padding: 28px 36px;
    max-width: 780px;
  }

  .subject {
    font-size: 21px;
    font-weight: 800;
    letter-spacing: -0.02em;
    line-height: 1.25;
  }

  .meta {
    display: flex;
    align-items: center;
    gap: 12px;
    margin-top: 20px;
  }
  .avatar {
    width: 36px;
    height: 36px;
    border-radius: 50%;
    background: var(--selected);
    display: grid;
    place-items: center;
    font-weight: 700;
    font-size: 14px;
    flex-shrink: 0;
  }
  .who {
    flex: 1;
    min-width: 0;
  }
  .from {
    font-weight: 600;
    font-size: 13.5px;
  }
  .addr {
    color: var(--text-faint);
    font-weight: 400;
  }
  .quick {
    display: flex;
    gap: 2px;
  }
  .quick button {
    width: 30px;
    height: 30px;
    display: grid;
    place-items: center;
    border-radius: var(--radius-s);
    color: var(--text-dim);
  }
  .quick button:hover {
    background: var(--hover);
    color: var(--text);
  }

  .ai-summary {
    margin-top: 20px;
    padding: 14px 16px;
    border-radius: var(--radius-m);
    background: var(--accent-soft);
    color: var(--text);
    font-size: 13.5px;
    line-height: 1.5;
  }
  .ai-label {
    color: var(--accent);
    margin-bottom: 6px;
  }

  .body {
    margin-top: 24px;
    font-size: 14px;
    line-height: 1.65;
    white-space: pre-wrap;
    color: var(--text);
  }

  .actions {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 12px 36px;
    border-top: 1px solid var(--hairline);
  }
  .ai-actions {
    display: flex;
    gap: 8px;
  }
  .ai-btn {
    padding: 6px 12px;
    border-radius: 999px;
    border: 1px solid var(--accent-dim);
    color: var(--accent);
    font-size: 12.5px;
    font-weight: 600;
  }
  .ai-btn:hover {
    background: var(--accent-soft);
  }
  .mail-actions {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--text-dim);
    font-size: 13px;
  }
  .plain:hover {
    color: var(--text);
  }
  .sep {
    color: var(--text-faint);
  }
</style>
