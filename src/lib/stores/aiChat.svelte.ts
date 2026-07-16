export type AskTurn = { role: "user" | "assistant"; content: string };

// How many per-message chats we keep around. A chat is a transient reading
// aid, not durable data, so it lives in memory only — but it must survive
// closing/reopening the dock and hopping between emails. We remember the last
// MAX message chats (LRU by last touch); older ones fall off.
const MAX = 10;

// messageId -> turns. A Map keeps insertion order, so the oldest key is the
// LRU eviction victim. Plain (non-reactive) — the reading pane renders from its
// own local `askTurns`; this is just the backing cache.
const chats = new Map<number, AskTurn[]>();

export const aiChat = {
  /** The stored chat for a message, or an empty array. Returns a fresh copy. */
  get(id: number): AskTurn[] {
    return [...(chats.get(id) ?? [])];
  },
  /** Persist (or clear, when empty) a message's chat and mark it most-recent. */
  save(id: number, turns: AskTurn[]) {
    chats.delete(id);
    if (turns.length === 0) return;
    chats.set(id, turns);
    while (chats.size > MAX) {
      const oldest = chats.keys().next().value;
      if (oldest === undefined) break;
      chats.delete(oldest);
    }
  },
};
