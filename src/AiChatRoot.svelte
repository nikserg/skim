<script lang="ts">
  // Root for chat windows. The window is a view, not an owner: the main window
  // keeps the conversation and the request in flight, streams its events here,
  // and takes questions back. That is what lets the chat pop out mid-answer and
  // slide back inline — closing this window returns it, nothing is interrupted.
  import AiAsk from "./components/AiAsk.svelte";
  import AiChat from "./components/AiChat.svelte";
  import AiRecap from "./components/AiRecap.svelte";
  import WindowControls from "./components/WindowControls.svelte";
  import { applyChatEvent, type ChatEvent, type ChatSession } from "./lib/ai-chat";
  import { api, type Citation } from "./lib/api";
  import { setLocale, t } from "./lib/i18n/index.svelte";
  import { ai } from "./lib/stores/ai.svelte";
  import { ui } from "./lib/stores/ui.svelte";

  let { sessionId }: { sessionId: number } = $props();
  let session = $state<ChatSession | null>(null);

  $effect(() => {
    void (async () => {
      try {
        const settings = await api.getSettings();
        if (settings.locale) await setLocale(settings.locale as never);
        // Match the stored theme (the main window owns migration write-back).
        ui.hydrate(settings.theme);
      } catch {
        // best effort
      }
      // Only for the "the model is loading" hint, which is provider-specific.
      void ai.refresh();

      const { emit, listen } = await import("@tauri-apps/api/event");
      await listen<{ id: number; sync?: ChatSession; ev?: ChatEvent }>(
        "ai-chat:to-window",
        (e) => {
          if (e.payload.id !== sessionId) return;
          if (e.payload.sync) {
            session = e.payload.sync;
          } else if (e.payload.ev && session) {
            applyChatEvent(session, e.payload.ev);
          }
        },
      );
      // Listeners first, then ask for the state — nothing can slip through.
      await emit("ai-chat:from-window", { id: sessionId, kind: "ready" });
    })();
  });

  async function toMain(kind: "ready" | "ask", question?: string) {
    const { emit } = await import("@tauri-apps/api/event");
    await emit("ai-chat:from-window", { id: sessionId, kind, question });
  }

  /** Put the chat back inline. Closing is the whole gesture: the backend tells
   *  the main window this window is gone, and the chat reappears there. */
  async function returnInline() {
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    await getCurrentWindow().close();
  }

  // The mail UI lives in the main window, so a citation opens the email there.
  function openCitation(citation: Citation) {
    void api
      .openThreadInMain(citation.folderId, citation.threadId, citation.messageId)
      .catch(() => {});
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.key !== "Escape") return;
    e.preventDefault();
    void returnInline();
  }
</script>

<svelte:window onkeydown={onKeydown} />

<div class="window">
  <WindowControls title={session?.title || t("ai.answer")} onclose={returnInline} />
  {#if session?.kind === "ask"}
    <AiAsk
      standalone
      {session}
      onsend={(q) => void toMain("ask", q)}
      onreturn={returnInline}
    />
  {:else if session?.kind === "chat"}
    <AiChat
      standalone
      {session}
      onsend={(q) => void toMain("ask", q)}
      oncitation={openCitation}
      onreturn={returnInline}
    />
  {:else if session?.kind === "recap"}
    <AiRecap
      standalone
      {session}
      onsend={(q) => void toMain("ask", q)}
      oncitation={openCitation}
      onreturn={returnInline}
    />
  {/if}
</div>

<style>
  .window {
    height: 100vh;
    display: flex;
    flex-direction: column;
    background: var(--bg);
    color: var(--text);
  }
</style>
