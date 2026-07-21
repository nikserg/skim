<script lang="ts">
  import { aiStream, api } from "../lib/api";
  import { aiLinks } from "../lib/ai-links";
  import { getLocale, t } from "../lib/i18n/index.svelte";
  import { mdLite } from "../lib/md";
  import { ai } from "../lib/stores/ai.svelte";
  import { aiChat } from "../lib/stores/aiChat.svelte";
  import { mail } from "../lib/stores/mail.svelte";
  import { ui } from "../lib/stores/ui.svelte";
  import type { MessageMeta, RenderedBody, ThreadDetail } from "../lib/types";
  import AttachmentChips from "./AttachmentChips.svelte";
  import HtmlViewer from "./HtmlViewer.svelte";
  import InviteCard from "./InviteCard.svelte";

  // ---- AI chat over this email ----
  // A single continuable chat is the one AI surface for the open message: the
  // "Ask" button opens it, the quick-prompt buttons seed it. The completed
  // turns live in askTurns; the in-flight answer lives in aiPanel.text.
  // A step in the reasoning trace — currently only `fetch` (opening a link the
  // email contains) surfaces here; the email chat has no search/read tools.
  interface ChatStep {
    id: string;
    kind: string;
    arg: string;
    count: number | null;
    done: boolean;
  }
  let aiPanel = $state<{
    status: "streaming" | "error";
    text: string;
    steps: ChatStep[];
  } | null>(null);
  let askOpen = $state(false);
  let askExpanded = $state(false);
  let askQuestion = $state("");
  let askTurns = $state<{ role: "user" | "assistant"; content: string }[]>([]);
  // Which message askTurns belongs to, so we can swap the visible chat for the
  // cached one as the AI target changes (thread switch, focus change).
  let askKey = $state<number | null>(null);
  let askInput: HTMLInputElement | undefined = $state();
  let askThreadEl: HTMLDivElement | undefined = $state();
  let cancelAi: (() => void) | null = null;

  // Close the dock. The completed turns stay in askTurns (and the cache), so
  // reopening — for this email or after hopping to another and back — restores
  // the session. Only the in-flight, not-yet-answered turn is dropped.
  function closeAiPanel() {
    cancelAi?.();
    aiPanel = null;
    askOpen = false;
    askExpanded = false;
    askQuestion = "";
  }

  function startAi(args: Record<string, unknown>) {
    cancelAi?.();
    aiPanel = { status: "streaming", text: "", steps: [] };
    cancelAi = aiStream("ai_ask", args, {
      delta: (text) => {
        if (aiPanel) aiPanel = { ...aiPanel, text: aiPanel.text + text };
      },
      toolCall: (id, kind, arg) => {
        if (aiPanel)
          aiPanel = {
            ...aiPanel,
            steps: [...aiPanel.steps, { id, kind, arg, count: null, done: false }],
          };
      },
      toolDone: (id, count) => {
        if (aiPanel)
          aiPanel = {
            ...aiPanel,
            steps: aiPanel.steps.map((s) => (s.id === id ? { ...s, count, done: true } : s)),
          };
      },
      done: () => {
        if (!aiPanel) return;
        askTurns.push({ role: "assistant", content: aiPanel.text });
        aiPanel = null;
        queueMicrotask(() => askInput?.focus());
      },
      error: (code, message) => {
        // Put the failed question back into the input so it can be retried.
        if (askTurns[askTurns.length - 1]?.role === "user") {
          askQuestion = askTurns.pop()!.content;
        }
        aiPanel = {
          status: "error",
          text: code === "ai_key" ? t("ai.needs_key") : message || t("ai.no_context"),
          steps: [],
        };
      },
    });
  }

  // Keep the dialog scrolled to the newest turn / streaming delta.
  $effect(() => {
    void aiPanel?.text;
    void aiPanel?.steps.length;
    void askTurns.length;
    if (askThreadEl) askThreadEl.scrollTop = askThreadEl.scrollHeight;
  });

  // Mirror the visible chat into the (LRU) cache on every completed turn, so it
  // outlives the dock closing and switching away from this email.
  $effect(() => {
    void askTurns.length;
    if (askKey !== null) aiChat.save(askKey, $state.snapshot(askTurns));
  });

  // Swap the visible chat for the target message's cached one when the AI
  // target changes (thread switch, focus change). The outgoing chat is already
  // in the cache via the effect above, so nothing is lost.
  $effect(() => {
    const id = replyTarget?.id ?? null;
    if (id === askKey) return;
    cancelAi?.();
    aiPanel = null;
    askQuestion = "";
    askKey = id;
    askTurns = id === null ? [] : aiChat.get(id);
  });

  // Toggle the (empty) chat for a free-form question: pressing Ask again (button
  // or the Q shortcut) closes the dock it opened.
  function openAsk() {
    if (askOpen) {
      closeAiPanel();
      return;
    }
    askOpen = true;
    askQuestion = "";
    queueMicrotask(() => askInput?.focus());
  }

  // Send a question through the email chat. Shared by the input form and the
  // quick-prompt buttons; guards against an empty/target-less/in-flight send.
  function sendAsk(question: string) {
    const target = replyTarget;
    if (!question || !target) return;
    if (aiPanel?.status === "streaming") return;
    askTurns.push({ role: "user", content: question });
    askQuestion = "";
    startAi({ messageId: target.id, turns: $state.snapshot(askTurns) });
  }

  function submitAsk(ev: SubmitEvent) {
    ev.preventDefault();
    sendAsk(askQuestion.trim());
  }

  // A quick-prompt button (Summarize / Translate): open the chat and fire a
  // canned prompt; the answer is a normal turn the user can follow up on.
  function quickPrompt(question: string) {
    askOpen = true;
    sendAsk(question);
    queueMicrotask(() => askInput?.focus());
  }

  let detail = $state<ThreadDetail | null>(null);
  let bodies = $state<Record<number, RenderedBody | "loading" | "error">>({});
  // Per-message unsubscribe state: true once the user clicks the chip.
  let unsubscribed = $state<Record<number, boolean>>({});
  let loadedFor = $state<string | null>(null);
  // The focused (fully open) message in the conversation. Reply/AI actions
  // target it; newer messages collapse above it, older ones below. Defaults to
  // the latest-in-folder message on thread load.
  let focusedId = $state<number | null>(null);
  // Accordion open state for the two collapsed sections around the focused one.
  let laterOpen = $state(false);
  let earlierOpen = $state(false);
  let focusedEl = $state<HTMLDivElement | undefined>();

  // The newest message of the thread IN THE CURRENT FOLDER — the default focus,
  // the flat-view fallback, and the read/loadKey anchor.
  const latest = $derived.by(() => {
    const msgs = detail?.messages ?? [];
    if (msgs.length === 0) return null;
    const inFolder = msgs.filter((m) => m.folderId === mail.selectedFolderId);
    if (inFolder.length > 0) return inFolder[inFolder.length - 1];
    return msgs[msgs.length - 1];
  });

  // Conversation view: the whole back-and-forth as a chat. Off in flat mode or
  // when a specific message was picked from a flat list.
  const conversation = $derived(mail.groupThreads && mail.selectedMessageId === null);

  // The message currently open in the pane (conversation view).
  const focused = $derived.by(() => {
    const msgs = detail?.messages ?? [];
    return msgs.find((m) => m.id === focusedId) ?? latest;
  });

  // Whole thread, newest first. Split around the focused message: `newer` sits
  // collapsed above it ("later in thread"), `older` collapsed below ("earlier").
  const ordered = $derived.by(() => [...(detail?.messages ?? [])].reverse());
  const focusIdx = $derived(ordered.findIndex((m) => m.id === focused?.id));
  const newer = $derived(focusIdx < 0 ? [] : ordered.slice(0, focusIdx));
  const older = $derived(focusIdx < 0 ? [] : ordered.slice(focusIdx + 1));

  // The single message shown when not in conversation view: the one picked from
  // a flat list, else the newest in folder.
  const shown = $derived.by(() => {
    const msgs = detail?.messages ?? [];
    if (mail.selectedMessageId !== null) {
      return msgs.find((m) => m.id === mail.selectedMessageId) ?? latest;
    }
    return latest;
  });

  // A message is outgoing if it's from the account owner (his own reply). The
  // unified view owns every connected address.
  function isOutgoing(m: MessageMeta): boolean {
    return mail.myEmails.includes(m.from.addr.toLowerCase());
  }

  // The message reply/AI actions target: focused in conversation, else shown.
  const replyTarget = $derived(conversation ? focused : shown);

  // Reply-all only makes sense with more than one other party (sender + other
  // recipients besides me). With a single correspondent it equals Reply.
  const canReplyAll = $derived.by(() => {
    const m = replyTarget;
    if (!m) return false;
    const mine = mail.myEmails;
    const others = new Set<string>();
    for (const a of [m.from, ...m.to, ...m.cc]) {
      const addr = a.addr?.toLowerCase();
      if (addr && !mine.includes(addr)) others.add(addr);
    }
    return others.size > 1;
  });

  // Open a different message from the chain. Resets AI context to the new one.
  function setFocus(id: number) {
    if (id === focusedId) return;
    closeAiPanel();
    focusedId = id;
  }

  // Fetch on demand the body of the open message (focused in conversation view,
  // or the single shown message in flat view). Collapsed rows need only snippets.
  $effect(() => {
    const m = conversation ? focused : shown;
    if (m && bodies[m.id] === undefined) void loadBody(m.id);
  });

  // Keep the open message in view when navigating the chain.
  $effect(() => {
    void focusedId;
    if (conversation) focusedEl?.scrollIntoView({ block: "nearest" });
  });

  // Expose the AI chat to the global keyboard handler (Q) in App.svelte. The
  // closure reads the current reactive state, so a single registration stays
  // correct across thread changes.
  $effect(() => {
    ui.setReadingAi({ ask: openAsk });
    return () => ui.setReadingAi(null);
  });

  // Publish the open message so the palette AI chat can pick it up as context.
  $effect(() => {
    ui.setOpenMessage(replyTarget?.id ?? null);
    return () => ui.setOpenMessage(null);
  });

  // Reload when the thread changes OR when the selected thread gains a new
  // message. messageCount/date on the row advance via refreshThreads() on the
  // `mail:updated` event, so this reacts to new mail landing in the thread that
  // is already open — showing the newest message and marking it read without a
  // re-click.
  const loadKey = $derived.by(() => {
    const id = mail.selectedThreadId;
    if (id === null) return null;
    const row = mail.selectedThread;
    return `${id}:${row?.messageCount ?? 0}:${row?.date ?? 0}`;
  });

  $effect(() => {
    const key = loadKey;
    if (key === null) {
      detail = null;
      loadedFor = null;
      return;
    }
    if (key === loadedFor) return;
    loadedFor = key;
    void loadThread(mail.selectedThreadId!);
  });

  async function loadThread(threadId: number) {
    detail = null;
    bodies = {};
    cancelAi?.();
    aiPanel = null;
    askOpen = false;
    askExpanded = false;
    askQuestion = "";
    // askTurns is not reset here: the target-sync effect loads the new email's
    // cached chat once its focused message settles.
    try {
      const d = await api.getThread(threadId);
      if (mail.selectedThreadId !== threadId) return;
      detail = d;
      // Focus the latest-in-folder message (the one just opened). Newer messages
      // collapse above it, older ones below; a side's accordion opens by default
      // only when it hides unread mail. Body loading follows `focused`/`shown`.
      const msgs = d.messages;
      const inFolder = msgs.filter((m) => m.folderId === mail.selectedFolderId);
      const topId = (inFolder.length ? inFolder[inFolder.length - 1] : msgs[msgs.length - 1])?.id ?? null;
      focusedId = topId;
      const orderedNow = [...msgs].reverse();
      const idx = orderedNow.findIndex((m) => m.id === topId);
      laterOpen = idx > 0 && orderedNow.slice(0, idx).some((m) => !m.isRead);
      earlierOpen = idx >= 0 && orderedNow.slice(idx + 1).some((m) => !m.isRead);

      const unread = d.messages.filter((m) => !m.isRead).map((m) => m.id);
      if (unread.length > 0) {
        mail.patchThreadRow(threadId, { isRead: true });
        void api.markRead(unread, true);
      }
    } catch {
      detail = null;
    }
  }

  async function loadBody(messageId: number, showImages?: boolean) {
    bodies = { ...bodies, [messageId]: "loading" };
    try {
      const body = await api.getMessageBody(messageId, showImages);
      bodies = { ...bodies, [messageId]: body };
    } catch {
      bodies = { ...bodies, [messageId]: "error" };
    }
  }

  async function allowSender(messageId: number, addr: string | null) {
    if (addr) await api.allowRemoteImages(addr);
    void loadBody(messageId, true);
  }

  async function allowAllImages(messageId: number) {
    await api.setSetting("images_policy", "always");
    void loadBody(messageId, true);
  }

  const allIds = $derived(detail?.messages.map((m) => m.id) ?? []);
  const anyStarred = $derived(detail?.messages.some((m) => m.isStarred) ?? false);
  // Read state tracks the visible thread row so the button and the global U
  // shortcut (App.svelte) always agree. Opening a thread auto-marks it read,
  // so this is normally true while the pane is shown.
  const isRead = $derived(mail.selectedThread?.isRead ?? true);

  function archive() {
    if (!detail) return;
    const threadId = detail.id;
    const ids = allIds;
    mail.removeThreadFromList(threadId);
    void api.archiveMessages(ids);
  }

  function remove() {
    if (!detail) return;
    const threadId = detail.id;
    const ids = allIds;
    mail.removeThreadFromList(threadId);
    void api.deleteMessages(ids);
  }

  function reportSpam() {
    if (!detail) return;
    const threadId = detail.id;
    const ids = allIds;
    mail.removeThreadFromList(threadId);
    void api.reportSpam(ids);
  }

  function unsubscribe(id: number) {
    if (unsubscribed[id]) return;
    unsubscribed[id] = true; // optimistic; the backend queues the actual op
    void api.unsubscribe(id).catch(() => {
      unsubscribed[id] = false; // let the user try again if it never queued
    });
  }

  function toggleStar() {
    if (!detail) return;
    const on = !anyStarred;
    detail = {
      ...detail,
      messages: detail.messages.map((m) => ({ ...m, isStarred: on })),
    };
    mail.patchThreadRow(detail.id, { isStarred: on });
    void api.setStarred(allIds, on);
  }

  function toggleRead() {
    if (!detail) return;
    const next = !isRead;
    mail.patchThreadRow(detail.id, { isRead: next });
    void api.markRead(allIds, next);
  }

  function initial(name: string | null): string {
    return (name ?? "?").charAt(0).toUpperCase() || "?";
  }

  function formatFull(unix: number): string {
    return new Date(unix * 1000).toLocaleString(getLocale(), {
      month: "short",
      day: "numeric",
      hour: "numeric",
      minute: "2-digit",
    });
  }

  function recipients(m: MessageMeta): string {
    const all = [...m.to, ...m.cc];
    if (all.length === 0) return t("reading.to_me");
    return all.map((a) => a.name || a.addr).join(", ");
  }

  async function reply(mode: "reply" | "reply_all" | "forward") {
    const target = replyTarget;
    if (!target) return;
    const draft = await api.getReplyTemplate(target.id, mode);
    await api.openComposeWindow(draft.id);
  }
</script>

<section class="pane">
  {#if !detail}
    {#if ui.temperature === "warm" && mail.selectedThreadId === null}
      <!-- Quiet-zine empty state: a taped paper note. Warm themes only. -->
      <div class="placeholder">
        <div class="note">
          <div class="note-paper">
            <svg class="note-icon" width="52" height="52" viewBox="0 0 48 48" fill="none" stroke="currentColor" stroke-width="2.1" stroke-linejoin="round" stroke-linecap="round" aria-hidden="true">
              <defs>
                <filter id="note-marker" x="-30%" y="-30%" width="160%" height="160%">
                  <feTurbulence type="fractalNoise" baseFrequency="0.85" numOctaves="2" seed="4" result="n" />
                  <feDisplacementMap in="SourceGraphic" in2="n" scale="1.8" />
                </filter>
              </defs>
              <g filter="url(#note-marker)">
                <rect x="6" y="12" width="36" height="24" rx="2.5" />
                <path d="M6.5 15l17.5 12.5L41.5 15" />
              </g>
            </svg>
            <div class="note-line">{t("reading.no_selection")}</div>
            <div class="note-hint">
              <kbd>J</kbd><kbd>K</kbd><span>{t("reading.hint_browse")}</span>
            </div>
            <div class="note-hint">
              <kbd>Ctrl</kbd><kbd>K</kbd><span>— {t("reading.hint_command")}</span>
            </div>
          </div>
          <span class="tape"></span>
        </div>
      </div>
    {:else}
      <div class="placeholder">
        <div class="ghost">✉</div>
        {mail.selectedThreadId === null ? t("reading.no_selection") : t("reading.loading")}
      </div>
    {/if}
  {:else}
    <header class="toolbar">
      <div class="spacer"></div>
      <button class="tool" onclick={archive} title={t("reading.archive")}>
        <svg width="15" height="15" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.2"><path d="M2 3h12v3H2V3zm1 3v7h10V6M6.5 9h3" /></svg>
        <kbd>E</kbd>
      </button>
      <button class="tool" onclick={remove} title={t("reading.delete")}>
        <svg width="15" height="15" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.2"><path d="M3 4h10M6.5 4V2.5h3V4M4.5 4l.5 9.5h6l.5-9.5M6.7 6.5v5M9.3 6.5v5" /></svg>
        <kbd>Del</kbd>
      </button>
      <button class="tool" onclick={reportSpam} title={t("reading.spam")}>
        <!-- Warning octagon: junk / report spam. -->
        <svg width="15" height="15" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.2"><path d="M5.4 1.8h5.2l3.6 3.6v5.2l-3.6 3.6H5.4L1.8 10.6V5.4L5.4 1.8z" /><path d="M8 4.6v4M8 11.1v.1" /></svg>
        <kbd>!</kbd>
      </button>
      <button class="tool" class:starred={anyStarred} onclick={toggleStar} title={anyStarred ? t("reading.unstar") : t("reading.star")}>
        <svg width="15" height="15" viewBox="0 0 16 16" fill={anyStarred ? "currentColor" : "none"} stroke="currentColor" stroke-width="1.2"><path d="M8 1.5l2 4.1 4.5.6-3.3 3.2.8 4.5L8 11.8l-4 2.1.8-4.5L1.5 6.2 6 5.6 8 1.5z" /></svg>
        <kbd>S</kbd>
      </button>
      <button class="tool" onclick={toggleRead} title={isRead ? t("reading.mark_unread") : t("reading.mark_read")}>
        {#if isRead}
          <!-- Sealed envelope: click to mark unread. -->
          <svg width="15" height="15" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.2"><rect x="2" y="3.5" width="12" height="9" rx="1" /><path d="M2 5l6 4.5L14 5" /></svg>
        {:else}
          <!-- Open envelope: click to mark read. -->
          <svg width="15" height="15" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.2"><path d="M2 6.5l6-4 6 4v6a1 1 0 0 1-1 1H3a1 1 0 0 1-1-1v-6z" /><path d="M2 6.5l6 4.5 6-4.5" /></svg>
        {/if}
        <kbd>U</kbd>
      </button>
    </header>

    <div class="scroll" class:hidden={askExpanded}>
      <h1 class="subject">{detail.subject || "—"}</h1>

      {#if conversation}
        {#if newer.length > 0}
          <div class="thread-more">
            <button
              class="more-toggle"
              onclick={() => (laterOpen = !laterOpen)}
              aria-expanded={laterOpen}
            >
              <span class="chev" class:open={laterOpen}>▸</span>
              {t("reading.later", { n: newer.length })}
            </button>
            {#if laterOpen}
              <div class="convo">
                {#each newer as m (m.id)}
                  {@render chatRow(m)}
                {/each}
              </div>
            {/if}
          </div>
        {/if}

        {#if focused}
          <div bind:this={focusedEl}>
            {@render messageBlock(focused, bodies[focused.id])}
          </div>
        {/if}

        {#if older.length > 0}
          <div class="thread-more">
            <button
              class="more-toggle"
              onclick={() => (earlierOpen = !earlierOpen)}
              aria-expanded={earlierOpen}
            >
              <span class="chev" class:open={earlierOpen}>▸</span>
              {t("reading.earlier", { n: older.length })}
            </button>
            {#if earlierOpen}
              <div class="convo">
                {#each older as m (m.id)}
                  {@render chatRow(m)}
                {/each}
              </div>
            {/if}
          </div>
        {/if}
      {:else if shown}
        {@render messageBlock(shown, bodies[shown.id])}
      {/if}
    </div>

    {#if askOpen}
      <!-- AI dock sits above the actions so it's visible at any scroll position. -->
      <div class="ai-dock" class:expanded={askExpanded}>
        <div class="dock-tools">
          <button
            class="dock-btn"
            onclick={() => (askExpanded = !askExpanded)}
            title={askExpanded ? t("ai.collapse") : t("ai.expand")}
            aria-label={askExpanded ? t("ai.collapse") : t("ai.expand")}
            aria-pressed={askExpanded}
          >
            {#if askExpanded}
              <svg width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1.2"><path d="M5 1v3.5a.5.5 0 0 1-.5.5H1M7 11V7.5a.5.5 0 0 1 .5-.5H11M1 7.5h3.5a.5.5 0 0 1 .5.5V11M11 4.5H7.5a.5.5 0 0 1-.5-.5V1" /></svg>
            {:else}
              <svg width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1.2"><path d="M4.5 1H1v3.5M11 4.5V1H7.5M7.5 11H11V7.5M1 7.5V11h3.5" /></svg>
            {/if}
          </button>
          <button class="dock-btn" onclick={closeAiPanel} aria-label={t("a11y.close")}>
            <svg width="9" height="9" viewBox="0 0 10 10"><path d="M0 0L10 10M10 0L0 10" stroke="currentColor" stroke-width="1.2" /></svg>
          </button>
        </div>
        {#if askTurns.length > 0 || aiPanel}
          <div class="ask-thread" bind:this={askThreadEl}>
            {#each askTurns as turn (turn)}
              {#if turn.role === "user"}
                <div class="ask-q">{turn.content}</div>
              {:else}
                <div class="ai-card">
                  <div class="ai-label microlabel">{t("ai.answer")}</div>
                  <div class="ai-text md-body" use:aiLinks>{@html mdLite(turn.content)}</div>
                </div>
              {/if}
            {/each}
            {#if aiPanel}
              <div class="ai-card" class:error={aiPanel.status === "error"}>
                <div class="ai-label microlabel">{t("ai.answer")}</div>
                {#if aiPanel.steps.length > 0}
                  <div class="ai-steps">
                    {#each aiPanel.steps as step (step.id)}
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
                {#if aiPanel.text === "" && aiPanel.status === "streaming"}
                  <span class="thinking">{t("ai.thinking")}</span>
                {:else}
                  <div class="ai-text md-body" use:aiLinks>{@html mdLite(aiPanel.text)}</div>
                {/if}
              </div>
            {/if}
          </div>
        {/if}
        <form class="ask-form" onsubmit={submitAsk}>
          <span class="ai-spark">✦</span>
          <input
            bind:this={askInput}
            bind:value={askQuestion}
            placeholder={askTurns.length > 0 ? t("ai.ask_followup") : t("ai.ask_placeholder")}
            spellcheck="false"
          />
        </form>
        <div class="ask-quick">
          <button class="quick-btn" onclick={() => quickPrompt(t("ai.prompt_summarize"))}>
            {t("ai.summarize")}
          </button>
          <button class="quick-btn" onclick={() => quickPrompt(t("ai.prompt_translate"))}>
            {t("ai.translate")}
          </button>
        </div>
      </div>
    {/if}

    <footer class="actions">
      {#if ai.keyPresent}
        <button class="ai-btn" onclick={openAsk}>✦ {t("ai.ask")}<kbd>Q</kbd></button>
      {/if}
      <button class="btn" onclick={() => reply("reply")}>{t("reading.reply")}<kbd>R</kbd></button>
      {#if canReplyAll}
        <button class="btn" onclick={() => reply("reply_all")}>{t("reading.reply_all")}<kbd>A</kbd></button>
      {/if}
      <button class="btn" onclick={() => reply("forward")}>{t("reading.forward")}<kbd>F</kbd></button>
    </footer>
  {/if}
</section>

{#snippet messageBlock(message: MessageMeta, body: RenderedBody | "loading" | "error" | undefined)}
  <article class="message">
    <div class="meta">
      <span class="avatar">{initial(message.from.name ?? message.from.addr)}</span>
      <div class="who">
        <div class="from">
          {message.from.name ?? message.from.addr}
          <span class="addr">&lt;{message.from.addr}&gt;</span>
        </div>
        <div class="microlabel">{recipients(message)}</div>
      </div>
      {#if message.canUnsubscribe}
        {#if unsubscribed[message.id]}
          <span class="unsub done">{t("reading.unsubscribed")} ✓</span>
        {:else}
          <button class="unsub" onclick={() => unsubscribe(message.id)}>
            {t("reading.unsubscribe")}
          </button>
        {/if}
      {/if}
      <span class="date microlabel">{formatFull(message.date)}</span>
    </div>

    {#if body === "loading" || body === undefined}
      <div class="body-note">{t("reading.loading")}</div>
    {:else if body === "error"}
      <div class="body-note">
        {t("reading.load_failed")}
        <button class="linkish" onclick={() => loadBody(message.id)}>{t("reading.retry")}</button>
      </div>
    {:else}
      {#if body.blockedImages > 0}
        <div class="images-bar">
          {t("reading.images_blocked", { n: body.blockedImages })}
          <button class="linkish" onclick={() => loadBody(message.id, true)}>
            {t("reading.show_once")}
          </button>
          {#if body.fromAddr}
            <span class="sep">·</span>
            <button class="linkish" onclick={() => allowSender(message.id, body.fromAddr)}>
              {t("reading.always_sender")}
            </button>
          {/if}
          <span class="sep">·</span>
          <button class="linkish" onclick={() => allowAllImages(message.id)}>
            {t("reading.always_all")}
          </button>
        </div>
      {/if}
      {#if body.invite}
        <InviteCard
          invite={body.invite}
          onRsvp={(response) => api.rsvpInvite(message.id, response)}
          onAddToCalendar={() => api.openInviteIcs(message.id)}
        />
      {/if}
      {#if body.invite && body.invite.method !== "reply"}
        <!-- The card says it all; the sender's verbose HTML (Google's
             banner etc.) stays one click away for Meet links & co. -->
        {#if body.html}
          <details class="orig-body">
            <summary class="linkish">{t("invite.show_original")}</summary>
            <div class="body">
              <HtmlViewer html={body.html} />
            </div>
          </details>
        {/if}
      {:else}
        <div class="body">
          <HtmlViewer html={body.html} />
        </div>
      {/if}
      {#if body.attachments.length > 0}
        <AttachmentChips attachments={body.attachments} />
      {/if}
    {/if}
  </article>
{/snippet}

{#snippet chatRow(m: MessageMeta)}
  <button
    class="chat-row"
    class:outgoing={isOutgoing(m)}
    class:unread={!m.isRead}
    onclick={() => setFocus(m.id)}
  >
    <span class="avatar sm">{initial(m.from.name ?? m.from.addr)}</span>
    <div class="chat-bubble">
      <div class="chat-head">
        <span class="chat-name">
          {isOutgoing(m) ? t("reading.you") : (m.from.name ?? m.from.addr)}
        </span>
        <span class="chat-date">{formatFull(m.date)}</span>
      </div>
      <div class="chat-snippet">{m.snippet}</div>
    </div>
  </button>
{/snippet}

<style>
  .pane {
    flex: 1;
    display: flex;
    flex-direction: column;
    background: var(--surface);
    min-width: 0;
  }

  .placeholder {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 12px;
    color: var(--text-faint);
    font-size: 13px;
  }
  .ghost {
    font-size: 28px;
    opacity: 0.4;
  }
  /* Quiet-zine empty state — a paper note pinned with tape. Only mounts in warm
     themes, so it can use theme tokens directly without gating.
     .note is an unclipped wrapper: it carries the rotation and hosts the tape as
     a sibling of .note-paper, so the paper's clip-path (torn edge) doesn't cut
     the tape off. */
  .note {
    position: relative;
    width: 340px;
    max-width: 80%;
    transform: rotate(-1.4deg);
  }
  .note-paper {
    background: var(--surface-raised);
    border: 1px solid var(--hairline);
    box-shadow: 4px 7px 18px rgba(0, 0, 0, 0.22);
    padding: 36px 28px 28px;
    color: var(--text);
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 12px;
    text-align: center;
    clip-path: polygon(
      0 2%,
      4% 0,
      46% 2%,
      73% 0,
      100% 2%,
      99% 46%,
      100% 75%,
      98% 100%,
      57% 98%,
      25% 100%,
      2% 99%,
      0 55%
    );
  }
  .note .tape {
    position: absolute;
    top: -13px;
    left: 50%;
    transform: translateX(-50%) rotate(-4deg);
    width: 112px;
    height: 28px;
    background: rgba(216, 190, 120, 0.5);
    box-shadow: 0 2px 5px rgba(0, 0, 0, 0.16);
    z-index: 1;
  }
  .note-icon {
    color: var(--text);
    opacity: 0.5;
  }
  .note-line {
    font-size: 20px;
    font-weight: 600;
    font-style: italic;
    line-height: 1.15;
    color: var(--text);
  }
  .note-hint {
    display: flex;
    align-items: center;
    gap: 7px;
    flex-wrap: wrap;
    justify-content: center;
    font-style: italic;
    font-size: 14px;
    color: var(--text-dim);
  }
  .note-hint kbd {
    font-family: var(--font-mono);
    font-style: normal;
    font-size: 11px;
    color: var(--text);
    border: 1.4px solid var(--text);
    box-shadow: 1.5px 1.5px 0 var(--text);
    border-radius: 3px;
    padding: 1px 6px;
  }
  .note-hint:first-of-type kbd:first-child {
    transform: rotate(-2deg);
  }
  .note-hint:first-of-type kbd:nth-child(2) {
    transform: rotate(2deg);
  }
  @media (prefers-reduced-motion: reduce) {
    .note,
    .note .tape,
    .note-hint kbd {
      transform: none;
    }
    .note .tape {
      transform: translateX(-50%);
    }
  }

  .toolbar {
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 8px 20px 0;
  }
  .spacer {
    flex: 1;
  }
  .tool {
    height: 32px;
    padding: 0 8px;
    display: flex;
    align-items: center;
    gap: 5px;
    border-radius: var(--radius-s);
    color: var(--text-dim);
  }
  .tool:hover {
    background: var(--hover);
    color: var(--text);
  }
  .tool.starred {
    color: var(--text);
  }
  kbd {
    font-family: var(--font-mono);
    font-size: 10px;
    color: var(--text-faint);
  }
  .btn kbd,
  .ai-btn kbd {
    margin-left: 6px;
  }

  .scroll {
    flex: 1;
    overflow-y: auto;
    padding: 8px 36px 28px;
    max-width: 840px;
    width: 100%;
  }

  .subject {
    font-size: 21px;
    font-weight: 800;
    letter-spacing: -0.02em;
    line-height: 1.25;
    margin-bottom: 8px;
  }

  .message {
    border-bottom: 1px solid var(--hairline);
    padding-bottom: 14px;
    margin-bottom: 6px;
  }
  .message:last-child {
    border-bottom: none;
  }

  /* ---- Conversation (chat) view ---- */
  .thread-more {
    margin-top: 8px;
  }
  .more-toggle {
    display: flex;
    align-items: center;
    gap: 7px;
    padding: 8px 4px;
    color: var(--text-dim);
    font-size: 12.5px;
    font-weight: 600;
  }
  .more-toggle:hover {
    color: var(--text);
  }
  .chev {
    font-size: 10px;
    transition: transform 0.12s;
    display: inline-block;
  }
  .chev.open {
    transform: rotate(90deg);
  }
  .convo {
    display: flex;
    flex-direction: column;
    gap: 4px;
    margin-top: 4px;
  }
  .chat-row {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    width: 100%;
    text-align: left;
    padding: 6px 4px;
    transition: opacity 0.08s;
  }
  .avatar.sm {
    width: 26px;
    height: 26px;
    font-size: 11px;
  }
  .chat-bubble {
    flex: 1;
    min-width: 0;
    background: var(--hover);
    border: 1px solid var(--hairline);
    border-radius: 12px;
    padding: 7px 11px;
    transition: border-color 0.08s;
  }
  .chat-row:hover .chat-bubble {
    border-color: var(--hairline-strong);
  }
  .chat-head {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    gap: 8px;
  }
  .chat-name {
    font-weight: 600;
    font-size: 12.5px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .chat-row.unread .chat-name {
    font-weight: 800;
  }
  .chat-date {
    font-family: var(--font-mono);
    font-size: 10px;
    color: var(--text-faint);
    flex-shrink: 0;
  }
  .chat-snippet {
    font-size: 12.5px;
    color: var(--text-faint);
    margin-top: 2px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  /* Outgoing (your own replies): mirrored to the right, tinted. */
  .chat-row.outgoing {
    flex-direction: row-reverse;
  }
  .chat-row.outgoing .chat-bubble {
    background: var(--selected);
  }

  .meta {
    display: flex;
    align-items: center;
    gap: 12px;
    margin-top: 12px;
    width: 100%;
    text-align: left;
  }
  .avatar {
    width: 34px;
    height: 34px;
    border-radius: 50%;
    background: var(--selected);
    display: grid;
    place-items: center;
    font-weight: 700;
    font-size: 13px;
    flex-shrink: 0;
  }
  .who {
    flex: 1;
    min-width: 0;
  }
  .from {
    font-weight: 600;
    font-size: 13.5px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .addr {
    color: var(--text-faint);
    font-weight: 400;
  }
  .who .microlabel {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    text-transform: none;
    letter-spacing: 0.02em;
    font-size: 11px;
  }
  .date {
    flex-shrink: 0;
  }

  /* Quiet, neutral chip — not the AI accent, not danger red. Shows only on
     mailing-list mail, tied to the sender it unsubscribes from. */
  .unsub {
    flex-shrink: 0;
    font-size: 12px;
    line-height: 1.4;
    padding: 1px 8px;
    border: 1px solid var(--hairline);
    border-radius: var(--radius-s);
    color: var(--text-dim);
    white-space: nowrap;
  }
  button.unsub:hover {
    background: var(--hover);
    color: var(--text);
  }
  .unsub.done {
    color: var(--success);
    border-color: transparent;
  }

  .images-bar {
    margin-top: 12px;
    padding: 8px 12px;
    border: 1px solid var(--hairline-strong);
    border-radius: var(--radius-s);
    font-size: 12.5px;
    color: var(--text-dim);
    display: flex;
    gap: 8px;
    flex-wrap: wrap;
    align-items: center;
  }
  .linkish {
    color: var(--text);
    text-decoration: underline;
    text-underline-offset: 3px;
    font-size: 12.5px;
  }

  .body {
    margin-top: 14px;
  }
  .orig-body {
    margin-top: 12px;
  }
  .orig-body summary {
    cursor: pointer;
    color: var(--text-dim);
    width: fit-content;
  }
  .orig-body summary::-webkit-details-marker {
    display: none;
  }
  .body-note {
    margin-top: 14px;
    color: var(--text-faint);
    font-size: 13px;
    display: flex;
    gap: 10px;
  }

  .actions {
    display: flex;
    flex-wrap: nowrap;
    align-items: center;
    gap: 6px;
    padding: 10px 36px 12px;
    border-top: 1px solid var(--hairline);
  }
  .ai-btn {
    padding: 7px 16px;
    border-radius: var(--radius-m);
    border: 1px solid var(--accent-dim);
    color: var(--accent);
    font-size: 13px;
    font-weight: 600;
    white-space: nowrap;
  }
  .ai-btn:hover {
    background: var(--accent-soft);
  }
  .btn {
    padding: 7px 16px;
    border-radius: var(--radius-m);
    border: 1px solid var(--hairline-strong);
    color: var(--text);
    font-size: 13px;
    font-weight: 600;
    white-space: nowrap;
  }
  .btn:hover {
    background: var(--hover);
    border-color: var(--text-faint);
  }

  .ai-dock {
    position: relative;
    border-top: 1px solid var(--hairline);
    padding: 12px 36px;
    max-height: 38vh;
    overflow-y: auto;
    flex-shrink: 0;
  }
  /* Expanded: the dock takes over the pane so a long chat has room to breathe. */
  .ai-dock.expanded {
    flex: 1;
    max-height: none;
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .scroll.hidden {
    display: none;
  }
  .dock-tools {
    position: absolute;
    top: 10px;
    right: 14px;
    display: flex;
    gap: 2px;
    z-index: 1;
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
  .ai-dock.expanded .ask-thread {
    flex: 1;
    max-height: none;
    min-height: 0;
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
  .sep {
    color: var(--text-faint);
  }
</style>
