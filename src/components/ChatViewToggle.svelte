<script lang="ts">
  // The buttons that move an AI chat between its views: "expand"/"collapse"
  // between the dock and the whole app window, "out"/"in" between the app and a
  // window of its own. Same component so the directions can't drift apart the
  // way the old expand buttons did; hosts only decide where to put it and which
  // key to caption it with.
  import { t } from "../lib/i18n/index.svelte";

  type Mode = "out" | "in" | "expand" | "collapse";
  interface Props {
    mode: Mode;
    /** The keystroke that does the same thing, printed on the button. Latin
     *  keycaps, never localized — like every other shortcut hint. */
    keys?: string;
    onclick: () => void;
  }
  let { mode, keys, onclick }: Props = $props();

  const labels: Record<Mode, () => string> = {
    out: () => t("ai.open_window"),
    in: () => t("ai.return_inline"),
    expand: () => t("ai.expand"),
    collapse: () => t("ai.collapse"),
  };
  const label = $derived(labels[mode]());
</script>

<button class="view-toggle" onclick={() => onclick()} title={label} aria-label={label}>
  {#if mode === "out"}
    <svg width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" stroke-linejoin="round">
      <path d="M9.5 6.5V9.5a1 1 0 0 1-1 1h-6a1 1 0 0 1-1-1v-6a1 1 0 0 1 1-1H5.5" />
      <path d="M7.5 1.5H10.5V4.5M10.5 1.5L6 6" />
    </svg>
  {:else if mode === "in"}
    <svg width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" stroke-linejoin="round">
      <path d="M9.5 6.5V9.5a1 1 0 0 1-1 1h-6a1 1 0 0 1-1-1v-6a1 1 0 0 1 1-1H5.5" />
      <path d="M10.5 1.5L6.5 5.5M6.5 5.5H9M6.5 5.5V3" />
    </svg>
  {:else if mode === "expand"}
    <!-- Arrows to opposite corners: the chat grows to fill what's around it. -->
    <svg width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" stroke-linejoin="round">
      <path d="M7 1.5H10.5V5M10.5 1.5L7 5" />
      <path d="M5 10.5H1.5V7M1.5 10.5L5 7" />
    </svg>
  {:else}
    <!-- The same arrows pulled back in. -->
    <svg width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" stroke-linejoin="round">
      <path d="M10.5 1.5L7 5M7 5H10M7 5V2" />
      <path d="M1.5 10.5L5 7M5 7H2M5 7V10" />
    </svg>
  {/if}
  {#if keys}
    <kbd>{keys}</kbd>
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
    gap: 5px;
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
  /* Bare, like every other key hint sitting next to a button (the reading
     toolbar, the action row). Boxed keycaps are for the shortcuts overlay,
     where the key is the subject rather than a footnote. */
  kbd {
    font-family: var(--font-mono);
    font-size: 10px;
    color: inherit;
    white-space: nowrap;
  }
  @media (prefers-reduced-motion: reduce) {
    .view-toggle {
      transition: none;
    }
  }
</style>
