<script lang="ts">
  // The pair of buttons that move an AI chat between the app and a window of
  // its own: "out" where the chat is cramped, "in" inside the window. Same
  // component so the two can't drift apart the way the old expand buttons did;
  // hosts only decide where to put it.
  import { t } from "../lib/i18n/index.svelte";

  let { mode, onclick }: { mode: "out" | "in"; onclick: () => void } = $props();

  const label = $derived(mode === "out" ? t("ai.open_window") : t("ai.return_inline"));
</script>

<button class="view-toggle" onclick={() => onclick()} title={label} aria-label={label}>
  {#if mode === "out"}
    <svg width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" stroke-linejoin="round">
      <path d="M9.5 6.5V9.5a1 1 0 0 1-1 1h-6a1 1 0 0 1-1-1v-6a1 1 0 0 1 1-1H5.5" />
      <path d="M7.5 1.5H10.5V4.5M10.5 1.5L6 6" />
    </svg>
  {:else}
    <svg width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" stroke-linejoin="round">
      <path d="M9.5 6.5V9.5a1 1 0 0 1-1 1h-6a1 1 0 0 1-1-1v-6a1 1 0 0 1 1-1H5.5" />
      <path d="M10.5 1.5L6.5 5.5M6.5 5.5H9M6.5 5.5V3" />
    </svg>
    <kbd>ESC</kbd>
  {/if}
</button>

<style>
  .view-toggle {
    height: 24px;
    min-width: 24px;
    padding: 0 4px;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    border-radius: var(--radius-s);
    color: var(--text-faint);
    transition:
      background 0.12s ease,
      color 0.12s ease;
  }
  .view-toggle:hover {
    background: var(--hover);
    /* Violet on purpose: this button only ever sits in an AI surface. */
    color: var(--accent);
  }
  kbd {
    font-family: var(--font-mono);
    font-size: 10px;
    color: inherit;
    border: 1px solid var(--hairline-strong);
    border-radius: 4px;
    padding: 2px 5px;
  }
  @media (prefers-reduced-motion: reduce) {
    .view-toggle {
      transition: none;
    }
  }
</style>
