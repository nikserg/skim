<script lang="ts">
  // Address field with autocomplete over every address in the mailbox.
  // Addresses are comma-separated; only the segment being typed (after the
  // last comma) drives the suggestions, so picking one and typing again
  // keeps adding recipients.
  import { api, type AddressSuggestion } from "../lib/api";

  let {
    value = $bindable(""),
    onchange,
  }: { value: string; onchange?: () => void } = $props();

  let inputEl: HTMLInputElement | undefined = $state();
  let items = $state<AddressSuggestion[]>([]);
  let open = $state(false);
  let active = $state(0);
  let timer: ReturnType<typeof setTimeout> | null = null;
  let seq = 0;

  function split(): { head: string; tail: string } {
    const i = value.lastIndexOf(",");
    return i === -1
      ? { head: "", tail: value }
      : { head: value.slice(0, i + 1), tail: value.slice(i + 1) };
  }

  function onInput() {
    onchange?.();
    const q = split().tail.trim();
    if (timer) clearTimeout(timer);
    if (q.length === 0) {
      open = false;
      return;
    }
    const request = ++seq;
    timer = setTimeout(async () => {
      const res = await api.suggestAddresses(q).catch(() => []);
      if (request !== seq) return; // a newer keystroke superseded this one
      const already = split().head.toLowerCase();
      items = res.filter((s) => !already.includes(s.addr.toLowerCase()));
      active = 0;
      open = items.length > 0;
    }, 120);
  }

  function pick(s: AddressSuggestion) {
    const { head } = split();
    value = head ? `${head.trimEnd()} ${s.addr}, ` : `${s.addr}, `;
    open = false;
    onchange?.();
    inputEl?.focus();
  }

  function onKeydown(e: KeyboardEvent) {
    if (!open) return;
    switch (e.key) {
      case "ArrowDown":
        e.preventDefault();
        active = (active + 1) % items.length;
        break;
      case "ArrowUp":
        e.preventDefault();
        active = (active - 1 + items.length) % items.length;
        break;
      case "Enter":
      case "Tab":
        e.preventDefault();
        pick(items[active]);
        break;
      case "Escape":
        e.stopPropagation();
        open = false;
        break;
    }
  }
</script>

<div class="wrap">
  <input
    bind:this={inputEl}
    bind:value
    oninput={onInput}
    onkeydown={onKeydown}
    onblur={() => setTimeout(() => (open = false), 120)}
    spellcheck="false"
    autocomplete="off"
  />
  {#if open}
    <div class="suggest">
      {#each items as s, i (s.addr)}
        <button
          class="item"
          class:active={i === active}
          onmousedown={(e) => {
            e.preventDefault();
            pick(s);
          }}
        >
          <span class="name">{s.name || s.addr}</span>
          {#if s.name}<span class="addr">{s.addr}</span>{/if}
        </button>
      {/each}
    </div>
  {/if}
</div>

<style>
  .wrap {
    flex: 1;
    position: relative;
    min-width: 0;
  }
  input {
    width: 100%;
    font-size: 13.5px;
    user-select: text;
  }
  .suggest {
    position: absolute;
    top: calc(100% + 6px);
    left: -8px;
    right: 0;
    max-width: 460px;
    background: var(--surface-raised);
    border: 1px solid var(--hairline-strong);
    border-radius: var(--radius-m);
    box-shadow: var(--shadow-pop);
    padding: 4px;
    z-index: 50;
    overflow: hidden;
  }
  .item {
    display: flex;
    align-items: baseline;
    gap: 10px;
    width: 100%;
    text-align: left;
    padding: 7px 10px;
    border-radius: var(--radius-s);
    font-size: 13px;
    min-width: 0;
  }
  .item.active,
  .item:hover {
    background: var(--hover);
  }
  .name {
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .addr {
    color: var(--text-faint);
    font-size: 12px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    flex-shrink: 1;
  }
</style>
