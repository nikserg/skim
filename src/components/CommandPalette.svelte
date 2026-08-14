<script lang="ts">
  // Ctrl+K palette: commands + instant local search, plus the mailbox-wide
  // AI chat (the "Ask Skim AI" row on any query).
  import { api, type Citation } from "../lib/api";
  import { folderLabel } from "../lib/folders";
  import { getLocale, t } from "../lib/i18n/index.svelte";
  import { ai } from "../lib/stores/ai.svelte";
  import { aiSessions } from "../lib/stores/aiSession.svelte";
  import { mail } from "../lib/stores/mail.svelte";
  import { palette } from "../lib/stores/palette.svelte";
  import { ui } from "../lib/stores/ui.svelte";
  import type { SearchHit } from "../lib/types";
  import AiChat from "./AiChat.svelte";

  let input = $state("");
  let hits = $state<SearchHit[]>([]);
  let active = $state(0);
  let inputEl: HTMLInputElement | undefined = $state();
  let searchTimer: ReturnType<typeof setTimeout> | null = null;

  // ---- AI chat mode (screen 1g) ----
  // The conversation lives in the session store, so it survives being popped
  // into a window of its own and coming back. The palette shows whichever
  // mailbox-wide chat is currently inline.
  const chat = $derived(aiSessions.paletteChat);

  /** Back to the search screen. The conversation ends with it, as before. */
  function exitChat() {
    if (chat) aiSessions.drop(chat);
    queueMicrotask(() => inputEl?.focus());
  }

  /** Give the chat a window of its own and get the palette out of the way. It
   *  keeps streaming; closing that window brings it back here. */
  async function popOut() {
    if (!chat) return;
    await aiSessions.detach(chat);
    palette.hide();
  }

  async function openCitation(citation: Citation) {
    palette.hide();
    // AI answers cite mail from any connected account — openLocation switches
    // the active one first when needed.
    await mail.openLocation(citation.folderId, citation.threadId, citation.messageId);
  }

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
          const draft = await api.createDraft(await mail.composeAccountId());
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
      {
        id: "toggle-sidebar",
        label: t("palette.toggle_sidebar"),
        hint: ".",
        run: () => ui.toggleSidebar(),
      },
      {
        id: "shortcuts",
        label: t("palette.shortcuts"),
        hint: "?",
        run: () => ui.openShortcuts(),
      },
    ];
    // Filing needs something to file — offering the row with nothing selected
    // would be a dead end.
    if (mail.selectedThread) {
      const thread = mail.selectedThread;
      list.push({
        id: "move",
        label: t("palette.move"),
        hint: "V",
        run: async () => {
          const ids = await api.threadMessageIds(thread.id);
          if (ids.length > 0)
            ui.openMove({ rowKeys: mail.rowKeysForThread(thread.id), messageIds: ids });
        },
      });
    }
    for (const folder of mail.folders) {
      if (folder.role === "all") continue;
      list.push({
        id: `goto-${folder.id}`,
        label: t("palette.goto", { folder: folderLabel(folder) }),
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

  const aiItemVisible = $derived(ai.keyPresent && input.trim().length > 2);
  const totalItems = $derived(filteredCommands.length + hits.length + (aiItemVisible ? 1 : 0));

  $effect(() => {
    if (palette.open) {
      input = "";
      hits = [];
      active = 0;
      queueMicrotask(() => inputEl?.focus());
    } else {
      // Closing the palette ends the chat it was holding — but not one that has
      // moved into its own window, which is exactly where it should be.
      const inline = aiSessions.paletteChat;
      if (inline) aiSessions.drop(inline);
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
      // Search the mailbox the user is looking at — the active account.
      const result = await api.searchMessages(q, 12, mail.account?.id).catch(() => []);
      if (input.trim() === q) hits = result;
    }, 140);
  }

  async function openHit(hit: SearchHit) {
    palette.hide();
    await mail.openLocation(hit.folderId, hit.threadId, hit.messageId);
  }

  async function activate(index: number) {
    if (aiItemVisible && index === 0) {
      aiSessions.startPaletteChat(input.trim(), ui.openMessageId);
      return;
    }
    const cmdIndex = index - (aiItemVisible ? 1 : 0);
    if (cmdIndex < filteredCommands.length) {
      const cmd = filteredCommands[cmdIndex];
      palette.hide();
      await cmd.run();
      return;
    }
    const hitIndex = cmdIndex - filteredCommands.length;
    if (hitIndex < hits.length) {
      const hit = hits[hitIndex];
      if (hit) await openHit(hit);
    }
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.key === "ArrowDown") {
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

  // Escape is handled at the window level so it works in chat mode too,
  // where the search input (and its keydown handler) is not mounted.
  function onWindowKeydown(e: KeyboardEvent) {
    if (!palette.open) return;
    if (e.key === "Escape") {
      e.preventDefault();
      if (chat) {
        exitChat();
      } else {
        palette.hide();
      }
      return;
    }
    // The same chord that pops the chat about an email out — one key for
    // "give this chat a window of its own", wherever the chat started.
    if (chat && (e.ctrlKey || e.metaKey) && e.shiftKey && e.code === "KeyE") {
      e.preventDefault();
      void popOut();
    }
  }

  function formatDate(unix: number): string {
    return new Date(unix * 1000).toLocaleDateString(getLocale(), {
      month: "short",
      day: "numeric",
    });
  }
</script>

<svelte:window onkeydown={onWindowKeydown} />

{#if palette.open}
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div class="overlay" onclick={() => palette.hide()}>
    <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
    <div class="panel" onclick={(e) => e.stopPropagation()}>
      {#if chat}
        <AiChat
          session={chat}
          onsend={(q) => aiSessions.send(chat, q)}
          oncitation={openCitation}
          onpopout={() => void popOut()}
        />
      {:else}
      <div class="input-row">
        <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.4">
          <circle cx="7" cy="7" r="4.5" /><path d="M10.5 10.5L14 14" />
        </svg>
        <input
          bind:this={inputEl}
          bind:value={input}
          oninput={onInput}
          onkeydown={onKeydown}
          placeholder={ai.keyPresent ? t("palette.placeholder_ai") : t("palette.placeholder")}
          spellcheck="false"
        />
        <kbd>ESC</kbd>
      </div>

      <div class="items">
        {#if aiItemVisible}
          <button
            class="item ai-item"
            class:active={active === 0}
            onclick={() => activate(0)}
            onmouseenter={() => (active = 0)}
          >
            <span class="cmd-icon spark">✦</span>
            <span class="label">{t("ai.ask_ai", { q: input.trim() })}</span>
          </button>
        {/if}

        {#if filteredCommands.length > 0}
          <div class="microlabel section">{t("palette.commands")}</div>
          {#each filteredCommands as cmd, j (cmd.id)}
            {@const i = (aiItemVisible ? 1 : 0) + j}
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
            {@const i = (aiItemVisible ? 1 : 0) + filteredCommands.length + j}
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
      {/if}
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
    position: relative;
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

  /* AI — violet accent */
  .spark {
    color: var(--accent);
  }
  .ai-item .label {
    color: var(--accent);
    font-weight: 600;
  }

</style>
