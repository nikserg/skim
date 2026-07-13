<script lang="ts">
  // AI Recap: a catch-up digest of the folder's unread mail. Occupies the
  // reading pane while open; covered messages are marked read on success.
  import { aiStream, api, type Citation } from "../lib/api";
  import { t } from "../lib/i18n/index.svelte";
  import { mail } from "../lib/stores/mail.svelte";
  import { ui } from "../lib/stores/ui.svelte";

  let status = $state<"scanning" | "streaming" | "done" | "error">("scanning");
  let text = $state("");
  let citations = $state<Citation[]>([]);
  let progress = $state<{ current: number; total: number } | null>(null);
  let markedCount = $state(0);
  let cancel: (() => void) | null = null;

  $effect(() => {
    const folderId = mail.selectedFolderId;
    if (folderId === null) return;
    cancel = aiStream(
      "ai_recap",
      { folderId },
      {
        progress: (current, total) => (progress = { current, total }),
        delta: (chunk) => {
          status = "streaming";
          progress = null;
          text += chunk;
        },
        done: (cited) => {
          status = "done";
          citations = cited;
          markRead(cited);
        },
        error: (code, message) => {
          status = "error";
          text = code === "ai_key" ? t("ai.needs_key") : message;
        },
      },
    );
    return () => cancel?.();
  });

  function markRead(cited: Citation[]) {
    const ids = cited.map((c) => c.messageId);
    if (ids.length === 0) return;
    markedCount = ids.length;
    for (const c of cited) {
      if (c.threadId !== null) mail.patchThreadRow(c.threadId, { isRead: true });
    }
    void api.markRead(ids, true);
  }

  async function openCitation(c: Citation) {
    if (c.folderId !== mail.selectedFolderId) {
      await mail.selectFolder(c.folderId);
    }
    if (c.threadId !== null) mail.selectedThreadId = c.threadId;
    ui.closeRecap();
  }
</script>

<section class="recap">
  <header class="head">
    <span class="title">✦ {t("ai.recap_title")}</span>
    <button class="close" onclick={() => ui.closeRecap()} aria-label={t("settings.close")}>
      <svg width="11" height="11" viewBox="0 0 10 10"><path d="M0 0L10 10M10 0L0 10" stroke="currentColor" stroke-width="1.2" /></svg>
    </button>
  </header>

  <div class="body">
    {#if status === "scanning"}
      <div class="progress">
        <span class="spinner"></span>
        {t("ai.recap_reading")}
        {#if progress}{progress.current}/{progress.total}{/if}
      </div>
    {:else if status === "error"}
      <div class="error">{text}</div>
    {:else}
      <div class="text">{text}</div>
      {#if status === "done"}
        {#if markedCount > 0}
          <div class="marked microlabel">✓ {t("ai.recap_marked", { n: markedCount })}</div>
        {/if}
        {#if citations.length > 0}
          <div class="sources">
            <span class="microlabel">{t("ai.sources")} · {citations.length}</span>
            <div class="chips">
              {#each citations as c (c.index)}
                <button class="chip" onclick={() => openCitation(c)}>
                  <span class="index">{c.index}</span>
                  {c.subject || c.from}
                </button>
              {/each}
            </div>
          </div>
        {/if}
      {/if}
    {/if}
  </div>
</section>

<style>
  .recap {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    background: var(--surface);
  }
  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 18px 24px 12px;
    border-bottom: 1px solid var(--accent-dim);
  }
  .title {
    color: var(--accent);
    font-weight: 700;
    font-size: 14px;
  }
  .close {
    width: 28px;
    height: 28px;
    display: grid;
    place-items: center;
    border-radius: var(--radius-s);
    color: var(--text-dim);
  }
  .close:hover {
    background: var(--hover);
    color: var(--text);
  }
  .body {
    flex: 1;
    overflow-y: auto;
    padding: 20px 24px;
    display: flex;
    flex-direction: column;
    gap: 16px;
  }
  .progress {
    display: flex;
    align-items: center;
    gap: 10px;
    color: var(--accent);
    font-size: 13px;
  }
  .spinner {
    width: 13px;
    height: 13px;
    border: 2px solid var(--accent-dim);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
    flex-shrink: 0;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  .text {
    font-size: 14px;
    line-height: 1.65;
    white-space: pre-wrap;
    user-select: text;
    max-width: 640px;
  }
  .marked {
    color: var(--success);
  }
  .error {
    color: var(--danger);
    font-size: 13px;
  }
  .sources {
    display: flex;
    flex-direction: column;
    gap: 8px;
    max-width: 640px;
  }
  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    max-width: 100%;
    padding: 5px 11px 5px 6px;
    border: 1px solid var(--accent-dim);
    border-radius: 999px;
    font-size: 12px;
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .chip:hover {
    background: var(--accent-soft);
  }
  .index {
    width: 17px;
    height: 17px;
    border-radius: 50%;
    background: var(--accent);
    color: var(--on-accent);
    display: grid;
    place-items: center;
    font-size: 10.5px;
    font-weight: 700;
    flex-shrink: 0;
  }
</style>
