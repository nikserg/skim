// Mock of `@tauri-apps/api/core` for the product demo.
//
// The whole app talks to its Rust backend through `invoke()` and streams AI
// output over a `Channel`. Alias this module in place of the real one and the
// UI runs unchanged in a plain browser, served entirely fake data.

import * as db from "./data";

// The app checks `"__TAURI_INTERNALS__" in window` to decide whether to boot
// (vs. show onboarding). Presence is enough — our aliased invoke does the work.
(globalThis as any).window && ((window as any).__TAURI_INTERNALS__ ||= { demo: true });

// Tunables the recorder can override via localStorage before the app boots.
function num(key: string, fallback: number): number {
  const v = Number((globalThis as any).localStorage?.getItem(key));
  return Number.isFinite(v) && v > 0 ? v : fallback;
}
const TYPING_MS = () => num("skimdemo.typingMs", 24); // per token
const THINK_MS = () => num("skimdemo.thinkMs", 420); // pause before first token
const STEP_MS = () => num("skimdemo.stepMs", 650); // tool-step dwell

// ---- AI streaming --------------------------------------------------------
export class Channel<T = unknown> {
  onmessage: (msg: T) => void = () => {};
}

const cancelled = new Set<string>();

function tokens(text: string): string[] {
  // Keep whitespace attached so the streamed text reflows naturally.
  return text.match(/\S+\s*/g) ?? [text];
}

function streamText(
  channel: Channel<any>,
  text: string,
  requestId: string,
  citations: any[] = [],
  startDelay = THINK_MS(),
): void {
  const parts = tokens(text);
  let i = 0;
  const tick = () => {
    if (cancelled.has(requestId)) return;
    if (i >= parts.length) {
      channel.onmessage({ type: "done", citations });
      return;
    }
    channel.onmessage({ type: "delta", text: parts[i++] });
    setTimeout(tick, TYPING_MS());
  };
  setTimeout(tick, startDelay);
}

function runChat(channel: Channel<any>, requestId: string): void {
  const { steps, answer, citations } = db.AI_CHAT;
  let delay = 300;
  steps.forEach((s, idx) => {
    const id = `step-${idx}`;
    setTimeout(() => {
      if (cancelled.has(requestId)) return;
      channel.onmessage({ type: "toolCall", id, kind: s.kind, arg: s.arg });
    }, delay);
    delay += STEP_MS();
    setTimeout(() => {
      if (cancelled.has(requestId)) return;
      channel.onmessage({ type: "toolDone", id, count: s.count });
    }, delay);
    delay += 120;
  });
  streamText(channel, answer, requestId, citations, delay + 200);
}

const AI_COMMANDS = new Set([
  "ai_summarize",
  "ai_ask",
  "ai_compose",
  "ai_draft",
  "ai_adjust_draft",
  "ai_chat",
  "ai_recap",
  "ai_analyze_style",
]);

function handleAi(cmd: string, args: any): void {
  const channel: Channel<any> | undefined = args?.channel;
  const requestId: string = args?.requestId ?? "";
  cancelled.delete(requestId);
  if (!channel) return;

  switch (cmd) {
    case "ai_summarize":
      streamText(channel, db.AI_SUMMARY, requestId);
      return;
    case "ai_ask":
      streamText(channel, db.AI_ASK, requestId);
      return;
    case "ai_recap":
      streamText(channel, db.AI_SUMMARY, requestId);
      return;
    case "ai_chat":
      runChat(channel, requestId);
      return;
    case "ai_compose":
    case "ai_draft":
    case "ai_adjust_draft": {
      const isReply = args?.replyToMessageId != null;
      streamText(channel, isReply ? db.AI_REPLY : db.AI_COMPOSE_NEW, requestId);
      return;
    }
    default:
      streamText(channel, db.AI_SUMMARY, requestId);
  }
}

// ---- Plain command surface ----------------------------------------------
export function invoke<T = any>(cmd: string, args: any = {}): Promise<T> {
  if (AI_COMMANDS.has(cmd)) {
    handleAi(cmd, args);
    return Promise.resolve(undefined as T);
  }

  const ok = <R>(v: R) => Promise.resolve(v as unknown as T);

  switch (cmd) {
    // accounts
    case "list_accounts":
      return ok([db.ACCOUNT]);
    case "google_oauth_available":
      return ok(false);
    case "autoconfig_lookup":
      return ok(null);

    // settings — the recorder/screenshotter can force a theme via localStorage.
    // Theme is two-axis ("<cold|warm>-<light|dark>"); warm-light is the app default.
    case "get_settings": {
      let theme = "warm-light";
      try {
        theme = (globalThis as any).localStorage?.getItem("skimdemo.theme") || "warm-light";
      } catch {}
      return ok({ locale: "en", theme, images_policy: "ask", group_threads: "on" });
    }
    case "set_setting":
      return ok(undefined);

    // mail
    case "list_folders":
      return ok(db.FOLDERS);
    // Threads vs. flat messages: the app picks one based on the group_threads
    // setting. The fixtures serve for both.
    case "list_threads":
    case "list_messages": {
      const list = db.THREADS_BY_FOLDER[args.folderId] ?? [];
      return ok(args.offset > 0 ? [] : list);
    }
    case "get_thread":
      return ok(db.threadDetail(args.threadId));
    case "get_message_body":
      return ok(db.renderedBody(args.messageId));
    case "thread_message_ids":
      return ok([args.threadId * 10 + 1]);
    case "take_pending_open":
      return ok(null);
    case "search_messages":
      return ok(db.searchHits(args.query ?? ""));

    // compose
    case "create_draft":
      return ok(db.createDraft());
    case "get_draft":
      return ok(db.getDraft(args.draftId));
    case "get_reply_template":
      return ok(db.replyTemplate(args.messageId, args.mode));
    case "update_draft":
      db.updateDraft(args.draft);
      return ok(undefined);
    case "list_draft_attachments":
      return ok([]);
    case "suggest_addresses":
      return ok([]);

    // AI key gate — the demo always has a key so AI actions are visible.
    case "ai_key_status":
      return ok({ provider: "anthropic", anthropic: true, openrouter: false });
    case "ai_cancel":
      cancelled.add(args.requestId);
      return ok(undefined);

    // Fire-and-forget mutations: mark read, star, archive, delete, send…
    default:
      return ok(undefined);
  }
}

export const convertFileSrc = (p: string) => p;
export const transformCallback = (cb: (r: any) => void) => cb;
export const isTauri = () => true;
export const PluginListener = class {};
