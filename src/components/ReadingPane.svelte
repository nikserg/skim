<script lang="ts">
  import { aiStream, api } from "../lib/api";
  import { getLocale, t } from "../lib/i18n/index.svelte";
  import { mdLite } from "../lib/md";
  import { ai } from "../lib/stores/ai.svelte";
  import { mail } from "../lib/stores/mail.svelte";
  import { ui } from "../lib/stores/ui.svelte";
  import type { MessageMeta, RenderedBody, ThreadDetail } from "../lib/types";
  import AttachmentChips from "./AttachmentChips.svelte";
  import HtmlViewer from "./HtmlViewer.svelte";

  // ---- AI panel ----
  type AiPanelKind = "summary" | "ask";
  let aiPanel = $state<{
    kind: AiPanelKind;
    status: "streaming" | "done" | "error";
    text: string;
  } | null>(null);
  let askOpen = $state(false);
  let askQuestion = $state("");
  let askInput: HTMLInputElement | undefined = $state();
  let cancelAi: (() => void) | null = null;

  function closeAiPanel() {
    cancelAi?.();
    aiPanel = null;
    askOpen = false;
  }

  function startAi(kind: AiPanelKind, command: "ai_summarize" | "ai_ask", args: Record<string, unknown>) {
    cancelAi?.();
    aiPanel = { kind, status: "streaming", text: "" };
    cancelAi = aiStream(command, args, {
      delta: (text) => {
        if (aiPanel) aiPanel = { ...aiPanel, text: aiPanel.text + text };
      },
      done: () => {
        if (aiPanel) aiPanel = { ...aiPanel, status: "done" };
      },
      error: (code, message) => {
        aiPanel = {
          kind,
          status: "error",
          text: code === "ai_key" ? t("ai.needs_key") : message || t("ai.no_context"),
        };
      },
    });
  }

  function summarize() {
    if (!detail) return;
    askOpen = false;
    startAi("summary", "ai_summarize", { threadId: detail.id });
  }

  function openAsk() {
    askOpen = true;
    askQuestion = "";
    queueMicrotask(() => askInput?.focus());
  }

  function submitAsk(ev: SubmitEvent) {
    ev.preventDefault();
    if (!latest || !askQuestion.trim()) return;
    startAi("ask", "ai_ask", { messageId: latest.id, question: askQuestion.trim() });
  }

  async function aiDraftReply() {
    if (!latest) return;
    const draft = await api.getReplyTemplate(latest.id, "reply");
    await api.openComposeWindow(draft.id);
  }

  let detail = $state<ThreadDetail | null>(null);
  let bodies = $state<Record<number, RenderedBody | "loading" | "error">>({});
  let loadedFor = $state<number | null>(null);
  // Minimalism: the pane shows only the newest message of the thread.
  // Outside the inbox it's the newest message IN THIS folder — in Sent you
  // want the email you sent, not the reply that came back.
  const latest = $derived.by(() => {
    const msgs = detail?.messages ?? [];
    if (msgs.length === 0) return null;
    if (mail.selectedFolder && mail.selectedFolder.role !== "inbox") {
      const inFolder = msgs.filter((m) => m.folderId === mail.selectedFolderId);
      if (inFolder.length > 0) return inFolder[inFolder.length - 1];
    }
    return msgs[msgs.length - 1];
  });

  // Fetch the shown message's body on demand.
  $effect(() => {
    const m = latest;
    if (m && bodies[m.id] === undefined) void loadBody(m.id);
  });

  let aiRowOpen = $state(localStorage.getItem("skim.aiRowOpen") !== "0");
  function toggleAiRow() {
    aiRowOpen = !aiRowOpen;
    localStorage.setItem("skim.aiRowOpen", aiRowOpen ? "1" : "0");
  }

  $effect(() => {
    const threadId = mail.selectedThreadId;
    if (threadId === null) {
      detail = null;
      loadedFor = null;
      return;
    }
    if (threadId === loadedFor) return;
    loadedFor = threadId;
    void loadThread(threadId);
  });

  async function loadThread(threadId: number) {
    detail = null;
    bodies = {};
    cancelAi?.();
    aiPanel = null;
    askOpen = false;
    try {
      const d = await api.getThread(threadId);
      if (mail.selectedThreadId !== threadId) return;
      detail = d;
      // Body loading follows `latest` via the effect above.

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

  function markUnread() {
    if (!detail) return;
    mail.patchThreadRow(detail.id, { isRead: false });
    void api.markRead(allIds, false);
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
    if (!latest) return;
    const draft = await api.getReplyTemplate(latest.id, mode);
    await api.openComposeWindow(draft.id);
  }
</script>

<section class="pane">
  {#if !detail}
    <div class="placeholder">
      <div class="ghost">✉</div>
      {mail.selectedThreadId === null ? t("reading.no_selection") : t("reading.loading")}
    </div>
  {:else}
    <header class="toolbar">
      <div class="spacer"></div>
      <button class="tool" onclick={archive} title={t("reading.archive")}>
        <svg width="15" height="15" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.2"><path d="M2 3h12v3H2V3zm1 3v7h10V6M6.5 9h3" /></svg>
      </button>
      <button class="tool" onclick={remove} title={t("reading.delete")}>
        <svg width="15" height="15" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.2"><path d="M3 4h10M6.5 4V2.5h3V4M4.5 4l.5 9.5h6l.5-9.5M6.7 6.5v5M9.3 6.5v5" /></svg>
      </button>
      <button class="tool" class:starred={anyStarred} onclick={toggleStar} title={anyStarred ? t("reading.unstar") : t("reading.star")}>
        <svg width="15" height="15" viewBox="0 0 16 16" fill={anyStarred ? "currentColor" : "none"} stroke="currentColor" stroke-width="1.2"><path d="M8 1.5l2 4.1 4.5.6-3.3 3.2.8 4.5L8 11.8l-4 2.1.8-4.5L1.5 6.2 6 5.6 8 1.5z" /></svg>
      </button>
      <button class="tool" onclick={markUnread} title={t("reading.mark_unread")}>
        <svg width="15" height="15" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.2"><rect x="2" y="3.5" width="12" height="9" rx="1" /><path d="M2 5l6 4.5L14 5" /></svg>
      </button>
    </header>

    <div class="scroll">
      <h1 class="subject">{detail.subject || "—"}</h1>

      {#if latest}
        {@const message = latest}
        {@const body = bodies[latest.id]}
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
            <div class="body">
              <HtmlViewer html={body.html} />
            </div>
            {#if body.attachments.length > 0}
              <AttachmentChips attachments={body.attachments} />
            {/if}
          {/if}
        </article>
      {/if}
    </div>

    {#if askOpen || aiPanel}
      <!-- AI dock sits above the actions so it's visible at any scroll position. -->
      <div class="ai-dock">
        <button class="dock-close" onclick={closeAiPanel} aria-label="Close">
          <svg width="9" height="9" viewBox="0 0 10 10"><path d="M0 0L10 10M10 0L0 10" stroke="currentColor" stroke-width="1.2" /></svg>
        </button>
        {#if askOpen}
          <form class="ask-form" onsubmit={submitAsk}>
            <span class="ai-spark">✦</span>
            <input
              bind:this={askInput}
              bind:value={askQuestion}
              placeholder={t("ai.ask_placeholder")}
              spellcheck="false"
            />
          </form>
        {/if}
        {#if aiPanel}
          <div class="ai-card" class:error={aiPanel.status === "error"}>
            <div class="ai-label microlabel">
              {aiPanel.kind === "summary" ? t("ai.summary") : t("ai.answer")}
            </div>
            {#if aiPanel.text === "" && aiPanel.status === "streaming"}
              <span class="thinking">{t("ai.thinking")}</span>
            {:else}
              <div class="ai-text md-body">{@html mdLite(aiPanel.text)}</div>
            {/if}
          </div>
        {/if}
      </div>
    {/if}

    <footer class="actions">
      {#if aiRowOpen}
        {#if ai.keyPresent}
          <div class="ai-row">
            <button class="ai-btn" onclick={aiDraftReply}>✦ {t("ai.draft_reply")}</button>
            <button class="ai-btn" onclick={summarize}>✦ {t("ai.summarize")}</button>
            <button class="ai-btn" onclick={openAsk}>✦ {t("ai.ask_about")}</button>
          </div>
        {:else}
          <div class="ai-row ai-hint">
            <span class="ai-hint-text">{t("ai.needs_key")}</span>
            <button class="ai-btn" onclick={() => ui.openSettings()}>
              {t("nav.settings")} →
            </button>
          </div>
        {/if}
      {/if}
      <div class="mail-row">
        <button
          class="ai-toggle"
          class:open={aiRowOpen}
          onclick={toggleAiRow}
          title="Skim AI"
          aria-expanded={aiRowOpen}
        >
          ✦
        </button>
        <button class="btn" onclick={() => reply("reply")}>{t("reading.reply")}</button>
        <button class="btn" onclick={() => reply("reply_all")}>{t("reading.reply_all")}</button>
        <button class="btn" onclick={() => reply("forward")}>{t("reading.forward")}</button>
      </div>
    </footer>
  {/if}
</section>

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
    width: 32px;
    height: 32px;
    display: grid;
    place-items: center;
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
  .body-note {
    margin-top: 14px;
    color: var(--text-faint);
    font-size: 13px;
    display: flex;
    gap: 10px;
  }

  .actions {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 10px 36px 12px;
    border-top: 1px solid var(--hairline);
  }
  .ai-row {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .ai-hint {
    align-items: center;
    gap: 10px;
  }
  .ai-hint-text {
    font-size: 12.5px;
    color: var(--text-dim);
    line-height: 1.4;
  }
  .mail-row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
  }
  .ai-btn {
    padding: 6px 12px;
    border-radius: 999px;
    border: 1px solid var(--accent-dim);
    color: var(--accent);
    font-size: 12.5px;
    font-weight: 600;
    white-space: nowrap;
  }
  .ai-btn:hover {
    background: var(--accent-soft);
  }
  .ai-toggle {
    width: 32px;
    height: 32px;
    display: grid;
    place-items: center;
    border-radius: 999px;
    border: 1px solid var(--hairline-strong);
    color: var(--text-faint);
    font-size: 13px;
    flex-shrink: 0;
    transition:
      color 0.12s,
      border-color 0.12s;
  }
  .ai-toggle:hover {
    background: var(--hover);
  }
  .ai-toggle.open {
    color: var(--accent);
    border-color: var(--accent-dim);
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
  .dock-close {
    position: absolute;
    top: 10px;
    right: 14px;
    width: 24px;
    height: 24px;
    display: grid;
    place-items: center;
    border-radius: var(--radius-s);
    color: var(--text-faint);
    z-index: 1;
  }
  .dock-close:hover {
    background: var(--hover);
    color: var(--text);
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
