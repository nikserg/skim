<script lang="ts">
  import { copyText } from "../lib/clipboard";
  import { t } from "../lib/i18n/index.svelte";

  interface Props {
    value: string;
    label: string;
  }

  let { value, label }: Props = $props();

  let justCopied = $state(false);
  let timer: number | undefined;

  async function onClick(e: MouseEvent) {
    // Don't let the click fall through to a parent that collapses the message
    // or toggles anything else.
    e.stopPropagation();
    e.preventDefault();
    const ok = await copyText(value);
    if (!ok) return;
    justCopied = true;
    if (timer !== undefined) clearTimeout(timer);
    timer = window.setTimeout(() => {
      justCopied = false;
      timer = undefined;
    }, 1200);
  }
</script>

<button
  type="button"
  class="copy-btn"
  class:copied={justCopied}
  aria-label={justCopied ? t("reading.copied") : label}
  title={justCopied ? t("reading.copied") : label}
  onclick={onClick}
>
  {#if justCopied}
    <!-- Checkmark: brief confirmation. -->
    <svg width="13" height="13" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
      <path d="M3.5 8.5l3 3 6-6" />
    </svg>
  {:else}
    <!-- Two stacked pages: copy. -->
    <svg width="13" height="13" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
      <rect x="5.5" y="5.5" width="8" height="8" rx="1.3" />
      <path d="M10.5 5.5V3.3a1 1 0 0 0-1-1H3.3a1 1 0 0 0-1 1v6.2a1 1 0 0 0 1 1h2.2" />
    </svg>
  {/if}
</button>

<style>
  .copy-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 20px;
    height: 20px;
    padding: 0;
    margin-left: 4px;
    border: none;
    background: transparent;
    color: var(--text-faint);
    border-radius: var(--radius-s);
    cursor: pointer;
    opacity: 0;
    transition:
      opacity 0.08s,
      color 0.08s,
      background 0.08s;
    vertical-align: middle;
    flex-shrink: 0;
  }
  :global(.copy-host:hover) > .copy-btn,
  :global(.copy-host) > .copy-btn:focus-visible,
  :global(.copy-host:hover) .copy-btn,
  :global(.copy-host) .copy-btn:focus-visible,
  .copy-btn.copied {
    opacity: 1;
  }
  .copy-btn:hover {
    color: var(--text-dim);
    background: var(--hover);
  }
  .copy-btn.copied {
    color: var(--text);
  }

  @media (prefers-reduced-motion: reduce) {
    .copy-btn {
      transition: none;
    }
  }
</style>
