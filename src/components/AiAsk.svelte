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
    /** True in the chat window: fills it, and swaps the dock chrome for the
     *  way back. */
    standalone?: boolean;
    onsend: (question: string) => void;
    /** Dock only. */
    onclose?: () => void;
    onpopout?: () => void;
    /** Window only: put the chat back inline. */
    onreturn?: () => void;
  }
  let { session, standalone = false, onsend, onclose, onpopout, onreturn }: Props = $props();

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
</script>

<div class="ai-dock" class:standalone>
  <div class="dock-tools">
    {#if standalone}
      <ChatViewToggle mode="in" onclick={() => onreturn?.()} />
    {:else}
      <ChatViewToggle mode="out" onclick={() => onpopout?.()} />
      <button class="dock-btn" onclick={() => onclose?.()} aria-label={t("a11y.close")}>
        <svg width="9" height="9" viewBox="0 0 10 10"><path d="M0 0L10 10M10 0L0 10" stroke="currentColor" stroke-width="1.2" /></svg>
      </button>
    {/if}
  </div>
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
    <button class="quick-btn" onclick={() => send(t("ai.prompt_translate"))}>
      {t("ai.translate")}
    </button>
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
  .ai-dock {
    position: relative;
    border-top: 1px solid var(--hairline);
    padding: 12px 36px;
    max-height: 38vh;
    overflow-y: auto;
    flex-shrink: 0;
  }
  /* Own window: fill it, and drop the room the dock reserves for its buttons. */
  .ai-dock.standalone {
    border-top: none;
    flex: 1;
    min-height: 0;
    max-height: none;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    /* Top padding clears the floating way-back button. */
    padding: 34px 24px 18px;
  }
  .standalone .ask-thread {
    flex: 1;
    max-height: none;
    min-height: 0;
  }
  .standalone .ask-q,
  .standalone .ask-form,
  .standalone .ask-quick,
  .standalone .ai-card {
    margin-right: 0;
  }
  .dock-tools {
    position: absolute;
    top: 10px;
    right: 14px;
    display: flex;
    align-items: center;
    gap: 2px;
    z-index: 1;
  }
  .standalone .dock-tools {
    top: 6px;
  }
  .dock-btn {
    width: 24px;
    height: 24px;
    display: grid;
    place-items: center;
    border-radius: var(--radius-s);
    color: var(--text-faint);
  }
  .dock-btn:hover {
    background: var(--hover);
    color: var(--text);
  }

  .ask-thread {
    display: flex;
    flex-direction: column;
    gap: 10px;
    max-height: 26vh;
    overflow-y: auto;
    margin-bottom: 10px;
  }
  .ask-thread .ai-card {
    margin-top: 0;
  }
  .ask-q {
    align-self: flex-end;
    max-width: 80%;
    margin-right: 30px;
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
    margin-right: 30px;
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
    margin-right: 30px;
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
    margin-right: 30px;
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
