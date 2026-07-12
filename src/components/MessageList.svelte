<script lang="ts">
  import { t } from "../lib/i18n/index.svelte";
  import { mail } from "../lib/stores/mail.svelte";
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

  let rowsEl: HTMLDivElement | undefined = $state();

  function onScroll() {
    if (!rowsEl) return;
    if (rowsEl.scrollTop + rowsEl.clientHeight > rowsEl.scrollHeight - 400) {
      if (mail.threads.length >= 100) void mail.loadMoreThreads();
    }
  }
</script>

<section class="list">
  <header class="head">
    <h1>{title}</h1>
    {#if unread > 0}
      <span class="microlabel">{t("list.unread", { n: unread })}</span>
    {/if}
  </header>
  <div class="rows" bind:this={rowsEl} onscroll={onScroll}>
    {#if mail.threads.length === 0 && !mail.threadsLoading}
      <div class="empty">
        {mail.syncState === "syncing" ? t("sync.syncing") : t("list.empty")}
      </div>
    {:else}
      {#each mail.threads as thread (thread.id)}
        <MessageRow
          {thread}
          selected={mail.selectedThreadId === thread.id}
          onselect={(id) => (mail.selectedThreadId = id)}
        />
      {/each}
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
    align-items: baseline;
    justify-content: space-between;
    padding: 18px 16px 12px;
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
