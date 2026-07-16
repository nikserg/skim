// Mail data store: the frontend's mirror of the Rust cache. Refreshed on
// backend events, mutated optimistically by UI actions later.
import { listen } from "@tauri-apps/api/event";
import { api } from "../api";
import { t } from "../i18n/index.svelte";
import type { Account, Folder, SyncState, ThreadRow } from "../types";

const PAGE = 100;

const state = $state({
  booted: false,
  account: null as Account | null,
  folders: [] as Folder[],
  threads: [] as ThreadRow[],
  selectedFolderId: null as number | null,
  selectedThreadId: null as number | null,
  selectedMessageId: null as number | null,
  groupThreads: true,
  syncState: "idle" as SyncState,
  syncMessage: null as string | null,
  syncProgress: null as { done: number; total: number } | null,
  threadsLoading: false,
  // Transient notice for a queued op that failed after all retries.
  opError: null as string | null,
});

let listenersAttached = false;
let opErrorTimer: ReturnType<typeof setTimeout> | null = null;

/** Show a self-clearing failure notice (a queued op gave up after retries). */
function showOpError(message: string) {
  state.opError = message;
  if (opErrorTimer) clearTimeout(opErrorTimer);
  opErrorTimer = setTimeout(() => {
    state.opError = null;
    opErrorTimer = null;
  }, 6000);
}

async function attachListeners() {
  if (listenersAttached) return;
  listenersAttached = true;

  await listen("folders:updated", () => void refreshFolders());
  await listen<{ folderId?: number }>("mail:updated", (e) => {
    if (!e.payload?.folderId || e.payload.folderId === state.selectedFolderId) {
      void refreshThreads();
    }
    void refreshFolders();
  });
  // Toast body click: jump to the message's thread.
  await listen<{ folderId: number; threadId: number }>("mail:open-thread", async (e) => {
    if (e.payload.folderId !== state.selectedFolderId) {
      await selectFolder(e.payload.folderId);
    }
    state.selectedThreadId = e.payload.threadId;
  });
  await listen<{ state: SyncState; message: string | null }>("sync:status", (e) => {
    state.syncState = e.payload.state;
    state.syncMessage = e.payload.message ?? null;
    if (e.payload.state !== "syncing") state.syncProgress = null;
  });
  await listen<{ folderId: number; done: number; total: number }>("sync:progress", (e) => {
    if (e.payload.folderId === state.selectedFolderId) {
      state.syncProgress = { done: e.payload.done, total: e.payload.total };
    }
  });
  // A queued mutation gave up after retries. Tell the user, and refresh so any
  // optimistic state the backend rolled back (e.g. a reverted RSVP) reappears.
  await listen<{ kind?: string; message?: string }>("ops:failed", (e) => {
    const kind = e.payload?.kind;
    const key =
      kind === "rsvp" ? "ops.rsvp_failed" : kind === "send" ? "ops.send_failed" : "ops.failed";
    showOpError(t(key));
    void refreshThreads();
    void refreshFolders();
  });
}

async function refreshFolders() {
  if (!state.account) return;
  state.folders = await api.listFolders(state.account.id);
  // Auto-select inbox once it appears.
  if (state.selectedFolderId === null) {
    const inbox = state.folders.find((f) => f.role === "inbox");
    if (inbox) await selectFolder(inbox.id);
  }
}

/** One page of rows for a folder — threads when grouping is on, else messages. */
function fetchPage(folderId: number, offset: number) {
  return state.groupThreads
    ? api.listThreads(folderId, offset, PAGE)
    : api.listMessages(folderId, offset, PAGE);
}

async function refreshThreads() {
  if (state.selectedFolderId === null) return;
  state.threads = await fetchPage(state.selectedFolderId, 0);
}

async function selectFolder(id: number) {
  state.selectedFolderId = id;
  state.selectedThreadId = null;
  state.selectedMessageId = null;
  state.threadsLoading = true;
  try {
    state.threads = await fetchPage(id, 0);
  } finally {
    state.threadsLoading = false;
  }
}

async function loadMoreThreads() {
  if (state.selectedFolderId === null) return;
  const more = await fetchPage(state.selectedFolderId, state.threads.length);
  state.threads = [...state.threads, ...more];
}

/** Toggle thread grouping and reload the current folder in the new mode. */
async function setGroupThreads(on: boolean) {
  if (state.groupThreads === on) return;
  state.groupThreads = on;
  state.selectedThreadId = null;
  state.selectedMessageId = null;
  if (state.selectedFolderId !== null) {
    state.threadsLoading = true;
    try {
      state.threads = await fetchPage(state.selectedFolderId, 0);
    } finally {
      state.threadsLoading = false;
    }
  }
}

export const mail = {
  get booted() {
    return state.booted;
  },
  get account() {
    return state.account;
  },
  get folders() {
    return state.folders;
  },
  get threads() {
    return state.threads;
  },
  get selectedFolderId() {
    return state.selectedFolderId;
  },
  get selectedThreadId() {
    return state.selectedThreadId;
  },
  set selectedThreadId(id: number | null) {
    state.selectedThreadId = id;
  },
  get selectedMessageId() {
    return state.selectedMessageId;
  },
  set selectedMessageId(id: number | null) {
    state.selectedMessageId = id;
  },
  get groupThreads() {
    return state.groupThreads;
  },
  get selectedFolder() {
    return state.folders.find((f) => f.id === state.selectedFolderId) ?? null;
  },
  get selectedThread() {
    return state.threads.find((t) => t.id === state.selectedThreadId) ?? null;
  },
  get syncState() {
    return state.syncState;
  },
  get syncMessage() {
    return state.syncMessage;
  },
  get syncProgress() {
    return state.syncProgress;
  },
  get opError() {
    return state.opError;
  },
  dismissOpError() {
    state.opError = null;
    if (opErrorTimer) {
      clearTimeout(opErrorTimer);
      opErrorTimer = null;
    }
  },
  get threadsLoading() {
    return state.threadsLoading;
  },

  /** App start: find the account and begin listening. */
  async boot() {
    await attachListeners();
    // Thread grouping preference (default on when the key is absent).
    const settings = await api.getSettings();
    state.groupThreads = settings.group_threads !== "off";
    const accounts = await api.listAccounts();
    state.account = accounts[0] ?? null;
    if (state.account) await refreshFolders();
    state.booted = true;
    // A cold-start toast click may have queued a thread to open (the
    // mail:open-thread event fired before listeners were attached).
    const pending = await api.takePendingOpen();
    if (pending) {
      if (pending.folderId !== state.selectedFolderId) await selectFolder(pending.folderId);
      state.selectedThreadId = pending.threadId;
    }
  },

  /** Called right after onboarding adds the account. */
  async accountAdded(account: Account) {
    state.account = account;
    await refreshFolders();
  },

  selectFolder,
  loadMoreThreads,
  refreshThreads,
  setGroupThreads,
  syncNow: () => api.syncNow(state.account?.id),

  /** Optimistically drop a thread from the visible list (archive/delete). */
  removeThreadFromList(threadId: number) {
    state.threads = state.threads.filter((t) => t.id !== threadId);
    if (state.selectedThreadId === threadId) state.selectedThreadId = null;
  },

  /** Optimistically patch a thread row in the visible list. */
  patchThreadRow(threadId: number, patch: Partial<ThreadRow>) {
    state.threads = state.threads.map((t) => (t.id === threadId ? { ...t, ...patch } : t));
  },
};
