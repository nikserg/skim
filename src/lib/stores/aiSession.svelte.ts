// Live AI chats, owned by the main window.
//
// Sessions outlive the surfaces that show them: closing the dock, switching
// emails, or popping the chat into a window of its own all leave the
// conversation — and any request in flight — right here. A chat window is only
// a view: it replays the events this store pushes and sends questions back, so
// closing it returns the chat inline mid-answer without dropping a token.
import { aiApi, aiErrorText, aiStream, api, type Citation } from "../api";
import {
  applyChatEvent,
  newSession,
  type ChatEvent,
  type ChatSession,
} from "../ai-chat";
import { t } from "../i18n/index.svelte";
import { mail } from "./mail.svelte";
import { palette } from "./palette.svelte";
import { ui } from "./ui.svelte";

/** Synthetic opening turn so the digest reads as an assistant reply the user can
 *  follow up on. Sent to the model, never rendered. Deliberately doesn't say
 *  "unread": by the time a follow-up runs the recap has marked those emails
 *  read, and the word sends the agent searching for mail it won't find. */
const RECAP_SEED = "Catch me up on my inbox.";

/** Per-email chats we keep around. A chat is a transient reading aid, not
 *  durable data, so it lives in memory only — but it must survive closing the
 *  dock and hopping between emails. The oldest idle one falls off. */
const MAX_ASK = 10;
/** Deltas arrive per token; a window doesn't need them faster than a screen
 *  refresh, and coalescing keeps the IPC quiet. */
const PUSH_MS = 50;

const state = $state({ list: [] as ChatSession[] });

let seq = 0;
/** id -> cancel the request in flight for that session. */
const streams = new Map<number, () => void>();
/** id -> deltas waiting to go to that session's window. */
const pushBuffer = new Map<number, string>();
let pushTimer: ReturnType<typeof setTimeout> | null = null;
let bridge: Promise<void> | null = null;

function find(id: number): ChatSession | undefined {
  return state.list.find((s) => s.id === id);
}

/** Put a new session in the list and hand back the *stored* one. Reads go
 *  through a proxy that tracks them; writing to the object we were handed
 *  instead would update the data and tell no one. */
function add(session: ChatSession): ChatSession {
  state.list.push(session);
  return state.list[state.list.length - 1];
}

// ---- the window bridge ---------------------------------------------------
// Only the owning (main) window runs this side. Events carry a session id, and
// the two directions have different names, so a window never hears itself.

function ensureBridge(): Promise<void> {
  if (!bridge) bridge = attachBridge();
  return bridge;
}

async function attachBridge() {
  const { listen } = await import("@tauri-apps/api/event");
  await listen<{ id: number; kind: "ready" | "ask"; question?: string }>(
    "ai-chat:from-window",
    (e) => {
      const session = find(e.payload.id);
      if (!session) return;
      if (e.payload.kind === "ready") {
        void pushSync(session);
      } else if (e.payload.kind === "ask" && e.payload.question) {
        send(session, e.payload.question);
      }
    },
  );
  // The backend reports every chat window that goes away — by the button, by
  // Esc, by Alt+F4 — so the chat comes back inline however it was closed.
  await listen<{ id: number }>("ai-chat:closed", (e) => void reattach(e.payload.id));
}

async function reattach(id: number) {
  const session = find(id);
  if (!session?.detached) return;
  session.detached = false;
  session.open = true;
  if (session.kind === "ask") {
    touch(session);
    return;
  }
  // The palette and the recap pane each show one chat at a time; the one coming
  // back wins.
  for (const other of [...state.list]) {
    if (other !== session && other.kind === session.kind && !other.detached) drop(other);
  }
  if (session.kind === "recap") {
    // The digest owns the reading pane, so it needs that pane free — and it is
    // about one folder, so bring the app back to that folder first. Awaited:
    // the panel must not open while the app still points at another folder, or
    // it would digest that one over the top of the one coming back.
    if (session.folderId !== null && mail.selectedFolderId !== session.folderId) {
      await mail.selectFolder(session.folderId).catch(() => {});
    }
    mail.selectedThreadId = null;
    ui.openRecap();
    return;
  }
  palette.show();
}

async function pushSync(session: ChatSession) {
  flushDeltas();
  const { emit } = await import("@tauri-apps/api/event");
  await emit("ai-chat:to-window", { id: session.id, sync: $state.snapshot(session) });
}

function push(session: ChatSession, ev: ChatEvent) {
  if (!session.detached) return;
  // Deltas coalesce; anything that changes the shape of the chat goes at once,
  // after the text that preceded it.
  if (ev.t === "delta") {
    pushBuffer.set(session.id, (pushBuffer.get(session.id) ?? "") + ev.text);
    if (!pushTimer) pushTimer = setTimeout(flushDeltas, PUSH_MS);
    return;
  }
  flushDeltas();
  void emitEvent(session.id, ev);
}

function flushDeltas() {
  if (pushTimer) {
    clearTimeout(pushTimer);
    pushTimer = null;
  }
  for (const [id, text] of pushBuffer) {
    if (text) void emitEvent(id, { t: "delta", text });
  }
  pushBuffer.clear();
}

async function emitEvent(id: number, ev: ChatEvent) {
  const { emit } = await import("@tauri-apps/api/event");
  await emit("ai-chat:to-window", { id, ev });
}

// ---- sessions ------------------------------------------------------------

/** Move a session to the end: the eviction victim is the oldest untouched one. */
function touch(session: ChatSession) {
  const i = state.list.indexOf(session);
  if (i >= 0) state.list.splice(i, 1);
  state.list.push(session);
  evict();
}

function evict() {
  let over = state.list.filter((s) => s.kind === "ask").length - MAX_ASK;
  for (let i = 0; i < state.list.length && over > 0; ) {
    const s = state.list[i];
    // Never drop a chat someone is watching or waiting on.
    if (s.kind === "ask" && !s.detached && s.status !== "streaming") {
      state.list.splice(i, 1);
      over--;
      continue;
    }
    i++;
  }
}

function apply(session: ChatSession, ev: ChatEvent) {
  applyChatEvent(session, ev);
  push(session, ev);
}

/** Run a question through the backend, streaming into the session — and out to
 *  its window, if it has one. */
function send(session: ChatSession, question: string) {
  const q = question.trim();
  if (!q || session.status === "streaming") return;
  streams.get(session.id)?.();
  apply(session, { t: "user", content: q });

  const history = session.turns.map((turn) => ({ role: turn.role, content: turn.content }));
  const args: Record<string, unknown> =
    session.kind === "ask"
      ? { messageId: session.messageId, turns: history }
      : {
          turns: history,
          // Citations from earlier answers, deduped by their [N] number, so the
          // agent can read those emails again and keep the numbers stable.
          priorCitations: priorCitations(session),
          contextMessageId: session.messageId,
        };

  const cancel = aiStream(session.kind === "ask" ? "ai_ask" : "ai_chat", args, {
    delta: (text) => apply(session, { t: "delta", text }),
    reasoning: () => apply(session, { t: "reasoning" }),
    toolCall: (id, kind, arg) => apply(session, { t: "toolCall", id, kind, arg }),
    toolDone: (id, count) => apply(session, { t: "toolDone", id, count }),
    done: (citations) => {
      streams.delete(session.id);
      // An answer that came back blank is a failure, not a turn — committing it
      // would leave an empty violet card sitting in the thread.
      if (!session.answer.trim()) {
        apply(session, { t: "fail", why: t("ai.no_answer") });
        return;
      }
      apply(session, { t: "answer", content: session.answer, citations });
    },
    error: (code, message) => {
      streams.delete(session.id);
      apply(session, { t: "fail", why: aiErrorText(code, message) });
    },
  });
  streams.set(session.id, cancel);
}

/** Digest a folder's unread mail. Same session shape as the chats, so the
 *  digest can be followed up on — and popped into a window — like any of them. */
function startRecap(folderId: number): ChatSession {
  for (const s of [...state.list]) if (s.kind === "recap" && !s.detached) drop(s);
  const session = add(newSession(++seq, "recap", { folderId, title: t("ai.recap_title") }));
  apply(session, { t: "start" });
  const cancel = aiStream(
    "ai_recap",
    { folderId },
    {
      progress: (current, total) => apply(session, { t: "progress", current, total }),
      delta: (text) => apply(session, { t: "delta", text }),
      reasoning: () => apply(session, { t: "reasoning" }),
      done: (citations) => {
        streams.delete(session.id);
        // No digest means nothing to seed the chat with — show why rather than
        // an empty panel.
        if (!session.answer.trim()) {
          apply(session, { t: "fail", why: t("ai.no_answer") });
          return;
        }
        apply(session, {
          t: "recap",
          content: session.answer,
          citations,
          seed: RECAP_SEED,
        });
        markRead(session, citations);
      },
      error: (code, message) => {
        streams.delete(session.id);
        apply(session, { t: "fail", why: aiErrorText(code, message) });
      },
    },
  );
  streams.set(session.id, cancel);
  return session;
}

/** The digest covered these emails, so they are no longer unread. Optimistic,
 *  like every other mutation — the op queue catches the server up. */
function markRead(session: ChatSession, cited: Citation[]) {
  const ids = cited.map((c) => c.messageId);
  if (ids.length === 0) return;
  apply(session, { t: "marked", n: ids.length });
  for (const c of cited) {
    if (c.threadId !== null) mail.patchThreadRow(c.threadId, { isRead: true });
  }
  void api.markRead(ids, true);
}

function priorCitations(session: ChatSession): Citation[] {
  const byIndex = new Map<number, Citation>();
  for (const turn of session.turns) for (const c of turn.citations) byIndex.set(c.index, c);
  return [...byIndex.values()];
}

function cancel(session: ChatSession) {
  streams.get(session.id)?.();
  streams.delete(session.id);
  // Through `apply`, not by hand: a chat that is popped out only learns about
  // state changes from the events this pushes over the bridge, so a direct
  // mutation would leave that window spinning on a request that no longer runs.
  if (session.status === "streaming") apply(session, { t: "cancelled" });
}

function drop(session: ChatSession) {
  cancel(session);
  const i = state.list.indexOf(session);
  if (i >= 0) state.list.splice(i, 1);
}

export const aiSessions = {
  /** The chat about an email, if one has been started. Read-only lookup. */
  askFor(messageId: number | null | undefined): ChatSession | undefined {
    if (messageId == null) return undefined;
    return state.list.find((s) => s.kind === "ask" && s.messageId === messageId);
  },

  /** Show the chat about an email, starting it if this is the first time. */
  openAsk(messageId: number, title: string, flagged: boolean): ChatSession {
    const found = this.askFor(messageId);
    const session =
      found ?? add(newSession(++seq, "ask", { messageId, title, flagged, open: false }));
    session.open = true;
    touch(session);
    return session;
  },

  /** Swap the inline chat between the dock and the whole window. Same session
   *  either way — only where it is drawn changes. */
  toggleExpand(session: ChatSession) {
    session.expanded = !session.expanded;
  },

  /** The mailbox-wide chat the palette is showing, if any. */
  get paletteChat(): ChatSession | undefined {
    return state.list.find((s) => s.kind === "chat" && !s.detached);
  },

  /** The unread digest the reading pane is showing, if any. */
  get recap(): ChatSession | undefined {
    return state.list.find((s) => s.kind === "recap" && !s.detached);
  },

  /** The digest of a folder wherever it is — including in a window of its own,
   *  which is what stops a second scan being started behind it. */
  recapFor(folderId: number | null): ChatSession | undefined {
    if (folderId === null) return undefined;
    return state.list.find((s) => s.kind === "recap" && s.folderId === folderId);
  },

  startRecap,

  /** Start a mailbox-wide chat from the palette. */
  startPaletteChat(question: string, contextMessageId: number | null): ChatSession {
    for (const s of [...state.list]) if (s.kind === "chat" && !s.detached) drop(s);
    const session = add(newSession(++seq, "chat", { messageId: contextMessageId }));
    send(session, question);
    return session;
  },

  send,
  drop,

  /** Give a chat its own window. The request keeps streaming here; the window
   *  only watches, so this is safe mid-answer. Already in a window? Then this
   *  is a request to go look at it. */
  async detach(session: ChatSession) {
    const wasDetached = session.detached;
    const wasExpanded = session.expanded;
    // Marked before the first await: the surface handing the chat over stands
    // down in the same tick, so nothing tears the session down behind us.
    session.detached = true;
    session.open = false;
    // The window is the roomier view; coming back should land in the dock.
    session.expanded = false;
    await ensureBridge();
    try {
      await aiApi.openWindow(session.id, session.title);
    } catch {
      session.detached = wasDetached;
      session.open = !wasDetached;
      session.expanded = wasExpanded;
    }
  },

  /** Close the surface showing a chat. The conversation stays — an answer
   *  nobody is waiting for any more does not. */
  close(session: ChatSession) {
    cancel(session);
    session.open = false;
    session.expanded = false;
  },
};

export type { ChatSession };
