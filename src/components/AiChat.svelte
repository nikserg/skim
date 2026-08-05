<script lang="ts">
  // The mailbox-wide AI chat: a conversation whose answers cite real emails,
  // shown inside the command palette or as the whole of its own window. A view
  // only — the session and the request live in the main window's store.
  import type { Citation } from "../lib/api";
  import { aiLinks } from "../lib/ai-links";
  import { isAlive, type ChatSession } from "../lib/ai-chat";
  import { t } from "../lib/i18n/index.svelte";
  import { mdLite } from "../lib/md";
  import { createSlowStart } from "../lib/slow-start.svelte";
  import ChatViewToggle from "./ChatViewToggle.svelte";

  interface Props {
    session: ChatSession;
    /** True in the chat window: fills it, and swaps the palette chrome for the
     *  way back. */
    standalone?: boolean;
    onsend: (question: string) => void;
    oncitation: (citation: Citation) => void;
    /** Palette only. */
    onpopout?: () => void;
    /** Window only: put the chat back inline. */
    onreturn?: () => void;
  }
  let { session, standalone = false, onsend, oncitation, onpopout, onreturn }: Props = $props();

  let followup = $state("");
  let followupEl: HTMLInputElement | undefined = $state();
  let threadEl: HTMLDivElement | undefined = $state();

  const streaming = $derived(session.status === "streaming");

  // "The model is loading" hint: armed when a round starts, dropped as soon as
  // anything comes back.
  const slowStart = createSlowStart();
  let wasStreaming = false;
  $effect(() => {
    const now = session.status === "streaming";
    if (now && !wasStreaming) slowStart.arm();
    if (!now) slowStart.clear();
    wasStreaming = now;
  });
  // Guarded in the template too: this effect runs after the DOM update, so on
  // its own the label would flash "loading" for one frame.
  $effect(() => {
    if (isAlive(session)) slowStart.clear();
  });

  function submitFollowup(e: SubmitEvent) {
    e.preventDefault();
    const q = followup.trim();
    if (!q || streaming) return;
    followup = "";
    onsend(q);
  }

  // A round that failed hands its question back — pick it up so it can be
  // retried with one keystroke.
  $effect(() => {
    if (session.pending) followup = session.pending;
  });

  $effect(() => {
    if (!streaming) followupEl?.focus();
  });

  // Keep the thread pinned to the newest turn / streaming delta.
  $effect(() => {
    void session.answer;
    void session.turns.length;
    void session.steps.length;
    if (threadEl) threadEl.scrollTop = threadEl.scrollHeight;
  });
</script>

<div class="ai-chat" class:standalone>
  <div class="chat-tools">
    {#if standalone}
      <ChatViewToggle mode="in" keys="Esc" onclick={() => onreturn?.()} />
    {:else}
      <ChatViewToggle mode="out" keys="Ctrl Shift E" onclick={() => onpopout?.()} />
    {/if}
  </div>
  <div class="chat" bind:this={threadEl}>
    {#each session.turns as turn, ti (ti)}
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
                <button class="source-chip" onclick={() => oncitation(c)}>
                  <span class="source-index">{c.index}</span>
                  {c.subject || c.from}
                </button>
              {/each}
            </div>
          </div>
        {/if}
      {/if}
    {/each}

    {#if streaming}
      {#if session.steps.length > 0}
        <div class="steps">
          {#each session.steps as step (step.id)}
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
        {#if session.answer === ""}
          <span class="thinking">{slowStart.slow && !isAlive(session) ? t("ai.loading_model") : t("ai.thinking")}</span>
        {:else}
          <div class="chat-text md-body" use:aiLinks>{@html mdLite(session.answer)}</div>
        {/if}
      </div>
    {/if}

    {#if session.status === "error"}
      <div class="chat-answer error">
        <div class="chat-text">{session.errorText}</div>
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
      disabled={streaming}
    />
    {#if !standalone}
      <kbd>ESC</kbd>
    {/if}
  </form>
</div>

<style>
  .ai-chat {
    position: relative;
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .chat-tools {
    position: absolute;
    top: 10px;
    right: 10px;
    z-index: 1;
  }
  .standalone .chat-tools {
    top: 6px;
  }

  .chat {
    flex: 1;
    min-height: 0;
    /* Extra top padding reserves room for the floating toggle so it never
       overlaps the first (right-aligned) message bubble. */
    padding: 40px 18px 18px;
    display: flex;
    flex-direction: column;
    gap: 14px;
    overflow-y: auto;
  }
  /* Own window: room to read. */
  .standalone .chat {
    padding: 36px 20px 18px;
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
  .spark {
    color: var(--accent);
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
  kbd {
    font-family: var(--font-mono);
    font-size: 10px;
    color: var(--text-faint);
    border: 1px solid var(--hairline-strong);
    border-radius: 4px;
    padding: 2px 6px;
  }
</style>
