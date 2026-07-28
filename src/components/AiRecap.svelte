<script lang="ts">
  // AI Recap: a catch-up digest of the folder's unread mail. Occupies the
  // reading pane while open, or the whole of its own window once popped out.
  // A view only — the session, the scan and any follow-up in flight live in the
  // main window's store, which is what lets the digest keep streaming while it
  // moves between the two.
  //
  // The conversation opens with a hidden seed turn and the digest itself
  // (turns 0 and 1); anything after that is a follow-up the user asked.
  import type { Citation } from "../lib/api";
  import { aiLinks } from "../lib/ai-links";
  import { isAlive, type ChatSession } from "../lib/ai-chat";
  import { t } from "../lib/i18n/index.svelte";
  import { mdLite } from "../lib/md";
  import { createSlowStart } from "../lib/slow-start.svelte";
  import { ui } from "../lib/stores/ui.svelte";
  import ChatViewToggle from "./ChatViewToggle.svelte";

  interface Props {
    session: ChatSession;
    /** True in the chat window: drops the panel header for the way back. */
    standalone?: boolean;
    onsend: (question: string) => void;
    oncitation: (citation: Citation) => void;
    /** Panel only. */
    onclose?: () => void;
    onpopout?: () => void;
    /** Window only: put the digest back in the reading pane. */
    onreturn?: () => void;
  }
  let {
    session,
    standalone = false,
    onsend,
    oncitation,
    onclose,
    onpopout,
    onreturn,
  }: Props = $props();

  let followup = $state("");
  let bodyEl: HTMLDivElement | undefined = $state();

  const streaming = $derived(session.status === "streaming");
  /** The digest itself, once it has landed. */
  const digest = $derived(session.turns[1]?.content ?? session.answer);
  const cited = $derived(session.turns[1]?.citations ?? []);
  const done = $derived(session.turns.length >= 2);
  /** Still counting unread mail: no digest text has arrived yet. */
  const scanning = $derived(streaming && !done && session.answer === "");

  // Cold-start hint, shared between the scan and the follow-ups since only one
  // of them can be in flight at a time.
  const slowStart = createSlowStart();
  let wasStreaming = false;
  $effect(() => {
    if (streaming && !wasStreaming) slowStart.arm();
    if (!streaming) slowStart.clear();
    wasStreaming = streaming;
    // A cold model start can leave the counter frozen at N/N after the scan
    // itself finishes — re-arm on every progress tick.
    if (session.progress) slowStart.arm();
  });
  $effect(() => {
    if (isAlive(session)) slowStart.clear();
  });

  // Keep the panel pinned to the newest turn / streaming delta.
  $effect(() => {
    void session.answer;
    void session.turns.length;
    void session.steps.length;
    if (bodyEl) bodyEl.scrollTop = bodyEl.scrollHeight;
  });

  // A round that failed hands its question back — pick it up so it can be
  // retried with one keystroke.
  $effect(() => {
    if (session.pending) followup = session.pending;
  });

  function submitFollowup(e: SubmitEvent) {
    e.preventDefault();
    const q = followup.trim();
    if (!q || streaming) return;
    followup = "";
    onsend(q);
  }
</script>

<section class="recap" class:standalone>
  {#if standalone}
    <div class="tools">
      <ChatViewToggle mode="in" onclick={() => onreturn?.()} />
    </div>
  {:else}
    <header class="head">
      <span class="title">✦ {t("ai.recap_title")}</span>
      <div class="head-tools">
        <ChatViewToggle mode="out" onclick={() => onpopout?.()} />
        <button class="close" onclick={() => onclose?.()} aria-label={t("settings.close")}>
          <svg width="11" height="11" viewBox="0 0 10 10"><path d="M0 0L10 10M10 0L0 10" stroke="currentColor" stroke-width="1.2" /></svg>
        </button>
      </div>
    </header>
  {/if}

  <div class="body" bind:this={bodyEl}>
    {#if scanning}
      <div class="progress">
        <span class="spinner"></span>
        {#if slowStart.slow}
          {t("ai.loading_model")}
        {:else if session.reasoning}
          <!-- The scan is over and the digest is being thought through: the
               counter would sit frozen at N/N saying "reading". -->
          {t("ai.thinking")}
        {:else}
          {t("ai.recap_reading")}
          {#if session.progress}{session.progress.current}/{session.progress.total}{/if}
        {/if}
      </div>
    {:else if !done && session.status === "error"}
      <div class="error">{session.errorText}</div>
    {:else}
      {#if ui.temperature === "warm"}
        <div class="eyebrow">
          // {t("ai.recap_eyebrow", { n: cited.length || session.scannedTotal })} //
        </div>
      {/if}
      <div class="clip">
        <div class="clip-paper">
          <div class="text md-body" use:aiLinks>{@html mdLite(digest)}</div>
        </div>
      </div>
      {#if done}
        {#if session.markedCount > 0}
          <div class="marked microlabel">✓ {t("ai.recap_marked", { n: session.markedCount })}</div>
        {/if}
        {#if cited.length > 0}
          <div class="sources">
            <span class="microlabel">{t("ai.sources")} · {cited.length}</span>
            <div class="chips">
              {#each cited as c (c.index)}
                <button class="chip" onclick={() => oncitation(c)}>
                  <span class="index">{c.index}</span>
                  {c.subject || c.from}
                </button>
              {/each}
            </div>
          </div>
        {/if}

        <div class="followups">
          {#each session.turns.slice(2) as turn, ti (ti)}
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
                  <div class="chips">
                    {#each turn.citations as c (c.index)}
                      <button class="chip" onclick={() => oncitation(c)}>
                        <span class="index">{c.index}</span>
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
                <span class="thinking">{slowStart.slow ? t("ai.loading_model") : t("ai.thinking")}</span>
              {:else}
                <div class="chat-text md-body" use:aiLinks>{@html mdLite(session.answer)}</div>
              {/if}
            </div>
          {/if}

          {#if session.status === "error"}
            <div class="chat-answer error-turn">
              <div class="chat-text">{session.errorText}</div>
            </div>
          {/if}
        </div>
      {/if}
    {/if}
  </div>

  {#if done}
    <form class="followup-form" onsubmit={submitFollowup}>
      <span class="spark">✦</span>
      <input
        bind:value={followup}
        placeholder={t("ai.ask_followup")}
        spellcheck="false"
        disabled={streaming}
      />
    </form>
  {/if}
</section>

<style>
  .recap {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    background: var(--surface);
  }
  /* Own window: the titlebar names it, so the panel header gives way to a
     floating button back. */
  .recap.standalone {
    position: relative;
    min-height: 0;
  }
  .tools {
    position: absolute;
    top: 10px;
    right: 14px;
    z-index: 1;
  }
  .standalone .body {
    padding-top: 40px;
  }
  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 18px 24px 12px;
    border-bottom: 1px solid var(--accent-dim);
  }
  .head-tools {
    display: flex;
    align-items: center;
    gap: 4px;
  }
  .title {
    color: var(--accent);
    font-weight: 700;
    font-size: 14px;
  }
  .close {
    width: 28px;
    height: 28px;
    display: grid;
    place-items: center;
    border-radius: var(--radius-s);
    color: var(--text-dim);
  }
  .close:hover {
    background: var(--hover);
    color: var(--text);
  }
  .body {
    flex: 1;
    overflow-y: auto;
    padding: 20px 24px;
    display: flex;
    flex-direction: column;
    gap: 16px;
  }
  .progress {
    display: flex;
    align-items: center;
    gap: 10px;
    color: var(--accent);
    font-size: 13px;
  }
  .spinner {
    width: 13px;
    height: 13px;
    border: 2px solid var(--accent-dim);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
    flex-shrink: 0;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  .text {
    font-size: 14px;
    line-height: 1.65;
    max-width: 640px;
  }
  /* Warm-only "paper clipping" treatment: eyebrow + torn card + tape strip.
     In cold themes .clip is an inert wrapper and .eyebrow isn't rendered. */
  .eyebrow {
    font-family: var(--font-mono);
    font-size: 11px;
    letter-spacing: 0.14em;
    text-transform: uppercase;
    color: var(--accent);
    transform: rotate(-1deg);
  }
  .clip {
    max-width: 640px;
  }
  /* .clip is the unclipped wrapper (rotation + tape); .clip-paper carries the
     torn-edge clip-path, so the tape (::before on .clip) isn't cut off. */
  :global(:root[data-theme="warm-light"]) .clip,
  :global(:root[data-theme="warm-dark"]) .clip {
    position: relative;
    transform: rotate(-0.6deg);
  }
  :global(:root[data-theme="warm-light"]) .clip-paper,
  :global(:root[data-theme="warm-dark"]) .clip-paper {
    background: var(--surface-raised);
    border: 1px solid var(--hairline-strong);
    box-shadow: 4px 6px 0 rgba(28, 23, 18, 0.1);
    padding: 24px 22px;
    clip-path: polygon(
      0 2%,
      4% 0,
      45% 2%,
      72% 0,
      100% 2%,
      99% 45%,
      100% 74%,
      98% 100%,
      58% 98%,
      26% 100%,
      2% 99%,
      0 55%
    );
  }
  /* translucent tape strip sitting on the top edge (on the unclipped .clip) */
  :global(:root[data-theme="warm-light"]) .clip::before,
  :global(:root[data-theme="warm-dark"]) .clip::before {
    content: "";
    position: absolute;
    top: -13px;
    left: 38px;
    width: 96px;
    height: 26px;
    background: rgba(216, 190, 120, 0.5);
    box-shadow: 0 2px 5px rgba(0, 0, 0, 0.16);
    transform: rotate(-4deg);
    z-index: 1;
  }
  @media (prefers-reduced-motion: reduce) {
    :global(:root[data-theme="warm-light"]) .clip,
    :global(:root[data-theme="warm-dark"]) .clip {
      transform: none;
    }
    .eyebrow {
      transform: none;
    }
  }
  .marked {
    color: var(--success);
  }
  .error {
    color: var(--danger);
    font-size: 13px;
  }
  .sources {
    display: flex;
    flex-direction: column;
    gap: 8px;
    max-width: 640px;
  }
  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    max-width: 100%;
    padding: 5px 11px 5px 6px;
    border: 1px solid var(--accent-dim);
    border-radius: 999px;
    font-size: 12px;
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .chip:hover {
    background: var(--accent-soft);
  }
  .index {
    width: 17px;
    height: 17px;
    border-radius: 50%;
    background: var(--accent);
    color: var(--on-accent);
    display: grid;
    place-items: center;
    font-size: 10.5px;
    font-weight: 700;
    flex-shrink: 0;
  }

  /* ---- follow-up conversation ------------------------------------------- */
  .followups {
    display: flex;
    flex-direction: column;
    gap: 14px;
    max-width: 640px;
  }
  .chat-question {
    align-self: flex-end;
    max-width: 85%;
    padding: 8px 12px;
    border-radius: var(--radius-m);
    background: var(--accent-soft);
    color: var(--text);
    font-size: 13.5px;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .chat-answer {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .chat-label {
    color: var(--accent);
  }
  .chat-text {
    font-size: 14px;
    line-height: 1.65;
  }
  .chat-answer.error-turn .chat-text {
    color: var(--danger);
    font-size: 13px;
  }
  .thinking {
    color: var(--text-dim);
    font-size: 13px;
  }
  .steps {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .step {
    display: flex;
    align-items: center;
    gap: 7px;
    font-size: 12px;
    color: var(--text-dim);
  }
  .step.done {
    color: var(--text);
  }
  .step-detail {
    color: var(--text-dim);
  }
  .followup-form {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 12px 24px;
    border-top: 1px solid var(--accent-dim);
    flex-shrink: 0;
  }
  .followup-form .spark {
    color: var(--accent);
    font-size: 13px;
    flex-shrink: 0;
  }
  .followup-form input {
    flex: 1;
    min-width: 0;
    border: none;
    background: transparent;
    color: var(--text);
    font-size: 14px;
    outline: none;
  }
  .followup-form input::placeholder {
    color: var(--text-dim);
  }
  .followup-form input:disabled {
    opacity: 0.5;
  }
</style>
