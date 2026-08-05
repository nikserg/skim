<script lang="ts">
  // The AI chat over one email, as a dock under the message or as the whole of
  // its own window. A view only: the session (and the request streaming into it)
  // is owned by the main window's store, so this can be mounted, unmounted and
  // mirrored into a second window without touching the conversation.
  import { aiLinks } from "../lib/ai-links";
  import { isAlive, type ChatSession } from "../lib/ai-chat";
  import { t } from "../lib/i18n/index.svelte";
  import { mdLite } from "../lib/md";
  import { createSlowStart } from "../lib/slow-start.svelte";
  import ChatViewToggle from "./ChatViewToggle.svelte";

  interface Props {
    session: ChatSession;
    /** Where this copy is drawn: docked under the message, filling the app
     *  window, or alone in a window of its own. The last two share a layout —
     *  they both get all the room there is. */
    view?: "dock" | "full" | "window";
    onsend: (question: string) => void;
    /** Inline only (dock and full). */
    onclose?: () => void;
    onpopout?: () => void;
    ontoggleexpand?: () => void;
    /** Window only: put the chat back inline. */
    onreturn?: () => void;
  }
  let {
    session,
    view = "dock",
    onsend,
    onclose,
    onpopout,
    ontoggleexpand,
    onreturn,
  }: Props = $props();

  const inline = $derived(view !== "window");

  let question = $state("");
  let inputEl: HTMLInputElement | undefined = $state();
  let threadEl: HTMLDivElement | undefined = $state();

  const streaming = $derived(session.status === "streaming");
  const busy = $derived(streaming || session.answer !== "" || session.steps.length > 0);

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

  function send(q: string) {
    if (!q || streaming) return;
    question = "";
    onsend(q);
  }

  function submit(ev: SubmitEvent) {
    ev.preventDefault();
    send(question.trim());
  }

  // A round that failed hands its question back — pick it up so it can be
  // retried with one keystroke.
  $effect(() => {
    if (session.pending) question = session.pending;
  });

  // The chat opens focused, whichever surface it opens in.
  $effect(() => {
    inputEl?.focus();
  });

  // Keep the dialog scrolled to the newest turn / streaming delta.
  $effect(() => {
    void session.answer;
    void session.steps.length;
    void session.turns.length;
    if (threadEl) threadEl.scrollTop = threadEl.scrollHeight;
  });

  // The chat owns these keys itself: its input takes focus the moment it opens,
  // so App's handler bails out on isTyping() long before it could see them. The
  // palette and the shortcuts overlay claim their keys the same way.
  function onKeydown(e: KeyboardEvent) {
    const mod = e.ctrlKey || e.metaKey;
    if (mod && e.code === "KeyE" && e.shiftKey) {
      // Toggle: out to a window of its own, or back from one.
      e.preventDefault();
      if (view === "window") onreturn?.();
      else onpopout?.();
      return;
    }
    if (mod && e.code === "KeyE") {
      // Toggle: dock ⇄ the whole window. A chat that already has a window of
      // its own has nothing left to fill.
      if (view === "window") return;
      e.preventDefault();
      ontoggleexpand?.();
      return;
    }
    // Escape steps back one view at a time — full to the dock, dock to closed.
    // In its own window the window itself handles it (AiChatRoot).
    if (e.key === "Escape" && inline) {
      e.preventDefault();
      if (view === "full") ontoggleexpand?.();
      else onclose?.();
    }
  }
</script>

<svelte:window onkeydown={onKeydown} />

<div class="ai-dock" class:fill={view !== "dock"}>
  <!-- A row of its own rather than buttons floating over the dialogue: the chat
       keeps its full width, and every action can show the key that does it. -->
  <header class="dock-head">
    {#if view === "full"}
      <!-- This view covers the message, so the chat names what it is about. -->
      <span class="head-ctx" title={session.title}>
        <span class="ai-spark">✦</span>{session.title}
      </span>
    {/if}
    <div class="head-tools">
      {#if view === "window"}
        <ChatViewToggle mode="in" keys="Esc" onclick={() => onreturn?.()} />
      {:else}
        <ChatViewToggle
          mode={view === "full" ? "collapse" : "expand"}
          keys="Ctrl E"
          onclick={() => ontoggleexpand?.()}
        />
        <ChatViewToggle mode="out" keys="Ctrl Shift E" onclick={() => onpopout?.()} />
        <button
          class="dock-btn"
          onclick={() => onclose?.()}
          title={t("ai.close_chat")}
          aria-label={t("ai.close_chat")}
        >
          <svg width="9" height="9" viewBox="0 0 10 10"><path d="M0 0L10 10M10 0L0 10" stroke="currentColor" stroke-width="1.2" /></svg>
          <kbd>Esc</kbd>
        </button>
      {/if}
    </div>
  </header>
  {#if session.turns.length > 0 || busy || session.status === "error"}
    <div class="ask-thread" bind:this={threadEl}>
      {#each session.turns as turn, ti (ti)}
        {#if turn.role === "user"}
          <div class="ask-q">{turn.content}</div>
        {:else}
          <div class="ai-card">
            <div class="ai-label microlabel">{t("ai.answer")}</div>
            <div class="ai-text md-body" use:aiLinks>{@html mdLite(turn.content)}</div>
          </div>
        {/if}
      {/each}
      {#if streaming}
        <div class="ai-card">
          <div class="ai-label microlabel">{t("ai.answer")}</div>
          {#if session.steps.length > 0}
            <div class="ai-steps">
              {#each session.steps as step (step.id)}
                <div class="ai-step" class:done={step.done}>
                  <span class="ai-step-icon">🌐</span>
                  <span class="ai-step-label">{t("ai.step.fetch", { arg: step.arg })}</span>
                  {#if step.done}
                    <span class="ai-step-detail">✓</span>
                  {:else}
                    <span class="thinking">…</span>
                  {/if}
                </div>
              {/each}
            </div>
          {/if}
          {#if session.answer === ""}
            <span class="thinking">{slowStart.slow && !isAlive(session) ? t("ai.loading_model") : t("ai.thinking")}</span>
          {:else}
            <div class="ai-text md-body" use:aiLinks>{@html mdLite(session.answer)}</div>
          {/if}
        </div>
      {/if}
      {#if session.status === "error"}
        <div class="ai-card error">
          <div class="ai-label microlabel">{t("ai.answer")}</div>
          <div class="ai-text">{session.errorText}</div>
        </div>
      {/if}
    </div>
  {/if}
  <form class="ask-form" onsubmit={submit}>
    <span class="ai-spark">✦</span>
    <input
      bind:this={inputEl}
      bind:value={question}
      placeholder={session.turns.length > 0 ? t("ai.ask_followup") : t("ai.ask_placeholder")}
      spellcheck="false"
      disabled={streaming}
    />
  </form>
  <div class="ask-quick">
    <button class="quick-btn" onclick={() => send(t("ai.prompt_summarize"))}>
      {t("ai.summarize")}
    </button>
    <!-- No translate chip: translation belongs to the message, not to the chat.
         The bar above the body (and T) does it in place. -->
    {#if session.flagged}
      <!-- Contextual: only exists when local heuristics flagged this message,
           so honest mail never grows an extra button. -->
      <button class="quick-btn" onclick={() => send(t("ai.prompt_phishing"))}>
        {t("ai.check_phishing")}
      </button>
    {/if}
  </div>
</div>

<style>
  /* The dialogue scrolls, the header and the input stay put — so the way out
     is never scrolled off. */
  .ai-dock {
    display: flex;
    flex-direction: column;
    overflow: hidden;
    border-top: 1px solid var(--hairline);
    padding: 8px 36px 12px;
    max-height: 38vh;
    flex-shrink: 0;
  }
  /* Filling the app window, or a window of its own: take all the room there is. */
  .ai-dock.fill {
    border-top: none;
    flex: 1;
    min-height: 0;
    max-height: none;
    padding: 8px 24px 18px;
  }
  /* A line length that stays readable however wide the window gets. */
  .fill .dock-head,
  .fill .ask-thread,
  .fill .ask-form,
  .fill .ask-quick {
    width: 100%;
    max-width: 780px;
    margin-inline: auto;
  }
  .fill .ask-thread {
    flex: 1;
  }
  /* Hang the conversation off the bottom, next to the input, instead of
     stranding two lines at the top of an empty window. An auto margin on the
     first turn does it without the clipping `justify-content: flex-end` causes
     once the thread overflows. */
  .fill .ask-thread > :first-child {
    margin-top: auto;
  }
  /* Nothing said yet: the input still belongs at the bottom, as in any chat. */
  .fill .ask-form {
    margin-top: auto;
  }

  .dock-head {
    display: flex;
    align-items: center;
    gap: 12px;
    flex-shrink: 0;
    margin-bottom: 6px;
  }
  .head-ctx {
    display: flex;
    align-items: baseline;
    gap: 6px;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 12px;
    color: var(--text-dim);
  }
  .head-tools {
    display: flex;
    align-items: center;
    /* Wide enough that each icon reads as belonging to the key beside it. */
    gap: 10px;
    flex-shrink: 0;
    /* Pushed right on its own, so the label beside it is optional. */
    margin-left: auto;
  }
  .dock-btn {
    height: 24px;
    padding: 0 4px;
    display: flex;
    align-items: center;
    gap: 5px;
    border-radius: var(--radius-s);
    color: var(--text-faint);
    transition:
      background 0.12s ease,
      color 0.12s ease;
  }
  .dock-btn:hover {
    background: var(--hover);
    color: var(--text);
  }
  .dock-btn kbd {
    font-family: var(--font-mono);
    font-size: 10px;
    color: inherit;
  }
  @media (prefers-reduced-motion: reduce) {
    .dock-btn {
      transition: none;
    }
  }

  .ask-thread {
    display: flex;
    flex-direction: column;
    gap: 10px;
    min-height: 0;
    overflow-y: auto;
    margin-bottom: 10px;
  }
  .ask-thread .ai-card {
    margin-top: 0;
  }
  .ask-q {
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

  .ask-form {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 10px 14px;
    flex-shrink: 0;
    border: 1px solid var(--accent-dim);
    border-radius: var(--radius-m);
  }
  .ai-spark {
    color: var(--accent);
  }
  .ask-form input {
    flex: 1;
    font-size: 13.5px;
    user-select: text;
  }
  .ask-form input:disabled {
    color: var(--text-dim);
  }

  /* Quick prompts under the input: canned AI actions that seed the same chat.
     Accent-tinted — allowed here because these are AI features. */
  .ask-quick {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin-top: 8px;
    flex-shrink: 0;
  }
  .quick-btn {
    padding: 4px 11px;
    border-radius: var(--radius-m);
    border: 1px solid var(--accent-dim);
    color: var(--accent);
    font-size: 12px;
    font-weight: 600;
    white-space: nowrap;
  }
  .quick-btn:hover {
    background: var(--accent-soft);
  }

  .ai-card {
    margin-top: 10px;
    padding: 14px 16px;
    border-radius: var(--radius-m);
    background: var(--accent-soft);
    font-size: 13.5px;
    line-height: 1.55;
  }
  .ai-card.error {
    background: transparent;
    border: 1px solid var(--hairline-strong);
    color: var(--text-dim);
  }
  .ai-label {
    color: var(--accent);
    margin-bottom: 6px;
  }
  .ai-text {
    white-space: pre-wrap;
    user-select: text;
    cursor: text;
  }
  .ai-steps {
    display: flex;
    flex-direction: column;
    gap: 3px;
    margin-bottom: 8px;
  }
  .ai-step {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    color: var(--text-faint);
  }
  .ai-step.done {
    opacity: 0.7;
  }
  .ai-step-icon {
    font-size: 11px;
  }
  .ai-step-label {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ai-step-detail {
    color: var(--text-faint);
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
</style>
