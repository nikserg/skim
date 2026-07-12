<script lang="ts">
  // The compose window's root component (mounted for #/compose/{id}).
  import { aiApi, aiStream, api, errorMessage } from "../lib/api";
  import { t } from "../lib/i18n/index.svelte";
  import type { Draft } from "../lib/types";

  let { draftId }: { draftId: number } = $props();

  let draft = $state<Draft | null>(null);
  let showCc = $state(false);
  let sending = $state(false);
  let error = $state("");
  let saveTimer: ReturnType<typeof setTimeout> | null = null;

  // ---- AI drafting ----
  let aiAvailable = $state(false);
  let instruction = $state("");
  let aiBusy = $state(false);
  let generatedOnce = $state(false);
  let cancelAi: (() => void) | null = null;
  /** Quoted original (reply/forward), preserved below AI-generated text. */
  let quotedTail = "";

  $effect(() => {
    void aiApi.keyStatus().then((present) => (aiAvailable = present));
  });

  function splitQuote(body: string): [string, string] {
    const idx = body.indexOf("\n\nOn ");
    if (idx >= 0 && body.slice(idx).includes(" wrote:\n")) {
      return [body.slice(0, idx), body.slice(idx)];
    }
    return [body, ""];
  }

  function runAi(command: "ai_draft" | "ai_adjust_draft", args: Record<string, unknown>) {
    if (!draft || aiBusy) return;
    cancelAi?.();
    aiBusy = true;
    error = "";
    const [, tail] = splitQuote(draft.body);
    quotedTail = tail;
    draft.body = "";
    cancelAi = aiStream(command, args, {
      delta: (text) => {
        if (draft) draft.body += text;
      },
      done: () => {
        if (draft && quotedTail) draft.body = `${draft.body}\n${quotedTail}`;
        aiBusy = false;
        generatedOnce = true;
        scheduleSave();
      },
      error: (_code, message) => {
        aiBusy = false;
        error = message;
        if (draft && quotedTail && !draft.body) draft.body = quotedTail;
      },
    });
  }

  function generate(tone?: string) {
    if (!draft || !instruction.trim()) return;
    runAi("ai_draft", {
      instruction: instruction.trim(),
      replyToMessageId: draft.replyToMessageId,
      tone: tone ?? null,
    });
  }

  function adjust(adjustment: "shorter" | "warmer" | "formal") {
    if (!draft) return;
    const [current] = splitQuote(draft.body);
    if (!current.trim()) return;
    runAi("ai_adjust_draft", { currentText: current, adjustment });
  }

  function stopAi() {
    cancelAi?.();
    aiBusy = false;
  }

  $effect(() => {
    void (async () => {
      try {
        const d = await api.getDraft(draftId);
        draft = d;
        showCc = d.cc.length > 0 || d.bcc.length > 0;
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

<div class="compose-window">
  <header class="titlebar" data-tauri-drag-region>
    <span class="title" data-tauri-drag-region>{title}</span>
    <div class="controls">
      <button class="ctl" onclick={async () => (await win()).minimize()} aria-label="Minimize">
        <svg width="10" height="10" viewBox="0 0 10 10"><line x1="0" y1="5" x2="10" y2="5" stroke="currentColor" stroke-width="1" /></svg>
      </button>
      <button class="ctl ctl-close" onclick={close} aria-label="Close">
        <svg width="10" height="10" viewBox="0 0 10 10"><path d="M0 0L10 10M10 0L0 10" stroke="currentColor" stroke-width="1" /></svg>
      </button>
    </div>
  </header>

  {#if draft}
    <div class="fields">
      <label class="field">
        <span class="microlabel">{t("compose.to")}</span>
        <input bind:value={draft.to} oninput={scheduleSave} spellcheck="false" />
        {#if !showCc}
          <button class="cc-toggle microlabel" onclick={() => (showCc = true)}>
            {t("compose.cc")}
          </button>
        {/if}
      </label>
      {#if showCc}
        <label class="field">
          <span class="microlabel">{t("compose.cc")}</span>
          <input bind:value={draft.cc} oninput={scheduleSave} spellcheck="false" />
        </label>
        <label class="field">
          <span class="microlabel">{t("compose.bcc")}</span>
          <input bind:value={draft.bcc} oninput={scheduleSave} spellcheck="false" />
        </label>
      {/if}
      <label class="field">
        <span class="microlabel">{t("compose.subject")}</span>
        <input bind:value={draft.subject} oninput={scheduleSave} class="subject" />
      </label>
    </div>

    {#if aiAvailable}
      <div class="ai-bar">
        <form
          class="ai-input"
          onsubmit={(e) => {
            e.preventDefault();
            generate();
          }}
        >
          <span class="spark">✦</span>
          <input
            bind:value={instruction}
            placeholder={t("ai.instruction_placeholder")}
            spellcheck="false"
          />
          {#if aiBusy}
            <button type="button" class="ai-chip" onclick={stopAi}>{t("ai.stop")}</button>
          {:else}
            <button type="submit" class="ai-chip solid" disabled={!instruction.trim()}>
              ✦ {generatedOnce ? t("ai.regenerate") : t("ai.generate")}
            </button>
          {/if}
        </form>
        {#if generatedOnce && !aiBusy}
          <div class="tone-chips">
            <button class="ai-chip" onclick={() => adjust("shorter")}>{t("ai.shorter")}</button>
            <button class="ai-chip" onclick={() => adjust("warmer")}>{t("ai.warmer")}</button>
            <button class="ai-chip" onclick={() => adjust("formal")}>{t("ai.formal")}</button>
          </div>
        {/if}
      </div>
    {/if}

    <textarea
      bind:value={draft.body}
      oninput={scheduleSave}
      placeholder={t("compose.body_placeholder")}
      spellcheck="true"
    ></textarea>

    {#if error}
      <div class="error">{error}</div>
    {/if}

    <footer class="bar">
      <button class="send" onclick={send} disabled={sending || !draft.to.trim()}>
        {sending ? t("compose.sending") : t("compose.send")}
      </button>
      <div class="grow"></div>
      <button class="discard" onclick={discard} title={t("compose.discard")}>
        <svg width="15" height="15" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.2"><path d="M3 4h10M6.5 4V2.5h3V4M4.5 4l.5 9.5h6l.5-9.5M6.7 6.5v5M9.3 6.5v5" /></svg>
      </button>
    </footer>
  {/if}
</div>

<style>
  .compose-window {
    height: 100%;
    display: flex;
    flex-direction: column;
    background: var(--surface);
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
  .ai-input {
    display: flex;
    align-items: center;
    gap: 10px;
    border: 1px solid var(--accent-dim);
    border-radius: var(--radius-m);
    padding: 8px 12px;
  }
  .spark {
    color: var(--accent);
  }
  .ai-input input {
    flex: 1;
    font-size: 13px;
    user-select: text;
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
</style>
