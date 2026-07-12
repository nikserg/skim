<script lang="ts">
  import Titlebar from "./components/Titlebar.svelte";
  import Sidebar from "./components/Sidebar.svelte";
  import MessageList from "./components/MessageList.svelte";
  import ReadingPane from "./components/ReadingPane.svelte";
  import CommandPalette from "./components/CommandPalette.svelte";
  import Onboarding from "./components/onboarding/Onboarding.svelte";
  import { api } from "./lib/api";
  import { setLocale } from "./lib/i18n/index.svelte";
  import { ai } from "./lib/stores/ai.svelte";
  import { mail } from "./lib/stores/mail.svelte";
  import { palette } from "./lib/stores/palette.svelte";
  import { ui } from "./lib/stores/ui.svelte";
  import type { Account, Theme } from "./lib/types";

  let ready = $state(false);

  $effect(() => {
    void (async () => {
      const inTauri = "__TAURI_INTERNALS__" in window;
      if (inTauri) {
        try {
          const settings = await api.getSettings();
          if (settings.locale) await setLocale(settings.locale as never);
          if (settings.theme) ui.setTheme(settings.theme as Theme);
        } catch {
          // settings are best-effort at boot
        }
        await mail.boot();
        void ai.refresh();
        void import("./lib/notifications").then((m) => m.initNotifications());
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
    const index = threads.findIndex((t) => t.id === mail.selectedThreadId);
    const next = index === -1 ? 0 : Math.max(0, Math.min(threads.length - 1, index + delta));
    mail.selectedThreadId = threads[next].id;
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
      case "unread":
        mail.patchThreadRow(thread.id, { isRead: false });
        void api.markRead(ids, false);
        break;
    }
  }

  async function replyToSelected() {
    const thread = mail.selectedThread;
    if (!thread) return;
    const detail = await api.getThread(thread.id);
    const latest = detail.messages[detail.messages.length - 1];
    const draft = await api.getReplyTemplate(latest.id, "reply");
    await api.openComposeWindow(draft.id);
  }

  async function composeNew() {
    const draft = await api.createDraft();
    await api.openComposeWindow(draft.id);
  }

  function onKeydown(e: KeyboardEvent) {
    if (!mail.account) return;

    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "k") {
      e.preventDefault();
      palette.toggle();
      return;
    }
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "n") {
      e.preventDefault();
      void composeNew();
      return;
    }
    if (palette.open || isTyping() || e.ctrlKey || e.metaKey || e.altKey) return;

    switch (e.key) {
      case "/":
        e.preventDefault();
        palette.show();
        break;
      case "j":
      case "ArrowDown":
        if (e.key === "j") moveSelection(1);
        break;
      case "k":
        moveSelection(-1);
        break;
      case "e":
        void actOnSelected("archive");
        break;
      case "#":
      case "Delete":
        void actOnSelected("delete");
        break;
      case "s":
        void actOnSelected("star");
        break;
      case "u":
        void actOnSelected("unread");
        break;
      case "r":
        void replyToSelected();
        break;
      case "Escape":
        mail.selectedThreadId = null;
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
        <ReadingPane />
      </main>
      <CommandPalette />
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
