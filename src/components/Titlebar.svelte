<script lang="ts">
  import { api } from "../lib/api";
  import { t } from "../lib/i18n/index.svelte";
  import { mail } from "../lib/stores/mail.svelte";

  const inTauri = "__TAURI_INTERNALS__" in window;

  let maximized = $state(false);

  // Account switcher — only rendered when more than one mailbox is connected.
  let switcherOpen = $state(false);
  let unread = $state<Record<string, number>>({});
  let brandEl = $state<HTMLElement | null>(null);

  function toggleSwitcher() {
    switcherOpen = !switcherOpen;
    if (switcherOpen) {
      void api
        .inboxUnreadCounts()
        .then((counts) => (unread = counts))
        .catch(() => {});
    }
  }

  function pick(id: string) {
    switcherOpen = false;
    void mail.switchAccount(id);
  }

  function onWindowMousedown(e: MouseEvent) {
    if (switcherOpen && brandEl && !brandEl.contains(e.target as Node)) switcherOpen = false;
  }
  function onWindowKeydown(e: KeyboardEvent) {
    if (switcherOpen && e.key === "Escape") switcherOpen = false;
  }

  async function win() {
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    return getCurrentWindow();
  }

  async function minimize() {
    if (inTauri) (await win()).minimize();
  }
  async function toggleMaximize() {
    if (!inTauri) return;
    const w = await win();
    await w.toggleMaximize();
    maximized = await w.isMaximized();
  }
  async function close() {
    if (inTauri) (await win()).close();
  }
</script>

<svelte:window onmousedown={onWindowMousedown} onkeydown={onWindowKeydown} />

<header class="titlebar" data-tauri-drag-region>
  <div class="brand" data-tauri-drag-region bind:this={brandEl}>
    <span class="logo" data-tauri-drag-region>{t("app.name")}</span>
    {#if mail.accounts.length > 1}
      <span class="switch-wrap">
        <button
          class="account microlabel switcher"
          aria-label={t("accounts.switch")}
          aria-expanded={switcherOpen}
          onclick={toggleSwitcher}
        >
          {mail.account?.email}
          <svg class="chev" width="8" height="8" viewBox="0 0 8 8" aria-hidden="true">
            <path d="M1 2.5L4 5.5L7 2.5" fill="none" stroke="currentColor" stroke-width="1.2" />
          </svg>
        </button>
        {#if switcherOpen}
          <div class="accounts-pop" role="listbox">
            {#each mail.accounts as a (a.id)}
              <button
                class="account-row"
                role="option"
                aria-selected={a.id === mail.account?.id}
                onclick={() => pick(a.id)}
              >
                <span class="addr">{a.email}</span>
                {#if a.id === mail.account?.id}
                  <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true">
                    <path d="M1.5 5.5L4 8L8.5 2.5" fill="none" stroke="currentColor" stroke-width="1.4" />
                  </svg>
                {:else if unread[a.id]}
                  <span class="count">{unread[a.id]}</span>
                {/if}
              </button>
            {/each}
          </div>
        {/if}
      </span>
    {:else if mail.account}
      <span class="account microlabel" data-tauri-drag-region>{mail.account.email}</span>
    {/if}
  </div>
  <div class="controls">
    <button class="ctl" onclick={minimize} aria-label={t("a11y.minimize")}>
      <svg width="10" height="10" viewBox="0 0 10 10"><line x1="0" y1="5" x2="10" y2="5" stroke="currentColor" stroke-width="1" /></svg>
    </button>
    <button class="ctl" onclick={toggleMaximize} aria-label={t("a11y.maximize")}>
      {#if maximized}
        <svg width="10" height="10" viewBox="0 0 10 10"><rect x="0.5" y="2.5" width="7" height="7" fill="none" stroke="currentColor" /><path d="M2.5 2.5V0.5H9.5V7.5H7.5" fill="none" stroke="currentColor" /></svg>
      {:else}
        <svg width="10" height="10" viewBox="0 0 10 10"><rect x="0.5" y="0.5" width="9" height="9" fill="none" stroke="currentColor" /></svg>
      {/if}
    </button>
    <button class="ctl ctl-close" onclick={close} aria-label={t("a11y.close")}>
      <svg width="10" height="10" viewBox="0 0 10 10"><path d="M0 0L10 10M10 0L0 10" stroke="currentColor" stroke-width="1" /></svg>
    </button>
  </div>
</header>

<style>
  .titlebar {
    height: var(--titlebar-h);
    display: flex;
    align-items: center;
    justify-content: space-between;
    border-bottom: 1px solid var(--hairline);
    background: var(--bg);
    flex-shrink: 0;
  }
  .brand {
    display: flex;
    align-items: baseline;
    gap: 12px;
    padding-left: 16px;
    position: relative;
  }
  .logo {
    font-weight: 800;
    font-size: 15px;
    letter-spacing: -0.02em;
  }
  .switch-wrap {
    position: relative;
  }
  .switcher {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 2px 4px;
    margin: -2px -4px;
    border-radius: var(--radius-s);
  }
  .switcher:hover {
    background: var(--hover);
    color: var(--text);
  }
  .chev {
    flex-shrink: 0;
  }
  .accounts-pop {
    position: absolute;
    top: calc(100% + 10px);
    left: -4px;
    z-index: 30;
    min-width: 240px;
    padding: 4px;
    display: flex;
    flex-direction: column;
    background: var(--surface);
    border: 1px solid var(--hairline-strong);
    border-radius: var(--radius-s);
    box-shadow: var(--shadow-pop);
  }
  .account-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    padding: 8px 10px;
    border-radius: calc(var(--radius-s) - 2px);
    font-size: 13px;
    text-align: left;
  }
  .account-row:hover {
    background: var(--hover);
  }
  .account-row .addr {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .account-row .count {
    font-family: var(--font-mono);
    font-size: 11.5px;
    color: var(--text-dim);
  }
  .controls {
    display: flex;
    height: 100%;
  }
  .ctl {
    width: 46px;
    height: 100%;
    display: grid;
    place-items: center;
    color: var(--text-dim);
    transition: background 0.1s;
  }
  .ctl:hover {
    background: var(--hover);
    color: var(--text);
  }
  .ctl-close:hover {
    background: #d64545;
    color: #fff;
  }
</style>
