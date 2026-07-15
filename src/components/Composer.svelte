<script lang="ts">
  // The compose window's root component (mounted for #/compose/{id}).
  import { aiApi, aiStream, api, errorMessage } from "../lib/api";
  import { t } from "../lib/i18n/index.svelte";
  import type { Draft, DraftAttachment } from "../lib/types";
  import AddressInput from "./AddressInput.svelte";

  let { draftId }: { draftId: number } = $props();

  let draft = $state<Draft | null>(null);
  let showCc = $state(false);
  let maximized = $state(false);
  let sending = $state(false);
  let error = $state("");
  let saveTimer: ReturnType<typeof setTimeout> | null = null;

  // ---- Attachments ----
  const MAX_ATTACHMENT_BYTES = 25 * 1024 * 1024;
  let attachments = $state<DraftAttachment[]>([]);
  let dragActive = $state(false);
  let fileInput = $state<HTMLInputElement | null>(null);

  function formatSize(bytes: number): string {
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(0)} KB`;
    return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
  }

  /** Read each file's bytes and stage it on the draft. Shared by the paperclip
   *  button and drag & drop. */
  async function attachFiles(files: FileList | File[] | null) {
    if (!draft || !files) return;
    for (const file of Array.from(files)) {
      if (file.size > MAX_ATTACHMENT_BYTES) {
        error = t("compose.attach_too_large");
        continue;
      }
      try {
        const bytes = Array.from(new Uint8Array(await file.arrayBuffer()));
        const meta = await api.addDraftAttachment(
          draft.id,
          file.name,
          file.type || "application/octet-stream",
          bytes,
        );
        attachments.push(meta);
      } catch (e) {
        error = errorMessage(e);
      }
    }
  }

  async function pickFiles(e: Event) {
    const input = e.currentTarget as HTMLInputElement;
    await attachFiles(input.files);
    input.value = ""; // allow re-picking the same file
  }

  async function removeAttachment(id: number) {
    try {
      await api.removeDraftAttachment(id);
      attachments = attachments.filter((a) => a.id !== id);
    } catch (e) {
      error = errorMessage(e);
    }
  }

  function onDrop(e: DragEvent) {
    e.preventDefault();
    dragActive = false;
    void attachFiles(e.dataTransfer?.files ?? null);
  }

  // ---- AI drafting ----
  let aiAvailable = $state(false);
  let instruction = $state("");

  /** Grow a textarea to fit its content (used by the AI instruction box). */
  function autogrow(node: HTMLTextAreaElement) {
    const resize = () => {
      node.style.height = "auto";
      node.style.height = `${node.scrollHeight}px`;
    };
    resize();
    node.addEventListener("input", resize);
    return { destroy: () => node.removeEventListener("input", resize) };
  }
  let aiBusy = $state(false);
  let cancelAi: (() => void) | null = null;
  /** Quoted original (reply/forward), preserved below AI-generated text. */
  let quotedTail = "";

  // The AI drafting session: one shared conversation the user refines turn by
  // turn. User turns are instructions; assistant turns are the drafts the AI
  // produced. The newest assistant turn mirrors the editable body.
  type ChatTurn = { role: "user" | "assistant"; content: string };
  let convo = $state<ChatTurn[]>([]);
  const userTurns = $derived(convo.filter((tn) => tn.role === "user"));
  const hasDraft = $derived(convo.some((tn) => tn.role === "assistant"));

  // Replies keep the client's automatic "Re:" subject — the AI drafts a subject
  // only for new mail. `subjectAuto` stays true while the subject is AI-owned;
  // a manual edit flips it off so the co-author stops overwriting it.
  const isReply = $derived(draft?.replyToMessageId != null);
  let subjectAuto = $state(true);

  /** Split raw model output into { subject, body }. New-email drafts lead with
   *  a `Subject:` header; if it's absent (or this is a reply) the whole text is
   *  the body and the subject is left untouched. */
  function parseDraft(raw: string): { subject: string | null; body: string } {
    const nl = raw.indexOf("\n");
    const firstLine = nl >= 0 ? raw.slice(0, nl) : raw;
    const m = firstLine.match(/^\s*subject\s*:(.*)$/i);
    if (!m) return { subject: null, body: raw };
    const subject = m[1].trim();
    const body = nl >= 0 ? raw.slice(nl + 1).replace(/^\n+/, "") : "";
    return { subject, body };
  }

  $effect(() => {
    void aiApi
      .keyStatus()
      .then((s) => (aiAvailable = s.provider === "openrouter" ? s.openrouter : s.anthropic));
  });

  function splitQuote(body: string): [string, string] {
    const idx = body.indexOf("\n\nOn ");
    if (idx >= 0 && body.slice(idx).includes(" wrote:\n")) {
      return [body.slice(0, idx), body.slice(idx)];
    }
    return [body, ""];
  }

  /** Push an instruction into the session and stream the revised draft. */
  function sendInstruction(text: string) {
    if (!draft || aiBusy) return;
    const instr = text.trim();
    if (!instr) return;

    // Respect manual edits: sync what the user currently sees back into the last
    // assistant turn, so the AI revises exactly that. For new mail the turn
    // carries the subject header too, so a hand-edited subject is fed back.
    const [current, tail] = splitQuote(draft.body);
    quotedTail = tail;
    if (current.trim()) {
      const synced = isReply
        ? current.trim()
        : `Subject: ${draft.subject}\n\n${current.trim()}`;
      for (let i = convo.length - 1; i >= 0; i--) {
        if (convo[i].role === "assistant") {
          convo[i] = { role: "assistant", content: synced };
          break;
        }
      }
    }

    convo.push({ role: "user", content: instr });
    instruction = "";
    runCompose();
  }

  function runCompose() {
    if (!draft) return;
    cancelAi?.();
    aiBusy = true;
    error = "";
    draft.body = quotedTail;
    let streamed = "";
    cancelAi = aiStream(
      "ai_compose",
      {
        turns: convo.map((tn) => ({ role: tn.role, content: tn.content })),
        replyToMessageId: draft.replyToMessageId,
      },
      {
        delta: (text) => {
          streamed += text;
          if (!draft) return;
          // Replies stream straight into the body; new mail has its subject
          // parsed off the first line so the field fills before the body does.
          let body = streamed;
          if (!isReply) {
            const parsed = parseDraft(streamed);
            body = parsed.body;
            if (subjectAuto && parsed.subject !== null) draft.subject = parsed.subject;
          }
          draft.body = quotedTail ? `${body}\n${quotedTail}` : body;
        },
        done: () => {
          const text = streamed.trim();
          if (text) {
            convo.push({ role: "assistant", content: text });
          } else {
            // Nothing usable came back — drop the user turn so the next round
            // doesn't carry a dangling instruction with no reply.
            const last = convo[convo.length - 1];
            if (last?.role === "user") convo.pop();
          }
          aiBusy = false;
          scheduleSave();
        },
        error: (_code, message) => {
          aiBusy = false;
          error = message;
          // Roll the failed instruction back into the input so it isn't lost.
          const last = convo[convo.length - 1];
          if (last?.role === "user") {
            instruction = last.content;
            convo.pop();
          }
          if (draft && quotedTail && !draft.body) draft.body = quotedTail;
        },
      },
    );
  }

  // The tone chips are just quick instructions fed into the same session; the
  // localized label doubles as the instruction and shows in the transcript.
  function adjust(kind: "shorter" | "warmer" | "formal") {
    sendInstruction(t(`ai.${kind}`));
  }

  function stopAi() {
    cancelAi?.();
    aiBusy = false;
    // Drop the unanswered instruction so the session stays consistent.
    const last = convo[convo.length - 1];
    if (last?.role === "user") convo.pop();
  }

  $effect(() => {
    void (async () => {
      try {
        const d = await api.getDraft(draftId);
        draft = d;
        showCc = d.cc.length > 0 || d.bcc.length > 0;
        attachments = await api.listDraftAttachments(draftId);
      } catch (e) {
        error = errorMessage(e);
      }
    })();
  });

  function scheduleSave() {
    if (saveTimer) clearTimeout(saveTimer);
    saveTimer = setTimeout(() => {
      if (draft) void api.updateDraft($state.snapshot(draft) as Draft);
    }, 800);
  }

  async function win() {
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    return getCurrentWindow();
  }

  async function toggleMaximize() {
    const w = await win();
    await w.toggleMaximize();
    maximized = await w.isMaximized();
  }

  async function send() {
    if (!draft || sending) return;
    sending = true;
    error = "";
    try {
      if (saveTimer) clearTimeout(saveTimer);
      await api.updateDraft($state.snapshot(draft) as Draft);
      await api.sendDraft(draft.id);
      (await win()).close();
    } catch (e) {
      error = errorMessage(e);
      sending = false;
    }
  }

  async function discard() {
    if (draft) await api.deleteDraft(draft.id).catch(() => {});
    (await win()).close();
  }

  async function close() {
    // Closing keeps the draft (it autosaves).
    if (saveTimer) clearTimeout(saveTimer);
    if (draft) await api.updateDraft($state.snapshot(draft) as Draft).catch(() => {});
    (await win()).close();
  }

  const title = $derived(draft?.subject || t("compose.new"));
</script>

<div
  class="compose-window"
  role="region"
  ondragover={(e) => {
    e.preventDefault();
    dragActive = true;
  }}
  ondragleave={(e) => {
    // Only clear when the pointer actually leaves the window, not on child
    // boundaries (dragleave fires when moving between child elements).
    if (e.currentTarget === e.target) dragActive = false;
  }}
  ondrop={onDrop}
>
  <header class="titlebar" data-tauri-drag-region>
    <span class="title" data-tauri-drag-region>{title}</span>
    <div class="controls">
      <button class="ctl" onclick={async () => (await win()).minimize()} aria-label={t("a11y.minimize")}>
        <svg width="10" height="10" viewBox="0 0 10 10"><line x1="0" y1="5" x2="10" y2="5" stroke="currentColor" stroke-width="1" /></svg>
      </button>
      <button class="ctl" onclick={toggleMaximize} aria-label={t("a11y.maximize")}>
        {#if maximized}
          <svg width="10" height="10" viewBox="0 0 10 10"><rect x="0.5" y="2.5" width="7" height="7" fill="none" stroke="currentColor" /><path d="M2.5 2.5V0.5H9.5V7.5H7.5" fill="none" stroke="currentColor" /></svg>
        {:else}
          <svg width="10" height="10" viewBox="0 0 10 10"><rect x="0.5" y="0.5" width="9" height="9" fill="none" stroke="currentColor" /></svg>
        {/if}
      </button>
      <button class="ctl ctl-close" onclick={close} aria-label={t("a11y.close")}>
        <svg width="10" height="10" viewBox="0 0 10 10"><path d="M0 0L10 10M10 0L0 10" stroke="currentColor" stroke-width="1" /></svg>
      </button>
    </div>
  </header>

  {#if draft}
    <div class="fields">
      <label class="field">
        <span class="microlabel">{t("compose.to")}</span>
        <AddressInput bind:value={draft.to} onchange={scheduleSave} />
        {#if !showCc}
          <button class="cc-toggle microlabel" onclick={() => (showCc = true)}>
            {t("compose.cc")}
          </button>
        {/if}
      </label>
      {#if showCc}
        <label class="field">
          <span class="microlabel">{t("compose.cc")}</span>
          <AddressInput bind:value={draft.cc} onchange={scheduleSave} />
        </label>
        <label class="field">
          <span class="microlabel">{t("compose.bcc")}</span>
          <AddressInput bind:value={draft.bcc} onchange={scheduleSave} />
        </label>
      {/if}
    </div>

    {#if aiAvailable}
      <div class="ai-bar">
        {#if userTurns.length > 0}
          <!-- The running session: instructions given so far, so it's clear
               this is one ongoing conversation about the draft. -->
          <div class="ai-thread">
            {#each userTurns as turn, i (i)}
              <div class="ai-turn">{turn.content}</div>
            {/each}
          </div>
        {/if}
        {#if aiBusy}
          <!-- Progress indicator: kept OUTSIDE the scrollable thread so a tall
               instruction can't push it below the fold — it must stay visible
               the whole time the draft is streaming. -->
          <div class="ai-thinking">✦ {t("ai.thinking")}</div>
        {/if}
        <form
          class="ai-input"
          onsubmit={(e) => {
            e.preventDefault();
            sendInstruction(instruction);
          }}
        >
          <span class="spark">✦</span>
          <textarea
            class="instruction"
            use:autogrow
            bind:value={instruction}
            placeholder={hasDraft ? t("ai.refine_placeholder") : t("ai.instruction_placeholder")}
            rows="1"
            spellcheck="false"
            onkeydown={(e) => {
              if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
                e.preventDefault();
                sendInstruction(instruction);
              }
            }}
          ></textarea>
          {#if aiBusy}
            <button type="button" class="ai-chip" onclick={stopAi}>{t("ai.stop")}</button>
          {:else}
            <button type="submit" class="ai-chip solid" disabled={!instruction.trim()}>
              ✦ {hasDraft ? t("ai.refine") : t("ai.generate")}
            </button>
          {/if}
        </form>
        {#if hasDraft && !aiBusy}
          <div class="tone-chips">
            <button class="ai-chip" onclick={() => adjust("shorter")}>{t("ai.shorter")}</button>
            <button class="ai-chip" onclick={() => adjust("warmer")}>{t("ai.warmer")}</button>
            <button class="ai-chip" onclick={() => adjust("formal")}>{t("ai.formal")}</button>
          </div>
        {/if}
      </div>
    {/if}

    <div class="fields subject-row">
      <label class="field">
        <span class="microlabel">{t("compose.subject")}</span>
        <input
          bind:value={draft.subject}
          oninput={() => {
            subjectAuto = false;
            scheduleSave();
          }}
          class="subject"
        />
      </label>
    </div>

    <textarea
      bind:value={draft.body}
      oninput={scheduleSave}
      placeholder={t("compose.body_placeholder")}
      spellcheck="true"
    ></textarea>

    {#if attachments.length > 0}
      <div class="attach-row">
        {#each attachments as a (a.id)}
          <div class="chip">
            <svg class="clip" width="13" height="13" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.2"><path d="M12.5 7.5l-5 5a3 3 0 0 1-4.243-4.243l5.657-5.657a2 2 0 0 1 2.829 2.829l-5.657 5.657a1 1 0 0 1-1.415-1.415l4.95-4.95" /></svg>
            <span class="name">{a.filename}</span>
            <span class="size">{formatSize(a.size)}</span>
            <button class="remove" onclick={() => removeAttachment(a.id)} title={t("compose.attach_remove")} aria-label={t("compose.attach_remove")}>
              <svg width="11" height="11" viewBox="0 0 10 10" fill="none" stroke="currentColor" stroke-width="1.2"><path d="M1 1l8 8M9 1L1 9" /></svg>
            </button>
          </div>
        {/each}
      </div>
    {/if}

    {#if error}
      <div class="error">{error}</div>
    {/if}

    <footer class="bar">
      <button class="send" onclick={send} disabled={sending || !draft.to.trim()}>
        {sending ? t("compose.sending") : t("compose.send")}
      </button>
      <button class="attach" onclick={() => fileInput?.click()} title={t("compose.attach")} aria-label={t("compose.attach")}>
        <svg width="17" height="17" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.2"><path d="M12.5 7.5l-5 5a3 3 0 0 1-4.243-4.243l5.657-5.657a2 2 0 0 1 2.829 2.829l-5.657 5.657a1 1 0 0 1-1.415-1.415l4.95-4.95" /></svg>
      </button>
      <input bind:this={fileInput} type="file" multiple class="file-input" onchange={pickFiles} />
      <div class="grow"></div>
      <button class="discard" onclick={discard} title={t("compose.discard")}>
        <svg width="15" height="15" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.2"><path d="M3 4h10M6.5 4V2.5h3V4M4.5 4l.5 9.5h6l.5-9.5M6.7 6.5v5M9.3 6.5v5" /></svg>
      </button>
    </footer>
  {/if}

  {#if dragActive}
    <div class="drop-overlay">
      <div class="drop-hint">{t("compose.drop_hint")}</div>
    </div>
  {/if}
</div>

<style>
  .compose-window {
    height: 100%;
    display: flex;
    flex-direction: column;
    background: var(--surface);
    position: relative;
  }

  .titlebar {
    height: var(--titlebar-h);
    display: flex;
    align-items: center;
    justify-content: space-between;
    border-bottom: 1px solid var(--hairline);
    flex-shrink: 0;
    background: var(--bg);
  }
  .title {
    padding-left: 16px;
    font-weight: 700;
    font-size: 13.5px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .controls {
    display: flex;
    height: 100%;
  }
  .ctl {
    width: 46px;
    height: 100%;
    display: grid;
    place-items: center;
    color: var(--text-dim);
  }
  .ctl:hover {
    background: var(--hover);
    color: var(--text);
  }
  .ctl-close:hover {
    background: #d64545;
    color: #fff;
  }

  .fields {
    padding: 6px 20px 0;
  }
  /* Subject sits just under the AI bar; give it a little breathing room. */
  .subject-row {
    padding-top: 10px;
  }
  .field {
    display: flex;
    align-items: center;
    gap: 12px;
    border-bottom: 1px solid var(--hairline);
    padding: 9px 0;
  }
  .field .microlabel {
    width: 52px;
    flex-shrink: 0;
  }
  .field input {
    flex: 1;
    font-size: 13.5px;
    user-select: text;
  }
  .subject {
    font-weight: 600;
  }
  .cc-toggle {
    color: var(--text-faint);
  }
  .cc-toggle:hover {
    color: var(--text);
  }

  textarea {
    flex: 1;
    resize: none;
    padding: 16px 20px;
    font-size: 14px;
    line-height: 1.6;
    user-select: text;
    cursor: text;
  }

  /* AI drafting bar — violet accent reserved for AI */
  .ai-bar {
    padding: 10px 20px 0;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .ai-thread {
    display: flex;
    flex-direction: column;
    gap: 4px;
    max-height: 108px;
    overflow-y: auto;
  }
  .ai-turn {
    align-self: flex-start;
    max-width: 100%;
    padding: 4px 10px;
    border-radius: var(--radius-s);
    background: var(--accent-soft);
    color: var(--text);
    font-size: 12.5px;
    line-height: 1.4;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .ai-thinking {
    align-self: flex-start;
    color: var(--accent);
    font-size: 12.5px;
    line-height: 1.4;
    animation: pulse 1.2s ease-in-out infinite;
  }
  @keyframes pulse {
    50% {
      opacity: 0.45;
    }
  }
  .ai-input {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    border: 1px solid var(--accent-dim);
    border-radius: var(--radius-m);
    padding: 8px 12px;
  }
  .spark {
    color: var(--accent);
    padding-top: 2px;
  }
  .ai-input .instruction {
    flex: 1;
    font-size: 13px;
    line-height: 1.5;
    user-select: text;
    resize: none;
    min-height: 20px;
    max-height: 180px;
    overflow-y: auto;
    padding: 0;
    font-family: inherit;
  }
  .ai-input .ai-chip {
    align-self: center;
  }
  .ai-chip {
    padding: 5px 12px;
    border-radius: 999px;
    border: 1px solid var(--accent-dim);
    color: var(--accent);
    font-size: 12px;
    font-weight: 600;
    flex-shrink: 0;
  }
  .ai-chip:hover:not(:disabled) {
    background: var(--accent-soft);
  }
  .ai-chip.solid {
    background: var(--accent);
    color: var(--on-accent);
    border-color: var(--accent);
  }
  .ai-chip.solid:hover:not(:disabled) {
    background: var(--accent);
    opacity: 0.9;
  }
  .ai-chip:disabled {
    opacity: 0.45;
    cursor: default;
  }
  .tone-chips {
    display: flex;
    gap: 6px;
  }

  .error {
    padding: 8px 20px;
    color: var(--danger);
    font-size: 12.5px;
  }

  .bar {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 12px 20px;
    border-top: 1px solid var(--hairline);
  }
  .send {
    padding: 8px 22px;
    border-radius: var(--radius-m);
    background: var(--text);
    color: var(--bg);
    font-weight: 700;
    font-size: 13.5px;
  }
  .send:hover:not(:disabled) {
    opacity: 0.88;
  }
  .send:disabled {
    opacity: 0.45;
    cursor: default;
  }
  .grow {
    flex: 1;
  }
  .attach {
    width: 34px;
    height: 34px;
    display: grid;
    place-items: center;
    border-radius: var(--radius-s);
    color: var(--text-dim);
  }
  .attach:hover {
    background: var(--hover);
    color: var(--text);
  }
  .file-input {
    display: none;
  }
  .discard {
    width: 34px;
    height: 34px;
    display: grid;
    place-items: center;
    border-radius: var(--radius-s);
    color: var(--text-dim);
  }
  .discard:hover {
    background: var(--hover);
    color: var(--danger);
  }

  /* Staged-attachment chips, above the footer. */
  .attach-row {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    padding: 8px 20px 0;
  }
  .chip {
    display: flex;
    align-items: center;
    gap: 7px;
    padding: 5px 6px 5px 10px;
    border: 1px solid var(--hairline-strong);
    border-radius: var(--radius-s);
    font-size: 12.5px;
    max-width: 100%;
  }
  .chip .clip {
    color: var(--text-dim);
    flex-shrink: 0;
  }
  .chip .name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .chip .size {
    color: var(--text-faint);
    font-family: var(--font-mono);
    font-size: 10.5px;
    flex-shrink: 0;
  }
  .chip .remove {
    display: grid;
    place-items: center;
    width: 18px;
    height: 18px;
    border-radius: var(--radius-s);
    color: var(--text-faint);
    flex-shrink: 0;
  }
  .chip .remove:hover {
    background: var(--hover);
    color: var(--danger);
  }

  /* Drop target feedback while dragging files over the window. */
  .drop-overlay {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    background: color-mix(in srgb, var(--surface) 78%, transparent);
    pointer-events: none;
    z-index: 10;
  }
  .drop-hint {
    padding: 14px 26px;
    border: 1.5px dashed var(--text-faint);
    border-radius: var(--radius-m);
    color: var(--text);
    font-size: 13.5px;
    font-weight: 600;
    background: var(--surface);
  }
</style>
