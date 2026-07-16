<script lang="ts">
  import { api } from "../lib/api";
  import { t } from "../lib/i18n/index.svelte";
  import { ai } from "../lib/stores/ai.svelte";
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

  // "Отправленные" и "Черновики" содержат письма, написанные самим
  // пользователем, — счётчик непрочитанных для них бессмысленен.
  const noUnreadRoles = new Set(["sent", "drafts"]);
  const showsUnread = (role: string | null) => !(role && noUnreadRoles.has(role));

  const mainFolders = $derived(
    mail.folders.filter((f) => f.role !== null && f.role !== "all" && f.role !== "starred"),
  );
  const labels = $derived(mail.folders.filter((f) => f.role === null));
  const collapsed = $derived(ui.sidebarCollapsed);
</script>

<nav class="sidebar" class:collapsed={ui.sidebarCollapsed}>
  <div class="scroll">
    <button class="compose" onclick={compose} title={collapsed ? t("nav.compose") : undefined}>
      <span class="plus">+</span>
      <span class="name">{t("nav.compose")}</span>
      <kbd>Ctrl N</kbd>
    </button>

    <button class="search" onclick={() => palette.show()} title={collapsed ? t("nav.search") : undefined}>
      <svg width="13" height="13" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.4">
        <circle cx="7" cy="7" r="4.5" /><path d="M10.5 10.5L14 14" />
      </svg>
      <span class="name">{ai.keyPresent ? t("palette.placeholder_ai") : t("nav.search")}</span>
      <kbd>Ctrl K</kbd>
    </button>

    <div class="section">
      {#each mainFolders as folder (folder.id)}
        {@const name = folder.role && roleKey[folder.role] ? t(roleKey[folder.role]) : folder.displayName}
        <button
          class="item"
          class:selected={mail.selectedFolderId === folder.id}
          onclick={() => mail.selectFolder(folder.id)}
          title={collapsed ? name : undefined}
        >
          <svg width="15" height="15" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.2" stroke-linejoin="round">
            <path d={roleIcon[folder.role ?? "inbox"] ?? roleIcon.inbox} />
          </svg>
          <span class="name">{name}</span>
          {#if folder.unreadCount > 0 && showsUnread(folder.role)}
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
            title={collapsed ? folder.displayName : undefined}
          >
            <span class="dot"></span>
            <span class="initial" aria-hidden="true">{folder.displayName.charAt(0).toUpperCase()}</span>
            <span class="name">{folder.displayName}</span>
            {#if folder.unreadCount > 0}
              <span class="count">{folder.unreadCount}</span>
            {/if}
          </button>
        {/each}
      </div>
    {/if}
  </div>

  <div class="footer">
    {#if mail.syncState === "syncing"}
      <div class="sync microlabel">
        <span class="spinner"></span>
        <span class="name">
          {t("sync.syncing")}
          {#if mail.syncProgress}
            {Math.round((mail.syncProgress.done / Math.max(1, mail.syncProgress.total)) * 100)}%
          {/if}
        </span>
      </div>
    {:else if mail.syncState === "error"}
      <button class="sync error microlabel" onclick={() => mail.syncNow()} title={mail.syncMessage}>
        <span class="warn-icon">⚠</span>
        <span class="name">{t("sync.error")}</span>
      </button>
    {/if}
    <button class="item" onclick={() => ui.openSettings()} title={collapsed ? t("nav.settings") : undefined}>
      <span class="avatar">{(mail.account?.email ?? "?").charAt(0).toUpperCase()}</span>
      <span class="name">{t("nav.settings")}</span>
    </button>
  </div>

  <button
    class="toggle"
    onclick={() => ui.toggleSidebar()}
    title={collapsed ? t("nav.expand") : t("nav.collapse")}
    aria-label={collapsed ? t("nav.expand") : t("nav.collapse")}
  >
    <svg width="13" height="13" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">
      {#if collapsed}
        <path d="M6 4l4 4-4 4" />
      {:else}
        <path d="M10 4L6 8l4 4" />
      {/if}
    </svg>
  </button>
</nav>

{#if ui.settingsOpen}
  <Settings onclose={() => ui.closeSettings()} />
{/if}

<style>
  .sidebar {
    position: relative;
    width: var(--sidebar-w);
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    border-right: 1px solid var(--hairline);
    background: var(--bg);
    /* visible so the toggle FAB can protrude past the right edge; scrolling
       lives on the inner .scroll wrapper. */
    overflow: visible;
    transition: width 0.18s ease;
  }
  .sidebar.collapsed {
    width: 60px;
  }
  .scroll {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    overflow-x: hidden;
    display: flex;
    flex-direction: column;
    gap: 18px;
    padding: 12px 10px;
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
  .compose kbd {
    color: var(--bg);
    opacity: 0.55;
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
    /* <button> defaults to text-align:center; keep menu labels left-aligned
       (folder .item sets this too, but .compose/.search rely on it here). */
    text-align: left;
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
  /* First letter of a label — the icon stand-in for the collapsed rail, where
     an anonymous dot tells you nothing. Hidden in the expanded menu. */
  .initial {
    display: none;
    font-size: 13px;
    font-weight: 600;
  }

  .footer {
    display: flex;
    flex-direction: column;
    gap: 1px;
    border-top: 1px solid var(--hairline);
    padding: 10px 10px 12px;
  }

  /* Toggle FAB — a small round button straddling the right edge, revealed on
     hover (always available to keyboard via focus). */
  .toggle {
    position: absolute;
    right: -13px;
    bottom: 16px;
    width: 26px;
    height: 26px;
    display: grid;
    place-items: center;
    border-radius: 50%;
    background: var(--surface);
    border: 1px solid var(--hairline-strong);
    box-shadow: 0 2px 7px rgba(0, 0, 0, 0.18);
    color: var(--text-dim);
    opacity: 0;
    transition:
      opacity 0.12s ease,
      color 0.12s ease;
    z-index: 6;
  }
  .sidebar:hover .toggle,
  .toggle:focus-visible {
    opacity: 1;
  }
  .toggle:hover {
    color: var(--text);
  }

  /* ---- Collapsed rail: icon-only, 60px ---- */
  .sidebar.collapsed .scroll {
    align-items: center;
    padding: 12px 0;
    gap: 8px;
  }
  .sidebar.collapsed .name,
  .sidebar.collapsed kbd,
  .sidebar.collapsed .heading {
    display: none;
  }
  .sidebar.collapsed .section {
    align-items: center;
    gap: 6px;
  }
  .sidebar.collapsed .compose,
  .sidebar.collapsed .search,
  .sidebar.collapsed .item {
    width: 40px;
    height: 40px;
    padding: 0;
    justify-content: center;
    gap: 0;
    position: relative;
  }
  .sidebar.collapsed .compose {
    border-radius: var(--radius-m);
  }
  .sidebar.collapsed .compose .plus {
    font-size: 20px;
  }
  .sidebar.collapsed .search {
    margin-top: 0;
  }
  .sidebar.collapsed .dot {
    display: none;
  }
  .sidebar.collapsed .initial {
    display: block;
  }
  /* unread count → corner badge (ink, never the AI accent) */
  .sidebar.collapsed .count {
    position: absolute;
    top: -2px;
    right: -2px;
    min-width: 16px;
    height: 16px;
    padding: 0 4px;
    border-radius: 8px;
    background: var(--text);
    color: var(--bg);
    font-size: 9.5px;
    font-weight: 700;
    display: grid;
    place-items: center;
    border: 2px solid var(--bg);
  }
  /* active folder: ink bar on the left edge of the rail */
  .sidebar.collapsed .item.selected::before {
    content: "";
    position: absolute;
    left: -10px;
    top: 50%;
    transform: translateY(-50%);
    width: 3px;
    height: 22px;
    border-radius: 2px;
    background: var(--text);
  }
  .sidebar.collapsed .footer {
    align-items: center;
    padding: 10px 0 12px;
  }
  .sidebar.collapsed .sync {
    justify-content: center;
    padding: 6px 0;
  }
  .sidebar.collapsed .avatar {
    width: 26px;
    height: 26px;
    font-size: 12px;
  }
  @media (prefers-reduced-motion: reduce) {
    .sidebar,
    .toggle {
      transition: none;
    }
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
