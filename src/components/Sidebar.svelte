<script lang="ts">
  import { api } from "../lib/api";
  import { t } from "../lib/i18n/index.svelte";
  import { mail } from "../lib/stores/mail.svelte";
  import { palette } from "../lib/stores/palette.svelte";
  import { ui } from "../lib/stores/ui.svelte";
  import Settings from "./settings/Settings.svelte";

  async function compose() {
    const draft = await api.createDraft();
    await api.openComposeWindow(draft.id);
  }

  const roleKey: Record<string, string> = {
    inbox: "nav.inbox",
    starred: "nav.starred",
    sent: "nav.sent",
    drafts: "nav.drafts",
    archive: "nav.archive",
    trash: "nav.trash",
    junk: "nav.junk",
    all: "nav.all_mail",
  };

  const roleIcon: Record<string, string> = {
    inbox: "M2 8l5-5h2l5 5v4a1 1 0 0 1-1 1H3a1 1 0 0 1-1-1V8zm0 0h3.5a2.5 2.5 0 0 0 5 0H14",
    starred:
      "M8 1.5l2 4.1 4.5.6-3.3 3.2.8 4.5L8 11.8l-4 2.1.8-4.5L1.5 6.2 6 5.6 8 1.5z",
    sent: "M14 2L2 7l4.5 2L8 14l6-12zM6.5 9L14 2",
    drafts: "M3 2h7l3 3v9H3V2zm7 0v3h3M5.5 8h5M5.5 11h5",
    archive: "M2 3h12v3H2V3zm1 3v7h10V6M6.5 9h3",
    trash: "M3 4h10M6.5 4V2.5h3V4M4.5 4l.5 9.5h6l.5-9.5M6.7 6.5v5M9.3 6.5v5",
    junk: "M8 2a6 6 0 1 0 0 12A6 6 0 0 0 8 2zM3.5 3.5l9 9",
  };

  const mainFolders = $derived(
    mail.folders.filter((f) => f.role !== null && f.role !== "all" && f.role !== "starred"),
  );
  const labels = $derived(mail.folders.filter((f) => f.role === null));
</script>

<nav class="sidebar">
  <button class="compose" onclick={compose}>
    <span class="plus">+</span>
    {t("nav.compose")}
  </button>

  <button class="search" onclick={() => palette.show()}>
    <svg width="13" height="13" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.4">
      <circle cx="7" cy="7" r="4.5" /><path d="M10.5 10.5L14 14" />
    </svg>
    {t("nav.search")}
    <kbd>Ctrl K</kbd>
  </button>

  <div class="section">
    {#each mainFolders as folder (folder.id)}
      <button
        class="item"
        class:selected={mail.selectedFolderId === folder.id}
        onclick={() => mail.selectFolder(folder.id)}
      >
        <svg width="15" height="15" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.2" stroke-linejoin="round">
          <path d={roleIcon[folder.role ?? "inbox"] ?? roleIcon.inbox} />
        </svg>
        <span class="name">{folder.role && roleKey[folder.role] ? t(roleKey[folder.role]) : folder.displayName}</span>
        {#if folder.unreadCount > 0}
          <span class="count">{folder.unreadCount}</span>
        {/if}
      </button>
    {/each}
  </div>

  {#if labels.length > 0}
    <div class="section">
      <div class="microlabel heading">{t("nav.labels")}</div>
      {#each labels as folder (folder.id)}
        <button
          class="item"
          class:selected={mail.selectedFolderId === folder.id}
          onclick={() => mail.selectFolder(folder.id)}
        >
          <span class="dot"></span>
          <span class="name">{folder.displayName}</span>
          {#if folder.unreadCount > 0}
            <span class="count">{folder.unreadCount}</span>
          {/if}
        </button>
      {/each}
    </div>
  {/if}

  <div class="footer">
    {#if mail.syncState === "syncing"}
      <div class="sync microlabel">
        <span class="spinner"></span>
        {t("sync.syncing")}
        {#if mail.syncProgress}
          {Math.round((mail.syncProgress.done / Math.max(1, mail.syncProgress.total)) * 100)}%
        {/if}
      </div>
    {:else if mail.syncState === "error"}
      <button class="sync error microlabel" onclick={() => mail.syncNow()} title={mail.syncMessage}>
        ⚠ {t("sync.error")}
      </button>
    {/if}
    <button class="item" onclick={() => ui.openSettings()}>
      <span class="avatar">{(mail.account?.email ?? "?").charAt(0).toUpperCase()}</span>
      <span class="name">{t("nav.settings")}</span>
    </button>
  </div>
</nav>

{#if ui.settingsOpen}
  <Settings onclose={() => ui.closeSettings()} />
{/if}

<style>
  .sidebar {
    width: var(--sidebar-w);
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    padding: 12px 10px;
    gap: 18px;
    border-right: 1px solid var(--hairline);
    background: var(--bg);
    overflow-y: auto;
  }

  .compose {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 9px 12px;
    border-radius: var(--radius-m);
    background: var(--text);
    color: var(--bg);
    font-weight: 600;
    font-size: 13.5px;
    transition: opacity 0.1s;
  }
  .compose:hover {
    opacity: 0.88;
  }
  .plus {
    font-size: 16px;
    line-height: 1;
    font-weight: 500;
  }

  .search {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 12px;
    border-radius: var(--radius-m);
    border: 1px solid var(--hairline-strong);
    color: var(--text-dim);
    font-size: 13px;
    margin-top: -8px;
  }
  .search:hover {
    background: var(--hover);
  }
  kbd {
    margin-left: auto;
    font-family: var(--font-mono);
    font-size: 10px;
    color: var(--text-faint);
  }

  .section {
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .heading {
    padding: 0 12px 6px;
  }

  .item {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 7px 12px;
    border-radius: var(--radius-s);
    color: var(--text-dim);
    font-size: 13.5px;
    text-align: left;
    width: 100%;
  }
  .item:hover {
    background: var(--hover);
    color: var(--text);
  }
  .item.selected {
    background: var(--selected);
    color: var(--text);
    font-weight: 600;
  }
  .name {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .count {
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--text-dim);
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    border: 1.5px solid var(--text-faint);
    margin: 0 3px;
  }

  .footer {
    margin-top: auto;
    display: flex;
    flex-direction: column;
    gap: 1px;
    border-top: 1px solid var(--hairline);
    padding-top: 10px;
  }
  .avatar {
    width: 18px;
    height: 18px;
    border-radius: 50%;
    background: var(--selected);
    display: grid;
    place-items: center;
    font-size: 10px;
    font-weight: 700;
  }
  .sync {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 12px;
  }
  .sync.error {
    color: var(--danger);
  }
  .spinner {
    width: 10px;
    height: 10px;
    border: 1.5px solid var(--text-faint);
    border-top-color: var(--text);
    border-radius: 50%;
    animation: spin 0.9s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
</style>
