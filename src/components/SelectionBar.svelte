<script lang="ts">
  import { bulkAct, bulkMove } from "../lib/bulk";
  import { t } from "../lib/i18n/index.svelte";
  import { mail } from "../lib/stores/mail.svelte";

  const count = $derived(mail.selectionCount);
  // One decision instead of two buttons: anything unread means "mark read".
  const markRead = $derived(mail.selectedRows.some((r) => !r.isRead));
  // IMAP cannot MOVE between accounts, so a mixed selection has no valid
  // destination — say why rather than opening a picker that must come up empty.
  const crossAccount = $derived(mail.selectionSpansAccounts);
  const allLoaded = $derived(count > 0 && count === mail.threads.length);
</script>

<div class="bar">
  <span class="count">{t("select.count", { n: count })}</span>
  <div class="tools">
    {#if !allLoaded}
      <button class="link" onclick={() => mail.selectAllLoaded()}>{t("select.all")}</button>
    {/if}
    <button class="tool" onclick={() => bulkAct("archive")} title={t("reading.archive")} aria-label={t("reading.archive")}>
      <svg width="15" height="15" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.2"><path d="M2 3h12v3H2V3zm1 3v7h10V6M6.5 9h3" /></svg>
    </button>
    <button class="tool" onclick={() => bulkAct("delete")} title={t("reading.delete")} aria-label={t("reading.delete")}>
      <svg width="15" height="15" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.2"><path d="M3 4h10M6.5 4V2.5h3V4M4.5 4l.5 9.5h6l.5-9.5M6.7 6.5v5M9.3 6.5v5" /></svg>
    </button>
    <button class="tool" onclick={() => bulkAct("spam")} title={t("reading.spam")} aria-label={t("reading.spam")}>
      <svg width="15" height="15" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.2"><path d="M5.4 1.8h5.2l3.6 3.6v5.2l-3.6 3.6H5.4L1.8 10.6V5.4L5.4 1.8z" /><path d="M8 4.6v4M8 11.1v.1" /></svg>
    </button>
    <button
      class="tool"
      onclick={() => void bulkMove()}
      disabled={crossAccount}
      title={crossAccount ? t("select.one_mailbox") : t("reading.move")}
      aria-label={t("reading.move")}
    >
      <svg width="15" height="15" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.2" stroke-linejoin="round"><path d="M1.5 3.5h4l1.5 2h7.5v7h-13v-9z" /><path d="M8 7.5v3.5M6.4 9.4L8 11l1.6-1.6" /></svg>
    </button>
    <button
      class="tool"
      onclick={() => bulkAct("read")}
      title={markRead ? t("reading.mark_read") : t("reading.mark_unread")}
      aria-label={markRead ? t("reading.mark_read") : t("reading.mark_unread")}
    >
      {#if markRead}
        <!-- Open envelope: click to mark read. -->
        <svg width="15" height="15" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.2"><path d="M2 6.5l6-4 6 4v6a1 1 0 0 1-1 1H3a1 1 0 0 1-1-1v-6z" /><path d="M2 6.5l6 4.5 6-4.5" /></svg>
      {:else}
        <!-- Sealed envelope: click to mark unread. -->
        <svg width="15" height="15" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.2"><rect x="2" y="3.5" width="12" height="9" rx="1" /><path d="M2 5l6 4.5L14 5" /></svg>
      {/if}
    </button>
    <button class="tool" onclick={() => mail.clearSelection()} title={t("select.clear")} aria-label={t("select.clear")}>
      <svg width="15" height="15" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.2" stroke-linecap="round"><path d="M4 4l8 8M12 4l-8 8" /></svg>
    </button>
  </div>
</div>

<style>
  .bar {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    min-width: 0;
  }
  .count {
    font-size: 13px;
    font-weight: 600;
    color: var(--text);
    white-space: nowrap;
  }
  .tools {
    display: flex;
    align-items: center;
    gap: 2px;
    margin-left: auto;
  }
  .link {
    font-size: 11.5px;
    color: var(--text-dim);
    padding: 3px 6px;
    border-radius: 5px;
    white-space: nowrap;
  }
  .link:hover {
    color: var(--text);
    background: var(--hover);
  }
  .tool {
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 5px;
    border-radius: 5px;
    color: var(--text-dim);
  }
  .tool:hover:not(:disabled) {
    color: var(--text);
    background: var(--hover);
  }
  .tool:disabled {
    opacity: 0.35;
  }
</style>
