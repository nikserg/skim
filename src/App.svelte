<script lang="ts">
  import Titlebar from "./components/Titlebar.svelte";
  import Sidebar from "./components/Sidebar.svelte";
  import MessageList from "./components/MessageList.svelte";
  import ReadingPane from "./components/ReadingPane.svelte";
  import AiRecap from "./components/AiRecap.svelte";
  import CommandPalette from "./components/CommandPalette.svelte";
  import ShortcutsOverlay from "./components/ShortcutsOverlay.svelte";
  import Onboarding from "./components/onboarding/Onboarding.svelte";
  import { api } from "./lib/api";
  import { setLocale } from "./lib/i18n/index.svelte";
  import { ai } from "./lib/stores/ai.svelte";
  import { mail } from "./lib/stores/mail.svelte";
  import { palette } from "./lib/stores/palette.svelte";
  import { ui } from "./lib/stores/ui.svelte";
  import type { Account } from "./lib/types";

  let ready = $state(false);

  // Opening a message dismisses the recap panel.
  $effect(() => {
    if (mail.selectedThreadId !== null && ui.recapOpen) ui.closeRecap();
  });

  $effect(() => {
    void (async () => {
      const inTauri = "__TAURI_INTERNALS__" in window;
      if (inTauri) {
        try {
          const settings = await api.getSettings();
          if (settings.locale) await setLocale(settings.locale as never);
          // Apply the stored theme (migrating legacy values); persist the
          // normalized string back once if migration changed it.
          const normalized = ui.hydrate(settings.theme);
          if (settings.theme !== normalized) void api.setSetting("theme", normalized).catch(() => {});
          if (settings.sidebar_collapsed) ui.setSidebarCollapsed(settings.sidebar_collapsed === "on");
        } catch {
          // settings are best-effort at boot
        }
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

  async function actOnSelected(action: "archive" | "delete" | "star" | "unread") {
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
    const draft = await api.createDraft();
    await api.openComposeWindow(draft.id);
  }

  function onKeydown(e: KeyboardEvent) {
    if (!mail.account) return;

    // Letter shortcuts match the physical key (e.code), not the produced
    // character (e.key): in a Cyrillic (or any non-Latin) layout the K key
    // emits "л", not "k", so an e.key check would only work in a US layout.
    if ((e.ctrlKey || e.metaKey) && e.code === "KeyK") {
      e.preventDefault();
      palette.toggle();
      return;
    }
    if ((e.ctrlKey || e.metaKey) && e.code === "KeyN") {
      e.preventDefault();
      void composeNew();
      return;
    }
    if (palette.open || ui.shortcutsOpen || isTyping() || e.ctrlKey || e.metaKey || e.altKey)
      return;

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
      case "KeyD":
        if (ai.keyPresent) ui.readingAi?.draftReply();
        return;
      case "KeyM":
        if (ai.keyPresent) ui.readingAi?.summarize();
        return;
      case "KeyQ":
        if (ai.keyPresent) ui.readingAi?.ask();
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
    {#if mail.account}
      <main class="panes">
        <Sidebar />
        <MessageList />
        {#if ui.recapOpen && mail.selectedThreadId === null}
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
