// Mail data store: the frontend's mirror of the Rust cache. Refreshed on
// backend events, mutated optimistically by UI actions later.
import { listen } from "@tauri-apps/api/event";
import { api } from "../api";
import { t } from "../i18n/index.svelte";
import type { Account, Folder, SyncState, ThreadRow } from "../types";

const PAGE = 100;

/** Sentinel "account id" meaning every mailbox at once — the unified view.
 *  Persisted in the `active_account` setting like a real id. */
export const UNIFIED = "*";

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
// Last reported engine state per account — the unified view's indicator
// aggregates these instead of following one mailbox.
const syncByAccount = new Map<string, { state: SyncState; message: string | null }>();

function isUnified(): boolean {
  return state.activeAccountId === UNIFIED && state.accounts.length > 1;
}

/** One indicator over every engine: busy wins, then trouble, then quiet. */
function applyAggregateSyncState() {
  const states = [...syncByAccount.values()];
  const pick =
    states.find((s) => s.state === "syncing") ??
    states.find((s) => s.state === "error") ??
    states.find((s) => s.state === "offline") ??
    null;
  state.syncState = pick?.state ?? "idle";
  state.syncMessage = pick?.message ?? null;
  if (state.syncState !== "syncing") state.syncProgress = null;
}

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
    // The unified list can show any folder's mail, so every update may
    // concern it — one bounded page query, cheap enough to just refresh.
    if (!e.payload?.folderId || isUnified() || e.payload.folderId === state.selectedFolderId) {
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
  // sync indicator (so a background mailbox can't clobber it), except in the
  // unified view, where the indicator aggregates all of them.
  await listen<{ state: SyncState; message: string | null; accountId?: string }>(
    "sync:status",
    (e) => {
      if (e.payload.accountId) {
        syncByAccount.set(e.payload.accountId, {
          state: e.payload.state,
          message: e.payload.message ?? null,
        });
      }
      if (isUnified()) {
        applyAggregateSyncState();
        return;
      }
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
  const folders =
    accountId === UNIFIED ? await api.listUnifiedFolders() : await api.listFolders(accountId);
  // The user may have switched accounts mid-fetch — these folders belong to
  // the previous mailbox.
  if (state.activeAccountId !== accountId) return;
  state.folders = folders;
  // Auto-select inbox once it appears — also when the selected folder is gone
  // (e.g. a virtual label vanished with its last message).
  if (
    state.selectedFolderId === null ||
    !state.folders.some((f) => f.id === state.selectedFolderId)
  ) {
    const inbox = state.folders.find((f) => f.role === "inbox");
    if (inbox) await selectFolder(inbox.id);
  }
}

/** Make another mailbox the active one: reset the view, load its folders
 *  (auto-selecting the inbox), and remember the choice across restarts. */
async function switchAccount(id: string) {
  const valid =
    id === UNIFIED ? state.accounts.length > 1 : state.accounts.some((a) => a.id === id);
  if (id === state.activeAccountId || !valid) return;
  state.activeAccountId = id;
  state.selectedFolderId = null;
  state.selectedThreadId = null;
  state.selectedMessageId = null;
  state.folders = [];
  state.threads = [];
  state.syncState = "idle";
  state.syncMessage = null;
  state.syncProgress = null;
  if (id === UNIFIED) applyAggregateSyncState();
  void api.setSetting("active_account", id);
  await refreshFolders();
}

/** Open a folder/thread/message wherever it lives — switching the active
 *  account first when the target belongs to another mailbox (toast clicks,
 *  cold-start pending opens, AI citations, search hits). */
async function openLocation(folderId: number, threadId: number | null, messageId: number) {
  if (isUnified()) {
    // The unified view has no real folders — map the target onto its virtual
    // counterpart (same role, or same label name).
    let ref: { role: string | null; displayName: string };
    try {
      ref = await api.folderRef(folderId);
    } catch {
      return; // the folder is gone (stale hit) — nothing to open
    }
    const target = state.folders.find((f) =>
      ref.role !== null
        ? f.role === ref.role
        : f.role === null && f.displayName.toLowerCase() === ref.displayName.toLowerCase(),
    );
    if (!target) return;
    if (target.id !== state.selectedFolderId) await selectFolder(target.id);
  } else {
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
  }
  if (threadId === null) return;
  state.selectedThreadId = threadId;
  // Match a normal click: in flat mode the list highlights by message id, so
  // point it at the opened message; grouped mode highlights by thread and
  // wants a null message id to keep the conversation view.
  state.selectedMessageId = state.groupThreads ? null : messageId;
}

/** One page of rows for a folder — threads when grouping is on, else messages.
 *  Negative ids are virtual (cross-account) folders, addressed by role/label. */
function fetchPage(folderId: number, offset: number): Promise<ThreadRow[]> {
  if (folderId < 0) {
    const virtual = state.folders.find((f) => f.id === folderId);
    if (!virtual) return Promise.resolve([]);
    const label = virtual.role === null ? virtual.displayName : null;
    return state.groupThreads
      ? api.listUnifiedThreads(virtual.role, label, offset, PAGE)
      : api.listUnifiedMessages(virtual.role, label, offset, PAGE);
  }
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
  /** The active account — the mailbox the whole UI currently shows.
   *  `null` in the unified view, which spans every mailbox. */
  get account() {
    return activeAccount();
  },
  /** Whether the unified ("All inboxes") view is active. */
  get unified() {
    return isUnified();
  },
  get accounts() {
    return state.accounts;
  },
  /** Lowercased addresses the user owns in the current scope — for
   *  "is this message mine?" checks. */
  get myEmails(): string[] {
    return (isUnified() ? state.accounts : state.accounts.filter((a) => a.id === state.activeAccountId))
      .map((a) => a.email.toLowerCase());
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
    // Restore the last scope; self-heal a stale id. With 2+ mailboxes the
    // unified view is the default — a concrete saved choice is respected,
    // anything else (unified, stale, missing) lands in "All inboxes".
    const saved = settings.active_account;
    const savedAccount = state.accounts.find((a) => a.id === saved)?.id;
    state.activeAccountId =
      state.accounts.length > 1
        ? (savedAccount ?? UNIFIED)
        : (savedAccount ?? state.accounts[0]?.id ?? null);
    if (state.activeAccountId) await refreshFolders();
    state.booted = true;
    // A cold-start toast click may have queued a thread to open (the
    // mail:open-thread event fired before listeners were attached).
    const pending = await api.takePendingOpen();
    if (pending) await openLocation(pending.folderId, pending.threadId, pending.messageId);
  },

  /** Called right after onboarding or settings connects a mailbox. A second
   *  mailbox turns on the unified view — that's its default experience. */
  async accountAdded(account: Account) {
    state.accounts = [...state.accounts, account];
    if (state.activeAccountId === UNIFIED) {
      // Already unified — just fold the new mailbox in as it syncs.
      void refreshFolders();
      void refreshThreads();
      return;
    }
    await switchAccount(state.accounts.length > 1 ? UNIFIED : account.id);
  },

  /** Called right after settings disconnects a mailbox. */
  async accountRemoved(id: string) {
    state.accounts = state.accounts.filter((a) => a.id !== id);
    syncByAccount.delete(id);
    if (state.accounts.length === 0) {
      // Last mailbox gone — a clean reload lands on onboarding.
      window.location.reload();
      return;
    }
    if (state.activeAccountId === id) {
      state.activeAccountId = null;
      await switchAccount(state.accounts.length > 1 ? UNIFIED : state.accounts[0].id);
    } else if (state.activeAccountId === UNIFIED) {
      if (state.accounts.length === 1) {
        // Unified collapses back to the lone mailbox.
        await switchAccount(state.accounts[0].id);
      } else {
        await refreshFolders();
        await refreshThreads();
      }
    }
  },

  /** Colored dot + letter identifying a row's mailbox in the unified list.
   *  Colors follow the stable account order and repeat past five. */
  accountBadge(accountId: string): { letter: string; color: number } | null {
    const i = state.accounts.findIndex((a) => a.id === accountId);
    if (i < 0) return null;
    return { letter: state.accounts[i].email[0]?.toLowerCase() ?? "?", color: (i % 5) + 1 };
  },

  /** Which mailbox a fresh compose should send from: the active one, or in
   *  the unified view the mailbox the user last sent from. */
  async composeAccountId(): Promise<string | undefined> {
    if (!isUnified()) return activeAccount()?.id;
    const last = (await api.getSettings()).last_from_account;
    return state.accounts.find((a) => a.id === last)?.id ?? state.accounts[0]?.id;
  },

  selectFolder,
  loadMoreThreads,
  refreshThreads,
  setGroupThreads,
  switchAccount,
  openLocation,
  // In the unified view the active account is null, so this syncs every engine.
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
