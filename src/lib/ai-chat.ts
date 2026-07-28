// The shape of an AI chat and the one function that advances it.
//
// A chat can be shown in two places at once-ish: inline (the reading pane dock,
// the palette) and, while the user wants room, in a window of its own. The main
// window always owns the session and the request — the chat window is a view
// that replays the same events onto its own copy. Keeping the reducer here means
// both sides can't drift.
import type { Citation } from "./api";

export interface ChatTurn {
  role: "user" | "assistant";
  content: string;
  /** Emails the answer cited (the mailbox-wide chat only). */
  citations: Citation[];
}

/** A step in the reasoning trace: "search" / "read" (mailbox chat) or "fetch"
 *  (a link the open email contains). */
export interface ChatStep {
  id: string;
  kind: string;
  arg: string;
  count: number | null;
  done: boolean;
}

/** "ask" is the chat about one email, "chat" the mailbox-wide one, "recap" the
 *  unread digest — which becomes a mailbox chat once the digest lands. */
export type ChatKind = "ask" | "chat" | "recap";

export interface ChatSession {
  id: number;
  kind: ChatKind;
  /** Names the window a pop-out opens: the subject, or the opening question. */
  title: string;
  /** ask: the email the chat is about. chat: the email that was open when it
   *  started, which the backend folds into the first turn. */
  messageId: number | null;
  /** recap: the folder being digested. */
  folderId: number | null;
  /** ask only: local heuristics flagged the email, so offer the phishing chip. */
  flagged: boolean;
  /** recap: how far the scan has got, until the digest starts arriving. */
  progress: { current: number; total: number } | null;
  /** recap: how many unread emails the scan covered, for the eyebrow. */
  scannedTotal: number;
  /** recap: how many the digest marked read. */
  markedCount: number;
  turns: ChatTurn[];
  /** The in-flight answer, streaming in. */
  answer: string;
  /** The model reasoned this round: it is alive and working, even though
   *  there is nothing to show yet. Reasoning itself is never rendered. */
  reasoning: boolean;
  steps: ChatStep[];
  status: "idle" | "streaming" | "error";
  errorText: string;
  /** A question handed back after a failed round, for the input to pick up. */
  pending: string;
  /** A window is showing this chat; the inline surface stands down. */
  detached: boolean;
  /** The inline surface shows it (dock open / palette in chat mode). */
  open: boolean;
}

export type ChatEvent =
  | { t: "user"; content: string }
  /** A round with no question of its own to show: the recap's opening scan. */
  | { t: "start" }
  | { t: "progress"; current: number; total: number }
  | { t: "delta"; text: string }
  /** The model is reasoning: nothing to render, but it is not stuck. */
  | { t: "reasoning" }
  /** The user stopped the round: keep the conversation, drop what was in flight. */
  | { t: "cancelled" }
  | { t: "toolCall"; id: string; kind: string; arg: string }
  | { t: "toolDone"; id: string; count: number | null }
  | { t: "answer"; content: string; citations: Citation[] }
  /** The digest landed: it opens the conversation as a reply to `seed`, which
   *  is sent to the model but never rendered. */
  | { t: "recap"; content: string; citations: Citation[]; seed: string }
  | { t: "marked"; n: number }
  | { t: "fail"; why: string };

export function newSession(
  id: number,
  kind: ChatKind,
  fields: Partial<ChatSession> = {},
): ChatSession {
  return {
    id,
    kind,
    title: "",
    messageId: null,
    folderId: null,
    flagged: false,
    progress: null,
    scannedTotal: 0,
    markedCount: 0,
    turns: [],
    answer: "",
    reasoning: false,
    steps: [],
    status: "idle",
    errorText: "",
    pending: "",
    detached: false,
    open: true,
    ...fields,
  };
}

/** Has the model shown any sign of life this round? Reasoning, an answer under
 *  way, or a tool it reached for all mean the same thing to the UI: it is
 *  working, so a "still loading" hint has no business being up. */
export function isAlive(s: ChatSession): boolean {
  return s.reasoning || s.answer !== "" || s.steps.length > 0;
}

/** Advance a session. Called on the owning side as the request streams, and on
 *  the window side as the same events arrive over IPC. */
export function applyChatEvent(s: ChatSession, ev: ChatEvent): void {
  switch (ev.t) {
    case "user":
      s.turns = [...s.turns, { role: "user", content: ev.content, citations: [] }];
      s.answer = "";
      s.reasoning = false;
      s.steps = [];
      s.status = "streaming";
      s.errorText = "";
      s.pending = "";
      if (!s.title) s.title = ev.content;
      break;
    case "start":
      s.answer = "";
      s.reasoning = false;
      s.steps = [];
      s.status = "streaming";
      s.errorText = "";
      break;
    case "progress":
      s.progress = { current: ev.current, total: ev.total };
      if (ev.total > s.scannedTotal) s.scannedTotal = ev.total;
      break;
    case "delta":
      // Text means the scan is over and the digest is being written.
      s.progress = null;
      s.answer += ev.text;
      break;
    case "reasoning":
      s.reasoning = true;
      break;
    case "cancelled":
      s.answer = "";
      s.reasoning = false;
      s.steps = [];
      s.progress = null;
      s.status = "idle";
      break;
    case "toolCall":
      s.steps = [...s.steps, { id: ev.id, kind: ev.kind, arg: ev.arg, count: null, done: false }];
      break;
    case "toolDone":
      s.steps = s.steps.map((step) =>
        step.id === ev.id ? { ...step, count: ev.count, done: true } : step,
      );
      break;
    case "answer":
      s.turns = [...s.turns, { role: "assistant", content: ev.content, citations: ev.citations }];
      s.answer = "";
      s.reasoning = false;
      s.steps = [];
      s.status = "idle";
      break;
    case "recap":
      // The digest becomes the first assistant reply; its citations seed the
      // [N] numbering so follow-ups can resolve those same emails.
      s.turns = [
        { role: "user", content: ev.seed, citations: [] },
        { role: "assistant", content: ev.content, citations: ev.citations },
      ];
      s.answer = "";
      s.reasoning = false;
      s.steps = [];
      s.progress = null;
      s.status = "idle";
      break;
    case "marked":
      s.markedCount = ev.n;
      break;
    case "fail": {
      // A round that produced nothing is a failure however it ended: say why,
      // and hand the unanswered question back so it can be retried.
      const last = s.turns[s.turns.length - 1];
      if (last?.role === "user") {
        s.pending = last.content;
        s.turns = s.turns.slice(0, -1);
      }
      s.answer = "";
      s.reasoning = false;
      s.steps = [];
      s.progress = null;
      s.status = "error";
      s.errorText = ev.why;
      break;
    }
  }
}
