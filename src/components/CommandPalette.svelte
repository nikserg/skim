<script lang="ts">
  // Ctrl+K palette: commands + instant local search, plus the mailbox-wide
  // AI chat (the "Ask Skim AI" row on any query).
  import { aiStream, api, type Citation } from "../lib/api";
  import { aiLinks } from "../lib/ai-links";
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
  // A continuable conversation: `turns` holds completed user/assistant turns,
  // while the in-flight assistant answer streams into `answer`/`steps` until
  // "done" folds it into a turn. Follow-ups carry the whole history.
  interface ChatStep {
    id: string;
    kind: string;
    arg: string;
    count: number | null;
    done: boolean;
  }
  interface ChatTurn {
    role: "user" | "assistant";
    content: string;
    citations: Citation[];
  }
  let chat = $state<{
    turns: ChatTurn[];
    answer: string;
    steps: ChatStep[];
    status: "streaming" | "done" | "error";
    errorText: string;
    /** The email open in the reading pane when the session started; the
     * backend folds it into the first turn, so it's fixed for the session. */
    contextMessageId: number | null;
  } | null>(null);
  let followup = $state("");
  let followupEl: HTMLInputElement | undefined = $state();
  let chatThreadEl: HTMLDivElement | undefined = $state();
  let cancelChat: (() => void) | null = null;
  // A custom endpoint's cold start can stall the first token for many seconds
  // — after 5s of silence, say what is actually happening.
  let slowStart = $state(false);
  let slowTimer: ReturnType<typeof setTimeout> | undefined;
  function armSlowStart() {
    clearTimeout(slowTimer);
    slowStart = false;
    if (ai.provider !== "custom") return;
    slowTimer = setTimeout(() => (slowStart = true), 5000);
  }
  function clearSlowStart() {
    clearTimeout(slowTimer);
    slowStart = false;
  }

  function askChat(question: string) {
    cancelChat?.();
    if (!chat)
      chat = {
        turns: [],
        answer: "",
        steps: [],
        status: "streaming",
        errorText: "",
        contextMessageId: ui.openMessageId,
      };
    chat.turns = [...chat.turns, { role: "user", content: question, citations: [] }];
    chat.answer = "";
    chat.steps = [];
    chat.status = "streaming";
    chat.errorText = "";
    const history = chat.turns.map((tn) => ({ role: tn.role, content: tn.content }));
    // Citations from earlier answers, deduped by their [N] number, so the agent
    // can resolve/read those emails again on a follow-up and keep numbers stable.
    const byIndex = new Map<number, Citation>();
    for (const tn of chat.turns) for (const c of tn.citations) byIndex.set(c.index, c);
    const priorCitations = [...byIndex.values()];
    armSlowStart();
    cancelChat = aiStream(
      "ai_chat",
      { turns: history, priorCitations, contextMessageId: chat.contextMessageId },
      {
        delta: (text) => {
          clearSlowStart();
          if (chat) chat.answer += text;
        },
        toolCall: (id, kind, arg) => {
          clearSlowStart();
          if (chat) chat.steps = [...chat.steps, { id, kind, arg, count: null, done: false }];
        },
        toolDone: (id, count) => {
          if (chat)
            chat.steps = chat.steps.map((s) => (s.id === id ? { ...s, count, done: true } : s));
        },
        done: (citations) => {
          clearSlowStart();
          if (!chat) return;
          chat.turns = [...chat.turns, { role: "assistant", content: chat.answer, citations }];
          chat.answer = "";
          chat.steps = [];
          chat.status = "done";
          queueMicrotask(() => followupEl?.focus());
        },
        error: (code, message) => {
          clearSlowStart();
          if (!chat) return;
          // Drop the unanswered question back into the box so it can be retried.
          const last = chat.turns[chat.turns.length - 1];
          if (last?.role === "user") {
            followup = last.content;
            chat.turns = chat.turns.slice(0, -1);
          }
          chat.answer = "";
          chat.steps = [];
          chat.status = "error";
          chat.errorText =
            code === "ai_no_context"
              ? t("ai.no_context")
              : code === "ai_key"
                ? t("ai.needs_key")
                : message;
          queueMicrotask(() => followupEl?.focus());
        },
      },
    );
  }

  function submitFollowup(e: SubmitEvent) {
    e.preventDefault();
    const q = followup.trim();
    if (!q || chat?.status === "streaming") return;
    followup = "";
    askChat(q);
  }

  function exitChat() {
    cancelChat?.();
    cancelChat = null;
    chat = null;
    followup = "";
    clearSlowStart();
    queueMicrotask(() => inputEl?.focus());
  }

  // Keep the thread pinned to the newest turn / streaming delta.
  $effect(() => {
    if (!chat) return;
    void chat.answer;
    void chat.turns.length;
    void chat.steps.length;
    if (chatThreadEl) chatThreadEl.scrollTop = chatThreadEl.scrollHeight;
  });

  async function openCitation(citation: Citation) {
    palette.hide();
    exitChat();
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
      followup = "";
      clearSlowStart();
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
      askChat(input.trim());
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
    // While a chat is open, repurpose Ctrl/Cmd+K to toggle the expanded view
    // instead of closing the palette (App keeps it open via palette.show()).
    if (chat && (e.ctrlKey || e.metaKey) && e.code === "KeyK") {
      e.preventDefault();
      ui.togglePaletteExpanded();
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
    <div class="panel" class:expanded={chat && ui.paletteExpanded} onclick={(e) => e.stopPropagation()}>
      {#if chat}
        <button
          class="expand-toggle"
          onclick={() => ui.togglePaletteExpanded()}
          aria-label={ui.paletteExpanded ? t("ai.collapse") : t("ai.expand")}
          title={ui.paletteExpanded ? t("ai.collapse") : t("ai.expand")}
        >
          {#if ui.paletteExpanded}
            <svg width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1.3">
              <path d="M6.5 5.5L10 2M10 2H7.5M10 2V4.5" />
              <path d="M5.5 6.5L2 10M2 10H4.5M2 10V7.5" />
            </svg>
          {:else}
            <svg width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1.3">
              <path d="M7 5L10.5 1.5M10.5 1.5H8M10.5 1.5V4" />
              <path d="M5 7L1.5 10.5M1.5 10.5H4M1.5 10.5V8" />
            </svg>
          {/if}
        </button>
        <div class="chat" bind:this={chatThreadEl}>
          {#each chat.turns as turn, ti (ti)}
            {#if turn.role === "user"}
              <div class="chat-question">{turn.content}</div>
            {:else}
              <div class="chat-answer">
                <div class="microlabel chat-label">✦ {t("ai.answer")}</div>
                <div class="chat-text md-body" use:aiLinks>{@html mdLite(turn.content)}</div>
              </div>
              {#if turn.citations.length > 0}
                <div class="sources">
                  <span class="microlabel">{t("ai.sources")} · {turn.citations.length}</span>
                  <div class="source-chips">
                    {#each turn.citations as c (c.index)}
                      <button class="source-chip" onclick={() => openCitation(c)}>
                        <span class="source-index">{c.index}</span>
                        {c.subject || c.from}
                      </button>
                    {/each}
                  </div>
                </div>
              {/if}
            {/if}
          {/each}

          {#if chat.status === "streaming"}
            {#if chat.steps.length > 0}
              <div class="steps">
                {#each chat.steps as step (step.id)}
                  <div class="step" class:done={step.done}>
                    <span class="step-icon"
                      >{step.kind === "read" ? "📧" : step.kind === "fetch" ? "🌐" : "🔍"}</span
                    >
                    <span class="step-label"
                      >{step.kind === "read"
                        ? t("ai.step.read", { arg: step.arg })
                        : step.kind === "fetch"
                          ? t("ai.step.fetch", { arg: step.arg })
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
              {#if chat.answer === ""}
                <span class="thinking">{slowStart ? t("ai.loading_model") : t("ai.thinking")}</span>
              {:else}
                <div class="chat-text md-body" use:aiLinks>{@html mdLite(chat.answer)}</div>
              {/if}
            </div>
          {/if}

          {#if chat.status === "error"}
            <div class="chat-answer error">
              <div class="chat-text">{chat.errorText}</div>
            </div>
          {/if}
        </div>
        <form class="chat-followup" onsubmit={submitFollowup}>
          <span class="spark">✦</span>
          <input
            bind:this={followupEl}
            bind:value={followup}
            placeholder={t("ai.ask_followup")}
            spellcheck="false"
            disabled={chat.status === "streaming"}
          />
          <kbd>ESC</kbd>
        </form>
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
  /* Expand/collapse is an instant state swap (like the sidebar rail), not an
     animated resize — cheaper and jank-free for a full-height panel. */
  .panel.expanded {
    width: min(1100px, calc(100vw - 64px));
    max-height: calc(100vh - 12vh - 48px);
    height: calc(100vh - 12vh - 48px);
  }
  /* Keep answer text in a readable column instead of stretching it edge-to-edge. */
  .panel.expanded .chat,
  .panel.expanded .chat-followup {
    padding-inline: max(20px, calc((100% - 760px) / 2));
  }

  .expand-toggle {
    position: absolute;
    top: 10px;
    right: 10px;
    z-index: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    width: 26px;
    height: 26px;
    padding: 0;
    border: none;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-faint);
    cursor: pointer;
    transition:
      background 0.12s ease,
      color 0.12s ease;
  }
  .expand-toggle:hover {
    background: var(--hover);
    color: var(--accent);
  }
  @media (prefers-reduced-motion: reduce) {
    .expand-toggle {
      transition: none;
    }
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
    flex: 1;
    min-height: 0;
    /* Extra top padding reserves room for the floating expand toggle so it
       never overlaps the first (right-aligned) message bubble. */
    padding: 40px 18px 18px;
    display: flex;
    flex-direction: column;
    gap: 14px;
    overflow-y: auto;
  }
  .chat-question {
    align-self: flex-end;
    max-width: 80%;
    padding: 8px 12px;
    border: 1px solid var(--hairline-strong);
    border-radius: var(--radius-m);
    font-size: 13.5px;
    color: var(--text-dim);
    white-space: pre-wrap;
    user-select: text;
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
  .chat-answer.error {
    background: transparent;
    border: 1px solid var(--hairline-strong);
    color: var(--text-dim);
  }

  .chat-followup {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 14px 16px;
    border-top: 1px solid var(--hairline);
    flex-shrink: 0;
  }
  .chat-followup input {
    flex: 1;
    font-size: 14px;
    color: var(--text);
    user-select: text;
  }
  .chat-followup input:disabled {
    color: var(--text-dim);
  }
</style>
