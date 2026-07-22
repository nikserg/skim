<script lang="ts">
  import Titlebar from "./components/Titlebar.svelte";
  import Sidebar from "./components/Sidebar.svelte";
  import MessageList from "./components/MessageList.svelte";
  import ReadingPane from "./components/ReadingPane.svelte";
  import ComposeForm from "./components/ComposeForm.svelte";
  import AiRecap from "./components/AiRecap.svelte";
  import CommandPalette from "./components/CommandPalette.svelte";
  import ShortcutsOverlay from "./components/ShortcutsOverlay.svelte";
  import Onboarding from "./components/onboarding/Onboarding.svelte";
  import { api } from "./lib/api";
  import { setLocale } from "./lib/i18n/index.svelte";
  import { ai } from "./lib/stores/ai.svelte";
  import { mail, UNIFIED } from "./lib/stores/mail.svelte";
  import { palette } from "./lib/stores/palette.svelte";
  import { ui } from "./lib/stores/ui.svelte";
  import { updater } from "./lib/stores/update.svelte";
  import type { Account, Draft } from "./lib/types";

  let ready = $state(false);

  // Opening a message dismisses the recap panel.
  $effect(() => {
    if (mail.selectedThreadId !== null && ui.recapOpen) ui.closeRecap();
  });

  // ---- In-pane draft editor (Drafts folder) ----
  // Selecting a draft opens the compose surface in the reading pane instead of
  // the read-only view. `draftEditorId` is the local draft to edit; `draftOrigin`
  // the Drafts-folder message it mirrors (needed to remove it on discard).
  let draftEditorId = $state<number | null>(null);
  let draftOrigin = $state<number | null>(null);
  let draftResolveToken = 0;

  $effect(() => {
    const folder = mail.selectedFolder;
    const threadId = mail.selectedThreadId;
    const messageId = mail.selectedMessageId;
    if (folder?.role !== "drafts" || threadId === null) {
      draftEditorId = null;
      draftOrigin = null;
      return;
    }
    const token = ++draftResolveToken;
    void (async () => {
      try {
        // Grouped rows carry no messageId — resolve the draft message from the
        // thread (its latest message).
        let msgId = messageId;
        if (msgId == null) {
          const detail = await api.getThread(threadId);
          msgId = detail.messages[detail.messages.length - 1]?.id ?? null;
        }
        if (msgId == null) return;
        const draft = await api.editDraft(msgId);
        if (token !== draftResolveToken) return; // selection moved on
        draftOrigin = msgId;
        draftEditorId = draft.id;
      } catch {
        if (token === draftResolveToken) {
          draftEditorId = null;
          draftOrigin = null;
        }
      }
    })();
  });

  function draftPreview(body: string): string {
    return (body.split(/\r?\n/).find((l) => l.trim()) ?? "").slice(0, 200);
  }

  /** Reflect a live autosave in the Drafts list row. */
  function onDraftLocalSave(d: Draft) {
    if (mail.selectedThreadId !== null) {
      mail.patchThreadRow(mail.selectedThreadId, {
        subject: d.subject,
        snippet: draftPreview(d.body),
      });
    }
  }

  function onDraftSent() {
    // The backend removes the server copy from Drafts; drop the row now.
    if (mail.selectedThreadId !== null) mail.removeThreadFromList(mail.selectedThreadId);
    draftEditorId = null;
    draftOrigin = null;
  }

  function onDraftDiscarded() {
    // The form deleted the local draft; remove the server copy too.
    if (draftOrigin !== null) void api.deleteMessages([draftOrigin]);
    if (mail.selectedThreadId !== null) mail.removeThreadFromList(mail.selectedThreadId);
    draftEditorId = null;
    draftOrigin = null;
  }

  $effect(() => {
    void (async () => {
      const inTauri = "__TAURI_INTERNALS__" in window;
      if (inTauri) {
        let settings: Record<string, string> = {};
        try {
          settings = await api.getSettings();
          if (settings.locale) await setLocale(settings.locale as never);
          // Apply the stored theme (migrating legacy values); persist the
          // normalized string back once if migration changed it.
          const normalized = ui.hydrate(settings.theme);
          if (settings.theme !== normalized) void api.setSetting("theme", normalized).catch(() => {});
          if (settings.sidebar_collapsed) ui.setSidebarCollapsed(settings.sidebar_collapsed === "on");
          if (settings.palette_expanded) ui.setPaletteExpanded(settings.palette_expanded === "on");
        } catch {
          // settings are best-effort at boot
        }
        updater.init(settings);
        await mail.boot();
        void ai.refresh();
      }
      ready = true;
    })();
  });

  function onboarded(account: Account) {
    void mail.accountAdded(account);
    void ai.refresh();
  }

  function isTyping(): boolean {
    const el = document.activeElement;
    return (
      el instanceof HTMLInputElement ||
      el instanceof HTMLTextAreaElement ||
      (el instanceof HTMLElement && el.isContentEditable)
    );
  }

  function moveSelection(delta: number) {
    const threads = mail.threads;
    if (threads.length === 0) return;
    // Rows are keyed by thread in grouped mode, by message in flat mode (where
    // several rows can share a thread id).
    const index = mail.groupThreads
      ? threads.findIndex((t) => t.id === mail.selectedThreadId)
      : threads.findIndex((t) => t.messageId === mail.selectedMessageId);
    const next = index === -1 ? 0 : Math.max(0, Math.min(threads.length - 1, index + delta));
    const row = threads[next];
    mail.selectedThreadId = row.id;
    mail.selectedMessageId = row.messageId ?? null;
  }

  async function actOnSelected(action: "archive" | "delete" | "spam" | "star" | "unread") {
    const thread = mail.selectedThread;
    if (!thread) return;
    const ids = await api.threadMessageIds(thread.id);
    if (ids.length === 0) return;
    switch (action) {
      case "archive":
        mail.removeThreadFromList(thread.id);
        void api.archiveMessages(ids);
        break;
      case "delete":
        mail.removeThreadFromList(thread.id);
        void api.deleteMessages(ids);
        break;
      case "spam":
        mail.removeThreadFromList(thread.id);
        void api.reportSpam(ids);
        break;
      case "star":
        mail.patchThreadRow(thread.id, { isStarred: !thread.isStarred });
        void api.setStarred(ids, !thread.isStarred);
        break;
      case "unread": {
        const next = !thread.isRead;
        mail.patchThreadRow(thread.id, { isRead: next });
        void api.markRead(ids, next);
        break;
      }
    }
  }

  async function replyToSelected(mode: "reply" | "reply_all" | "forward" = "reply") {
    const thread = mail.selectedThread;
    if (!thread) return;
    const detail = await api.getThread(thread.id);
    const latest = detail.messages[detail.messages.length - 1];
    const draft = await api.getReplyTemplate(latest.id, mode);
    await api.openComposeWindow(draft.id);
  }

  async function composeNew() {
    const draft = await api.createDraft(await mail.composeAccountId());
    await api.openComposeWindow(draft.id);
  }

  function onKeydown(e: KeyboardEvent) {
    if (mail.accounts.length === 0) return;

    // Letter shortcuts match the physical key (e.code), not the produced
    // character (e.key): in a Cyrillic (or any non-Latin) layout the K key
    // emits "л", not "k", so an e.key check would only work in a US layout.
    // Ctrl/Cmd+K opens the palette (idempotent — no-op when already open). When
    // it's open and a chat is active, the palette's own window handler repurposes
    // this chord to toggle the expanded view instead of closing.
    if ((e.ctrlKey || e.metaKey) && e.code === "KeyK") {
      e.preventDefault();
      palette.show();
      return;
    }
    if ((e.ctrlKey || e.metaKey) && e.code === "KeyN") {
      e.preventDefault();
      void composeNew();
      return;
    }
    // Ctrl+1 is "All inboxes", Ctrl+2..9 the Nth mailbox — matching the
    // switcher's order (only when several accounts exist).
    if ((e.ctrlKey || e.metaKey) && mail.accounts.length > 1 && /^Digit[1-9]$/.test(e.code)) {
      const n = Number(e.code.slice(5));
      const target = n === 1 ? UNIFIED : mail.accounts[n - 2]?.id;
      if (target) {
        e.preventDefault();
        void mail.switchAccount(target);
      }
      return;
    }
    // Escape leaves the in-pane draft editor (its teardown writes edits back),
    // even while a field is focused — handle it before the typing guard.
    if (e.key === "Escape" && draftEditorId !== null && mail.selectedFolder?.role === "drafts") {
      e.preventDefault();
      mail.selectedThreadId = null;
      return;
    }
    if (palette.open || ui.shortcutsOpen || isTyping() || e.ctrlKey || e.metaKey || e.altKey)
      return;
    // The in-pane draft editor owns the keyboard — don't let list shortcuts
    // (archive/reply/star…) act on the draft being edited.
    if (mail.selectedFolder?.role === "drafts" && draftEditorId !== null) return;

    switch (e.code) {
      case "KeyJ":
        moveSelection(1);
        return;
      case "KeyK":
        moveSelection(-1);
        return;
      case "KeyE":
        void actOnSelected("archive");
        return;
      case "KeyS":
        void actOnSelected("star");
        return;
      case "KeyU":
        void actOnSelected("unread");
        return;
      case "KeyR":
        void replyToSelected("reply");
        return;
      case "KeyA":
        void replyToSelected("reply_all");
        return;
      case "KeyF":
        void replyToSelected("forward");
        return;
      case "KeyQ":
        // preventDefault so the "q" that opened the dock isn't also typed into
        // the Ask input we're about to focus.
        if (ai.keyPresent) {
          e.preventDefault();
          ui.readingAi?.ask();
        }
        return;
    }

    // Symbol / navigation keys are layout-independent enough to match on e.key.
    switch (e.key) {
      case "/":
        e.preventDefault();
        palette.show();
        break;
      case "?":
        e.preventDefault();
        ui.openShortcuts();
        break;
      case ".":
        e.preventDefault();
        ui.toggleSidebar();
        break;
      case "#":
      case "Delete":
        void actOnSelected("delete");
        break;
      case "!":
        void actOnSelected("spam");
        break;
      case "Escape":
        if (ui.recapOpen) ui.closeRecap();
        else mail.selectedThreadId = null;
        break;
    }
  }
</script>

<svelte:window onkeydown={onKeydown} />

<div class="app">
  <Titlebar />
  {#if ready}
    {#if mail.accounts.length > 0}
      <main class="panes">
        <Sidebar />
        <MessageList />
        {#if mail.selectedFolder?.role === "drafts" && draftEditorId !== null}
          {#key draftEditorId}
            <ComposeForm
              draftId={draftEditorId}
              onSent={onDraftSent}
              onDiscarded={onDraftDiscarded}
              onLocalSave={onDraftLocalSave}
            />
          {/key}
        {:else if ui.recapOpen && mail.selectedThreadId === null}
          <AiRecap />
        {:else}
          <ReadingPane />
        {/if}
      </main>
      <CommandPalette />
      {#if ui.shortcutsOpen}
        <ShortcutsOverlay />
      {/if}
    {:else}
      <Onboarding oncomplete={onboarded} />
    {/if}
  {/if}
</div>

<style>
  .app {
    height: 100%;
    display: flex;
    flex-direction: column;
  }
  .panes {
    flex: 1;
    display: flex;
    min-height: 0;
  }
</style>
