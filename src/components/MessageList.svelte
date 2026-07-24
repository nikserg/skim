<script lang="ts">
  import { t } from "../lib/i18n/index.svelte";
  import { ai } from "../lib/stores/ai.svelte";
  import { mail } from "../lib/stores/mail.svelte";
  import { ui } from "../lib/stores/ui.svelte";
  import MessageRow from "./MessageRow.svelte";

  const title = $derived.by(() => {
    const f = mail.selectedFolder;
    if (!f) return t("nav.inbox");
    const roleKey: Record<string, string> = {
      inbox: "nav.inbox",
      starred: "nav.starred",
      sent: "nav.sent",
      drafts: "nav.drafts",
      archive: "nav.archive",
      trash: "nav.trash",
      junk: "nav.junk",
    };
    return f.role && roleKey[f.role] ? t(roleKey[f.role]) : f.displayName;
  });

  const unread = $derived(mail.selectedFolder?.unreadCount ?? 0);

  // AI Recap is an inbox catch-up: only there, only with unread mail.
  const recapAvailable = $derived(
    mail.selectedFolder?.role === "inbox" && unread > 0 && ai.keyPresent,
  );

  function openRecap() {
    // The digest occupies the reading pane — clear the selection.
    mail.selectedThreadId = null;
    ui.openRecap();
  }

  let rowsEl: HTMLDivElement | undefined = $state();

  // Windowed rendering: only the rows near the viewport get live components,
  // so a folder with thousands of messages doesn't hold thousands of DOM
  // nodes. Rows are fixed-height (three nowrap lines), which keeps the
  // arithmetic exact; the real height is measured off the first rendered row.
  const OVERSCAN = 8;
  let scrollTop = $state(0);
  let viewH = $state(0);
  let rowH = $state(76);

  const start = $derived(Math.max(0, Math.floor(scrollTop / rowH) - OVERSCAN));
  const end = $derived(
    Math.min(mail.threads.length, Math.ceil((scrollTop + viewH) / rowH) + OVERSCAN),
  );
  const visible = $derived(mail.threads.slice(start, end));

  // Measure the row height for the windowing arithmetic. Rows are NOT perfectly
  // uniform — one with an empty snippet is a line shorter — so as different rows
  // scroll into the measured slot, a two-way `!= rowH` update would flip rowH
  // between e.g. 68 and 76 forever: that feeds `visible`, which re-runs this
  // effect, which re-measures… Svelte caps that as `effect_update_depth_exceeded`
  // and then stops applying updates — the whole UI freezes (list stops loading,
  // clicks stop opening) until a full restart. So only ever ratchet *up* to the
  // tallest row seen; it converges in a couple of steps and can't oscillate.
  // Overestimating a little is harmless — overscan keeps the viewport full.
  $effect(() => {
    void visible;
    const el = rowsEl?.querySelector<HTMLElement>("button.row");
    if (el && el.offsetHeight > rowH) rowH = el.offsetHeight;
  });

  // A new folder (or grouping mode) is a new list — start it at the top.
  $effect(() => {
    void mail.selectedFolderId;
    void mail.groupThreads;
    if (rowsEl) rowsEl.scrollTop = 0;
    scrollTop = 0;
  });

  function onScroll() {
    if (!rowsEl) return;
    scrollTop = rowsEl.scrollTop;
    if (rowsEl.scrollTop + rowsEl.clientHeight > rowsEl.scrollHeight - 400) {
      if (mail.threads.length >= 100) void mail.loadMoreThreads();
    }
  }
</script>

<section class="list">
  <header class="head">
    <h1>{title}</h1>
    <div class="head-right">
      {#if recapAvailable}
        <button class="recap-chip" onclick={openRecap}>✦ {t("ai.recap")}</button>
      {/if}
      {#if unread > 0}
        <span class="microlabel">{t("list.unread", { n: unread })}</span>
      {/if}
      {#if mail.threads.length > 0}
        <span class="nav-hint" title="{t('shortcuts.next')} · {t('shortcuts.prev')}">
          <svg width="11" height="11" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1.3">
            <path d="M3.5 5L6 2.5 8.5 5" />
            <path d="M3.5 7L6 9.5 8.5 7" />
          </svg>
          <kbd>J</kbd><kbd>K</kbd>
        </span>
      {/if}
    </div>
  </header>
  <div class="rows" bind:this={rowsEl} bind:clientHeight={viewH} onscroll={onScroll}>
    {#if mail.threads.length === 0 && !mail.threadsLoading}
      <div class="empty">
        {mail.syncState === "syncing" ? t("sync.syncing") : t("list.empty")}
      </div>
    {:else}
      <div class="spacer" style="height: {start * rowH}px"></div>
      {#each visible as thread (thread.messageId ?? thread.id)}
        <MessageRow
          {thread}
          selected={mail.groupThreads
            ? mail.selectedThreadId === thread.id
            : mail.selectedMessageId === thread.messageId}
          onselect={() => {
            mail.selectedThreadId = thread.id;
            mail.selectedMessageId = thread.messageId ?? null;
          }}
        />
      {/each}
      <div class="spacer" style="height: {(mail.threads.length - end) * rowH}px"></div>
    {/if}
  </div>
</section>

<style>
  .list {
    width: var(--list-w);
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    border-right: 1px solid var(--hairline);
    background: var(--bg);
    min-width: 0;
  }
  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding: 18px 16px 12px;
  }
  .head-right {
    display: flex;
    align-items: center;
    gap: 10px;
    min-width: 0;
  }
  .nav-hint {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    color: var(--text-faint);
    flex-shrink: 0;
  }
  .nav-hint kbd {
    font-family: var(--font-mono);
    font-size: 10px;
    color: var(--text-faint);
  }
  /* Violet — an AI moment */
  .recap-chip {
    padding: 4px 11px;
    border-radius: 999px;
    border: 1px solid var(--accent-dim);
    color: var(--accent);
    font-size: 12px;
    font-weight: 600;
    white-space: nowrap;
  }
  .recap-chip:hover {
    background: var(--accent-soft);
  }
  /* Quiet-zine character moment: the chip sits slightly askew, like a stamp. */
  :global(:root[data-theme="warm-light"]) .recap-chip,
  :global(:root[data-theme="warm-dark"]) .recap-chip {
    border-radius: 3px;
    background: var(--surface);
    transform: rotate(-1.5deg);
  }
  @media (prefers-reduced-motion: reduce) {
    :global(:root[data-theme="warm-light"]) .recap-chip,
    :global(:root[data-theme="warm-dark"]) .recap-chip {
      transform: none;
    }
  }
  h1 {
    font-size: 17px;
    font-weight: 800;
    letter-spacing: -0.02em;
  }
  .rows {
    overflow-y: auto;
    flex: 1;
  }
  .empty {
    padding: 48px 16px;
    text-align: center;
    color: var(--text-faint);
    font-size: 13px;
  }
</style>
