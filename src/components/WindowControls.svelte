<script lang="ts">
  // Custom titlebar for the app's own windows (they're built with
  // decorations: false, so this is the whole window chrome).
  import { t } from "../lib/i18n/index.svelte";

  let { title, onclose }: { title: string; onclose?: () => void } = $props();

  let maximized = $state(false);

  async function win() {
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    return getCurrentWindow();
  }

  async function toggleMaximize() {
    const w = await win();
    await w.toggleMaximize();
    maximized = await w.isMaximized();
  }

  async function close() {
    if (onclose) {
      onclose();
      return;
    }
    (await win()).close();
  }
</script>

<header class="titlebar" data-tauri-drag-region>
  <span class="title" data-tauri-drag-region>{title}</span>
  <div class="controls">
    <button class="ctl" onclick={async () => (await win()).minimize()} aria-label={t("a11y.minimize")}>
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
    flex-shrink: 0;
    background: var(--bg);
  }
  .title {
    padding-left: 16px;
    font-weight: 700;
    font-size: 13.5px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
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
