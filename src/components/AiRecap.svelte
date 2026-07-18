<script lang="ts">
  // AI Recap: a catch-up digest of the folder's unread mail. Occupies the
  // reading pane while open; covered messages are marked read on success.
  // Once the digest lands, the user can keep the conversation going — follow-ups
  // route through the mailbox chat (`ai_chat`), seeded with the recap's turns and
  // citations, exactly like the Ctrl+K assistant.
  import { aiStream, api, type Citation } from "../lib/api";
  import { aiLinks } from "../lib/ai-links";
  import { t } from "../lib/i18n/index.svelte";
  import { mdLite } from "../lib/md";
  import { mail } from "../lib/stores/mail.svelte";
  import { ui } from "../lib/stores/ui.svelte";

  /** Synthetic opening turn so the digest reads as an assistant reply the user
   * can follow up on. Sent to the model, never rendered. */
  const RECAP_SEED = "Recap my unread inbox.";

  interface ChatTurn {
    role: "user" | "assistant";
    content: string;
    citations: Citation[];
  }
  interface ChatStep {
    id: string;
    kind: string;
    arg: string;
    count: number | null;
    done: boolean;
  }

  let status = $state<"scanning" | "streaming" | "done" | "error">("scanning");
  let text = $state("");
  let citations = $state<Citation[]>([]);
  let progress = $state<{ current: number; total: number } | null>(null);
  /** How many unread messages were scanned — for the digest eyebrow count. */
  let scannedTotal = $state(0);
  let markedCount = $state(0);
  let cancel: (() => void) | null = null;

  /** Follow-up conversation, seeded once the digest is done. */
  let chat = $state<{
    turns: ChatTurn[];
    answer: string;
    steps: ChatStep[];
    status: "idle" | "streaming" | "done" | "error";
    errorText: string;
  } | null>(null);
  let followup = $state("");
  let cancelChat: (() => void) | null = null;
  let bodyEl: HTMLDivElement | undefined = $state();

  $effect(() => {
    const folderId = mail.selectedFolderId;
    if (folderId === null) return;
    // Fresh recap for this folder — drop any prior conversation and streams.
    cancelChat?.();
    cancelChat = null;
    chat = null;
    followup = "";
    status = "scanning";
    text = "";
    citations = [];
    progress = null;
    scannedTotal = 0;
    markedCount = 0;
    cancel = aiStream(
      "ai_recap",
      { folderId },
      {
        progress: (current, total) => {
          progress = { current, total };
          if (total > scannedTotal) scannedTotal = total;
        },
        delta: (chunk) => {
          status = "streaming";
          progress = null;
          text += chunk;
        },
        done: (cited) => {
          status = "done";
          citations = cited;
          markRead(cited);
          // The digest becomes the first assistant reply; its citations seed the
          // [N] numbering so follow-ups can resolve those same emails.
          chat = {
            turns: [
              { role: "user", content: RECAP_SEED, citations: [] },
              { role: "assistant", content: text, citations: cited },
            ],
            answer: "",
            steps: [],
            status: "idle",
            errorText: "",
          };
        },
        error: (code, message) => {
          status = "error";
          text = code === "ai_key" ? t("ai.needs_key") : message;
        },
      },
    );
    return () => {
      cancel?.();
      cancelChat?.();
    };
  });

  // Keep the panel pinned to the newest turn / streaming delta.
  $effect(() => {
    if (!chat) return;
    void chat.answer;
    void chat.turns.length;
    void chat.steps.length;
    if (bodyEl) bodyEl.scrollTop = bodyEl.scrollHeight;
  });

  function markRead(cited: Citation[]) {
    const ids = cited.map((c) => c.messageId);
    if (ids.length === 0) return;
    markedCount = ids.length;
    for (const c of cited) {
      if (c.threadId !== null) mail.patchThreadRow(c.threadId, { isRead: true });
    }
    void api.markRead(ids, true);
  }

  function askChat(question: string) {
    if (!chat) return;
    cancelChat?.();
    chat.turns = [...chat.turns, { role: "user", content: question, citations: [] }];
    chat.answer = "";
    chat.steps = [];
    chat.status = "streaming";
    chat.errorText = "";
    const history = chat.turns.map((tn) => ({ role: tn.role, content: tn.content }));
    // Citations across all turns, deduped by their [N] number, so the agent can
    // re-resolve/read those emails on a follow-up and keep numbers stable.
    const byIndex = new Map<number, Citation>();
    for (const tn of chat.turns) for (const c of tn.citations) byIndex.set(c.index, c);
    const priorCitations = [...byIndex.values()];
    cancelChat = aiStream(
      "ai_chat",
      { turns: history, priorCitations, contextMessageId: null },
      {
        delta: (chunk) => {
          if (chat) chat.answer += chunk;
        },
        toolCall: (id, kind, arg) => {
          if (chat) chat.steps = [...chat.steps, { id, kind, arg, count: null, done: false }];
        },
        toolDone: (id, count) => {
          if (chat)
            chat.steps = chat.steps.map((s) => (s.id === id ? { ...s, count, done: true } : s));
        },
        done: (cited) => {
          if (!chat) return;
          chat.turns = [...chat.turns, { role: "assistant", content: chat.answer, citations: cited }];
          chat.answer = "";
          chat.steps = [];
          chat.status = "done";
        },
        error: (code, message) => {
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

  async function openCitation(c: Citation) {
    if (c.folderId !== mail.selectedFolderId) {
      await mail.selectFolder(c.folderId);
    }
    if (c.threadId !== null) mail.selectedThreadId = c.threadId;
    ui.closeRecap();
  }
</script>

<section class="recap">
  <header class="head">
    <span class="title">✦ {t("ai.recap_title")}</span>
    <button class="close" onclick={() => ui.closeRecap()} aria-label={t("settings.close")}>
      <svg width="11" height="11" viewBox="0 0 10 10"><path d="M0 0L10 10M10 0L0 10" stroke="currentColor" stroke-width="1.2" /></svg>
    </button>
  </header>

  <div class="body" bind:this={bodyEl}>
    {#if status === "scanning"}
      <div class="progress">
        <span class="spinner"></span>
        {t("ai.recap_reading")}
        {#if progress}{progress.current}/{progress.total}{/if}
      </div>
    {:else if status === "error"}
      <div class="error">{text}</div>
    {:else}
      {#if ui.temperature === "warm"}
        <div class="eyebrow">
          // {t("ai.recap_eyebrow", { n: citations.length || scannedTotal })} //
        </div>
      {/if}
      <div class="clip">
        <div class="clip-paper">
          <div class="text md-body" use:aiLinks>{@html mdLite(text)}</div>
        </div>
      </div>
      {#if status === "done"}
        {#if markedCount > 0}
          <div class="marked microlabel">✓ {t("ai.recap_marked", { n: markedCount })}</div>
        {/if}
        {#if citations.length > 0}
          <div class="sources">
            <span class="microlabel">{t("ai.sources")} · {citations.length}</span>
            <div class="chips">
              {#each citations as c (c.index)}
                <button class="chip" onclick={() => openCitation(c)}>
                  <span class="index">{c.index}</span>
                  {c.subject || c.from}
                </button>
              {/each}
            </div>
          </div>
        {/if}

        {#if chat}
          <div class="followups">
            {#each chat.turns.slice(2) as turn, ti (ti)}
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
                        <button class="chip" onclick={() => openCitation(c)}>
                          <span class="index">{c.index}</span>
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
                  <span class="thinking">{t("ai.thinking")}</span>
                {:else}
                  <div class="chat-text md-body" use:aiLinks>{@html mdLite(chat.answer)}</div>
                {/if}
              </div>
            {/if}

            {#if chat.status === "error"}
              <div class="chat-answer error-turn">
                <div class="chat-text">{chat.errorText}</div>
              </div>
            {/if}
          </div>
        {/if}
      {/if}
    {/if}
  </div>

  {#if chat}
    <form class="followup-form" onsubmit={submitFollowup}>
      <span class="spark">✦</span>
      <input
        bind:value={followup}
        placeholder={t("ai.ask_followup")}
        spellcheck="false"
        disabled={chat.status === "streaming"}
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
  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 18px 24px 12px;
    border-bottom: 1px solid var(--accent-dim);
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
