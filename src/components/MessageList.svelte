<script lang="ts">
  import { t } from "../lib/i18n/index.svelte";
  import { mockThreads } from "../lib/mock";
  import { ui } from "../lib/stores/ui.svelte";
  import MessageRow from "./MessageRow.svelte";

  const unread = $derived(mockThreads.filter((th) => !th.isRead).length);
</script>

<section class="list">
  <header class="head">
    <h1>{t("nav.inbox")}</h1>
    <span class="microlabel">{t("list.unread", { n: unread })}</span>
  </header>
  <div class="rows">
    {#if mockThreads.length === 0}
      <div class="empty">{t("list.empty")}</div>
    {:else}
      {#each mockThreads as thread (thread.id)}
        <MessageRow
          {thread}
          selected={ui.selectedThreadId === thread.id}
          onselect={(id) => (ui.selectedThreadId = id)}
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
