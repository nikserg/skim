// Mail data store: the frontend's mirror of the Rust cache. Refreshed on
// backend events, mutated optimistically by UI actions later.
import { listen } from "@tauri-apps/api/event";
import { api } from "../api";
import type { Account, Folder, SyncState, ThreadRow } from "../types";

const PAGE = 100;

const state = $state({
  booted: false,
  account: null as Account | null,
  folders: [] as Folder[],
  threads: [] as ThreadRow[],
  selectedFolderId: null as number | null,
  selectedThreadId: null as number | null,
  syncState: "idle" as SyncState,
  syncMessage: null as string | null,
  syncProgress: null as { done: number; total: number } | null,
  threadsLoading: false,
});

let listenersAttached = false;

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

async function refreshThreads() {
  if (state.selectedFolderId === null) return;
  state.threads = await api.listThreads(state.selectedFolderId, 0, PAGE);
}

async function selectFolder(id: number) {
  state.selectedFolderId = id;
  state.selectedThreadId = null;
  state.threadsLoading = true;
  try {
    state.threads = await api.listThreads(id, 0, PAGE);
  } finally {
    state.threadsLoading = false;
  }
}

async function loadMoreThreads() {
  if (state.selectedFolderId === null) return;
  const more = await api.listThreads(state.selectedFolderId, state.threads.length, PAGE);
  state.threads = [...state.threads, ...more];
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
  get threadsLoading() {
    return state.threadsLoading;
  },

  /** App start: find the account and begin listening. */
  async boot() {
    await attachListeners();
    const accounts = await api.listAccounts();
    state.account = accounts[0] ?? null;
    if (state.account) await refreshFolders();
    state.booted = true;
  },

  /** Called right after onboarding adds the account. */
  async accountAdded(account: Account) {
    state.account = account;
    await refreshFolders();
  },

  selectFolder,
  loadMoreThreads,
  refreshThreads,
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
