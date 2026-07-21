// Mail data store: the frontend's mirror of the Rust cache. Refreshed on
// backend events, mutated optimistically by UI actions later.
import { listen } from "@tauri-apps/api/event";
import { api } from "../api";
import { t } from "../i18n/index.svelte";
import type { Account, Folder, SyncState, ThreadRow } from "../types";

const PAGE = 100;

const state = $state({
  booted: false,
  accounts: [] as Account[],
  activeAccountId: null as string | null,
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
  await listen<{ folderId: number; threadId: number; messageId: number }>(
    "mail:open-thread",
    (e) => void openLocation(e.payload.folderId, e.payload.threadId, e.payload.messageId),
  );
  // Every account's engine reports here — only the active one drives the
  // sync indicator, so a background mailbox can't clobber it.
  await listen<{ state: SyncState; message: string | null; accountId?: string }>(
    "sync:status",
    (e) => {
      if (e.payload.accountId && e.payload.accountId !== state.activeAccountId) return;
      state.syncState = e.payload.state;
      state.syncMessage = e.payload.message ?? null;
      if (e.payload.state !== "syncing") state.syncProgress = null;
    },
  );
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

function activeAccount(): Account | null {
  return state.accounts.find((a) => a.id === state.activeAccountId) ?? null;
}

async function refreshFolders() {
  const accountId = state.activeAccountId;
  if (accountId === null) return;
  const folders = await api.listFolders(accountId);
  // The user may have switched accounts mid-fetch — these folders belong to
  // the previous mailbox.
  if (state.activeAccountId !== accountId) return;
  state.folders = folders;
  // Auto-select inbox once it appears.
  if (state.selectedFolderId === null) {
    const inbox = state.folders.find((f) => f.role === "inbox");
    if (inbox) await selectFolder(inbox.id);
  }
}

/** Make another mailbox the active one: reset the view, load its folders
 *  (auto-selecting the inbox), and remember the choice across restarts. */
async function switchAccount(id: string) {
  if (id === state.activeAccountId || !state.accounts.some((a) => a.id === id)) return;
  state.activeAccountId = id;
  state.selectedFolderId = null;
  state.selectedThreadId = null;
  state.selectedMessageId = null;
  state.folders = [];
  state.threads = [];
  state.syncState = "idle";
  state.syncMessage = null;
  state.syncProgress = null;
  void api.setSetting("active_account", id);
  await refreshFolders();
}

/** Open a folder/thread/message wherever it lives — switching the active
 *  account first when the target belongs to another mailbox (toast clicks,
 *  cold-start pending opens, AI citations, search hits). */
async function openLocation(folderId: number, threadId: number | null, messageId: number) {
  if (!state.folders.some((f) => f.id === folderId)) {
    let owner: string;
    try {
      owner = await api.folderAccountId(folderId);
    } catch {
      return; // the folder is gone (stale hit) — nothing to open
    }
    await switchAccount(owner);
  }
  if (folderId !== state.selectedFolderId) await selectFolder(folderId);
  if (threadId === null) return;
  state.selectedThreadId = threadId;
  // Match a normal click: in flat mode the list highlights by message id, so
  // point it at the opened message; grouped mode highlights by thread and
  // wants a null message id to keep the conversation view.
  state.selectedMessageId = state.groupThreads ? null : messageId;
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

let loadingMore = false;

async function loadMoreThreads() {
  // Scroll fires this repeatedly; a second call during the await would read
  // the same offset and append the same page twice (duplicate {#each} keys).
  if (state.selectedFolderId === null || loadingMore) return;
  const folderId = state.selectedFolderId;
  const grouped = state.groupThreads;
  loadingMore = true;
  try {
    const more = await fetchPage(folderId, state.threads.length);
    // The user may have switched folders (or grouping) mid-fetch — these rows
    // belong to the previous view, don't append them to the new one.
    if (state.selectedFolderId !== folderId || state.groupThreads !== grouped) return;
    // A concurrent refresh can shift the offset; drop rows we already show.
    const seen = new Set(state.threads.map((t) => t.messageId ?? t.id));
    state.threads = [...state.threads, ...more.filter((t) => !seen.has(t.messageId ?? t.id))];
  } finally {
    loadingMore = false;
  }
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
  /** The active account — the mailbox the whole UI currently shows. */
  get account() {
    return activeAccount();
  },
  get accounts() {
    return state.accounts;
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

  /** App start: find the accounts and begin listening. */
  async boot() {
    await attachListeners();
    // Thread grouping preference (default on when the key is absent).
    const settings = await api.getSettings();
    state.groupThreads = settings.group_threads !== "off";
    state.accounts = await api.listAccounts();
    // Restore the last active account; self-heal a stale id.
    const saved = settings.active_account;
    state.activeAccountId =
      state.accounts.find((a) => a.id === saved)?.id ?? state.accounts[0]?.id ?? null;
    if (state.activeAccountId) await refreshFolders();
    state.booted = true;
    // A cold-start toast click may have queued a thread to open (the
    // mail:open-thread event fired before listeners were attached).
    const pending = await api.takePendingOpen();
    if (pending) await openLocation(pending.folderId, pending.threadId, pending.messageId);
  },

  /** Called right after onboarding or settings connects a mailbox. */
  async accountAdded(account: Account) {
    state.accounts = [...state.accounts, account];
    await switchAccount(account.id);
  },

  /** Called right after settings disconnects a mailbox. */
  async accountRemoved(id: string) {
    state.accounts = state.accounts.filter((a) => a.id !== id);
    if (state.accounts.length === 0) {
      // Last mailbox gone — a clean reload lands on onboarding.
      window.location.reload();
      return;
    }
    if (state.activeAccountId === id) {
      state.activeAccountId = null;
      await switchAccount(state.accounts[0].id);
    }
  },

  selectFolder,
  loadMoreThreads,
  refreshThreads,
  setGroupThreads,
  switchAccount,
  openLocation,
  syncNow: () => api.syncNow(activeAccount()?.id),

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
