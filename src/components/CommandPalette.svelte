<script lang="ts">
  // Ctrl+K palette: commands + instant local search. Becomes AI chat in
  // phase 7 (ask a question ending with "?").
  import { api } from "../lib/api";
  import { t } from "../lib/i18n/index.svelte";
  import { mail } from "../lib/stores/mail.svelte";
  import { palette } from "../lib/stores/palette.svelte";
  import { ui } from "../lib/stores/ui.svelte";
  import type { SearchHit } from "../lib/types";

  let input = $state("");
  let hits = $state<SearchHit[]>([]);
  let active = $state(0);
  let inputEl: HTMLInputElement | undefined = $state();
  let searchTimer: ReturnType<typeof setTimeout> | null = null;

  interface Command {
    id: string;
    label: string;
    hint?: string;
    run: () => void | Promise<void>;
  }

  const commands = $derived.by<Command[]>(() => {
    const list: Command[] = [
      {
        id: "compose",
        label: t("palette.compose"),
        hint: "Ctrl N",
        run: async () => {
          const draft = await api.createDraft();
          await api.openComposeWindow(draft.id);
        },
      },
      {
        id: "theme",
        label: t("palette.theme"),
        run: () => ui.cycleTheme(),
      },
      {
        id: "sync",
        label: t("palette.sync"),
        run: () => mail.syncNow(),
      },
    ];
    for (const folder of mail.folders) {
      if (folder.role === "all") continue;
      list.push({
        id: `goto-${folder.id}`,
        label: t("palette.goto", { folder: folder.displayName }),
        run: () => mail.selectFolder(folder.id),
      });
    }
    return list;
  });

  const filteredCommands = $derived(
    input.trim() === ""
      ? commands.slice(0, 4)
      : commands.filter((c) => c.label.toLowerCase().includes(input.trim().toLowerCase())),
  );

  const totalItems = $derived(filteredCommands.length + hits.length);

  $effect(() => {
    if (palette.open) {
      input = "";
      hits = [];
      active = 0;
      queueMicrotask(() => inputEl?.focus());
    }
  });

  function onInput() {
    active = 0;
    if (searchTimer) clearTimeout(searchTimer);
    const q = input.trim();
    if (q.length < 2) {
      hits = [];
      return;
    }
    searchTimer = setTimeout(async () => {
      const result = await api.searchMessages(q, 12).catch(() => []);
      if (input.trim() === q) hits = result;
    }, 140);
  }

  async function openHit(hit: SearchHit) {
    palette.hide();
    if (hit.folderId !== mail.selectedFolderId) {
      await mail.selectFolder(hit.folderId);
    }
    if (hit.threadId !== null) {
      mail.selectedThreadId = hit.threadId;
    }
  }

  async function activate(index: number) {
    if (index < filteredCommands.length) {
      const cmd = filteredCommands[index];
      palette.hide();
      await cmd.run();
    } else {
      const hit = hits[index - filteredCommands.length];
      if (hit) await openHit(hit);
    }
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      palette.hide();
    } else if (e.key === "ArrowDown") {
      e.preventDefault();
      active = Math.min(active + 1, totalItems - 1);
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      active = Math.max(active - 1, 0);
    } else if (e.key === "Enter") {
      e.preventDefault();
      void activate(active);
    }
  }

  function formatDate(unix: number): string {
    return new Date(unix * 1000).toLocaleDateString(undefined, {
      month: "short",
      day: "numeric",
    });
  }
</script>

{#if palette.open}
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div class="overlay" onclick={() => palette.hide()}>
    <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
    <div class="panel" onclick={(e) => e.stopPropagation()}>
      <div class="input-row">
        <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.4">
          <circle cx="7" cy="7" r="4.5" /><path d="M10.5 10.5L14 14" />
        </svg>
        <input
          bind:this={inputEl}
          bind:value={input}
          oninput={onInput}
          onkeydown={onKeydown}
          placeholder={t("palette.placeholder")}
          spellcheck="false"
        />
        <kbd>ESC</kbd>
      </div>

      <div class="items">
        {#if filteredCommands.length > 0}
          <div class="microlabel section">{t("palette.commands")}</div>
          {#each filteredCommands as cmd, i (cmd.id)}
            <button
              class="item"
              class:active={active === i}
              onclick={() => activate(i)}
              onmouseenter={() => (active = i)}
            >
              <span class="cmd-icon">›</span>
              <span class="label">{cmd.label}</span>
              {#if cmd.hint}<kbd>{cmd.hint}</kbd>{/if}
            </button>
          {/each}
        {/if}

        {#if hits.length > 0}
          <div class="microlabel section">{t("palette.results")}</div>
          {#each hits as hit, j (hit.messageId)}
            {@const i = filteredCommands.length + j}
            <button
              class="item"
              class:active={active === i}
              onclick={() => activate(i)}
              onmouseenter={() => (active = i)}
            >
              <span class="cmd-icon">✉</span>
              <span class="hit">
                <span class="hit-top">
                  <span class="hit-from">{hit.fromName}</span>
                  <span class="hit-subject">{hit.subject}</span>
                </span>
                {#if hit.snippet}
                  <span class="hit-snippet">{hit.snippet}</span>
                {/if}
              </span>
              <span class="date microlabel">{formatDate(hit.date)}</span>
            </button>
          {/each}
        {/if}

        {#if totalItems === 0 && input.trim().length >= 2}
          <div class="empty">{t("palette.no_results")}</div>
        {/if}
      </div>
    </div>
  </div>
{/if}

<style>
  .overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.35);
    display: flex;
    justify-content: center;
    padding-top: 12vh;
    z-index: 100;
  }
  .panel {
    width: 620px;
    max-width: calc(100vw - 48px);
    max-height: 60vh;
    background: var(--surface-raised);
    border: 1px solid var(--hairline-strong);
    border-radius: var(--radius-l);
    box-shadow: var(--shadow-pop);
    display: flex;
    flex-direction: column;
    overflow: hidden;
    height: fit-content;
  }

  .input-row {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 14px 16px;
    border-bottom: 1px solid var(--hairline);
    color: var(--text-dim);
  }
  .input-row input {
    flex: 1;
    font-size: 15px;
    color: var(--text);
    user-select: text;
  }
  kbd {
    font-family: var(--font-mono);
    font-size: 10px;
    color: var(--text-faint);
    border: 1px solid var(--hairline-strong);
    border-radius: 4px;
    padding: 2px 6px;
  }

  .items {
    overflow-y: auto;
    padding: 8px;
  }
  .section {
    padding: 8px 10px 4px;
  }
  .item {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    text-align: left;
    padding: 8px 10px;
    border-radius: var(--radius-s);
    font-size: 13.5px;
  }
  .item.active {
    background: var(--selected);
  }
  .cmd-icon {
    color: var(--text-faint);
    width: 16px;
    text-align: center;
    flex-shrink: 0;
  }
  .label {
    flex: 1;
  }

  .hit {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .hit-top {
    display: flex;
    gap: 8px;
    min-width: 0;
  }
  .hit-from {
    font-weight: 600;
    flex-shrink: 0;
  }
  .hit-subject {
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .hit-snippet {
    font-size: 12px;
    color: var(--text-faint);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .date {
    flex-shrink: 0;
  }

  .empty {
    padding: 24px;
    text-align: center;
    color: var(--text-faint);
    font-size: 13px;
  }
</style>
