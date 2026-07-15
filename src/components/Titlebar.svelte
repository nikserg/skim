<script lang="ts">
  import { t } from "../lib/i18n/index.svelte";
  import { mail } from "../lib/stores/mail.svelte";

  const inTauri = "__TAURI_INTERNALS__" in window;

  let maximized = $state(false);

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

<header class="titlebar" data-tauri-drag-region>
  <div class="brand" data-tauri-drag-region>
    <span class="logo" data-tauri-drag-region>{t("app.name")}</span>
    {#if mail.account}
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
  }
  .logo {
    font-weight: 800;
    font-size: 15px;
    letter-spacing: -0.02em;
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
