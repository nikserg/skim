<script lang="ts">
  // Ctrl+K palette: commands + instant local search. Becomes AI chat in
  // phase 7 (ask a question ending with "?").
  import { aiStream, api, type Citation } from "../lib/api";
  import { getLocale, t } from "../lib/i18n/index.svelte";
  import { mdLite } from "../lib/md";
  import { ai } from "../lib/stores/ai.svelte";
  import { mail } from "../lib/stores/mail.svelte";
  import { palette } from "../lib/stores/palette.svelte";
  import { ui } from "../lib/stores/ui.svelte";
  import type { SearchHit } from "../lib/types";

  let input = $state("");
  let hits = $state<SearchHit[]>([]);
  let active = $state(0);
  let inputEl: HTMLInputElement | undefined = $state();
  let searchTimer: ReturnType<typeof setTimeout> | null = null;

  // ---- AI chat mode (screen 1g) ----
  interface ChatStep {
    id: string;
    kind: string;
    arg: string;
    count: number | null;
    done: boolean;
  }
  let chat = $state<{
    question: string;
    answer: string;
    status: "streaming" | "done" | "error";
    citations: Citation[];
    steps: ChatStep[];
  } | null>(null);
  let cancelChat: (() => void) | null = null;

  function startChat(question: string) {
    cancelChat?.();
    chat = { question, answer: "", status: "streaming", citations: [], steps: [] };
    cancelChat = aiStream(
      "ai_chat",
      { question, contextMessageId: null },
      {
        delta: (text) => {
          if (chat) chat = { ...chat, answer: chat.answer + text };
        },
        toolCall: (id, kind, arg) => {
          if (chat)
            chat = { ...chat, steps: [...chat.steps, { id, kind, arg, count: null, done: false }] };
        },
        toolDone: (id, count) => {
          if (chat)
            chat = {
              ...chat,
              steps: chat.steps.map((s) => (s.id === id ? { ...s, count, done: true } : s)),
            };
        },
        done: (citations) => {
          if (chat) chat = { ...chat, status: "done", citations };
        },
        error: (code, message) => {
          chat = {
            question,
            answer:
              code === "ai_no_context"
                ? t("ai.no_context")
                : code === "ai_key"
                  ? t("ai.needs_key")
                  : message,
            status: "error",
            citations: [],
            steps: chat?.steps ?? [],
          };
        },
      },
    );
  }

  function exitChat() {
    cancelChat?.();
    cancelChat = null;
    chat = null;
    queueMicrotask(() => inputEl?.focus());
  }

  async function openCitation(citation: Citation) {
    palette.hide();
    exitChat();
    if (citation.folderId !== mail.selectedFolderId) {
      await mail.selectFolder(citation.folderId);
    }
    if (citation.threadId !== null) {
      mail.selectedThreadId = citation.threadId;
    }
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
    const roleKey: Record<string, string> = {
      inbox: "nav.inbox",
      starred: "nav.starred",
      sent: "nav.sent",
      drafts: "nav.drafts",
      archive: "nav.archive",
      trash: "nav.trash",
      junk: "nav.junk",
    };
    for (const folder of mail.folders) {
      if (folder.role === "all") continue;
      const name =
        folder.role && roleKey[folder.role] ? t(roleKey[folder.role]) : folder.displayName;
      list.push({
        id: `goto-${folder.id}`,
        label: t("palette.goto", { folder: name }),
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
      chat = null;
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
    if (aiItemVisible && index === 0) {
      startChat(input.trim());
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
        <div class="chat">
          <div class="chat-question">{chat.question}</div>
          {#if chat.steps.length > 0}
            <div class="steps">
              {#each chat.steps as step (step.id)}
                <div class="step" class:done={step.done}>
                  <span class="step-icon">{step.kind === "read" ? "📧" : "🔍"}</span>
                  <span class="step-label"
                    >{step.kind === "read"
                      ? t("ai.step.read", { arg: step.arg })
                      : t("ai.step.search", { arg: step.arg })}</span
                  >
                  {#if step.done}
                    {#if step.count !== null}
                      <span class="step-detail">{t("ai.step.found", { n: step.count })}</span>
                    {:else}
                      <span class="step-detail">✓</span>
                    {/if}
                  {:else}
                    <span class="step-spin thinking">…</span>
                  {/if}
                </div>
              {/each}
            </div>
          {/if}
          <div class="chat-answer">
            <div class="microlabel chat-label">✦ {t("ai.answer")}</div>
            {#if chat.answer === "" && chat.status === "streaming"}
              <span class="thinking">{t("ai.thinking")}</span>
            {:else}
              <div class="chat-text md-body">{@html mdLite(chat.answer)}</div>
            {/if}
          </div>
          {#if chat.citations.length > 0}
            <div class="sources">
              <span class="microlabel">{t("ai.sources")} · {chat.citations.length}</span>
              <div class="source-chips">
                {#each chat.citations as c (c.index)}
                  <button class="source-chip" onclick={() => openCitation(c)}>
                    <span class="source-index">{c.index}</span>
                    {c.subject || c.from}
                  </button>
                {/each}
              </div>
            </div>
          {/if}
          <button class="chat-footer microlabel" onclick={exitChat}>ESC ↩</button>
        </div>
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

  .chat {
    padding: 18px;
    display: flex;
    flex-direction: column;
    gap: 14px;
    overflow-y: auto;
  }
  .chat-question {
    font-size: 15px;
    font-weight: 700;
  }
  .steps {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .step {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 12.5px;
    color: var(--text-dim);
    padding: 3px 2px;
  }
  .step-icon {
    flex-shrink: 0;
    font-size: 12px;
  }
  .step-label {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .step.done .step-label {
    color: var(--text);
  }
  .step-detail {
    flex-shrink: 0;
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--accent);
  }
  .step-spin {
    flex-shrink: 0;
  }

  .chat-answer {
    background: var(--accent-soft);
    border-radius: var(--radius-m);
    padding: 12px 14px;
    font-size: 13.5px;
    line-height: 1.55;
  }
  .chat-label {
    color: var(--accent);
    margin-bottom: 6px;
  }
  .chat-text {
    white-space: pre-wrap;
    user-select: text;
    cursor: text;
  }
  .thinking {
    color: var(--accent);
    animation: pulse 1.2s ease-in-out infinite;
  }
  @keyframes pulse {
    50% {
      opacity: 0.45;
    }
  }
  .sources {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .source-chips {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .source-chip {
    display: flex;
    align-items: center;
    gap: 7px;
    padding: 5px 10px;
    border: 1px solid var(--accent-dim);
    border-radius: 999px;
    font-size: 12px;
    color: var(--text);
    max-width: 260px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .source-chip:hover {
    background: var(--accent-soft);
  }
  .source-index {
    font-family: var(--font-mono);
    font-size: 10px;
    color: var(--accent);
  }
  .chat-footer {
    align-self: flex-end;
    color: var(--text-faint);
  }
  .chat-footer:hover {
    color: var(--text);
  }
</style>
