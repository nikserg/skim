// Mail data store: the frontend's mirror of the Rust cache. Refreshed on
// backend events, mutated optimistically by UI actions later.
import { listen } from "@tauri-apps/api/event";
import { api, reportError } from "../api";
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
  // How many rows have been FETCHED for the current folder — the paging offset.
  // Deliberately not `threads.length`: an optimistic archive/delete shrinks the
  // list, and paging from that shorter length skips exactly as many server rows
  // as were removed. One row per action went unnoticed; bulk removes dozens.
  fetched: 0,
  // Rows ticked for a bulk action, by `rowKey`. Empty means "no selection" and
  // the list shows no selection chrome at all.
  selectedKeys: [] as number[],
  // Where the last tick happened, so Shift+click can extend a range from it.
  anchorKey: null as number | null,
  // Transient notice for a queued op that failed after all retries.
  opError: null as string | null,
  // The last attempt to read folders or mail out of the cache failed. Without
  // this an empty list is ambiguous, and the app answers "nothing to skim" to
  // a mailbox full of mail — the one thing it must never do.
  loadFailed: false,
});

/** Membership lookup for the selection. Rebuilt only when the selection
 *  changes, so the per-row check while rendering stays O(1). */
const selectedSet = $derived(new Set(state.selectedKeys));

/** The canonical identity of a list row. Rows are threads when grouping is on
 *  and messages when it is off, so this is the one expression that identifies a
 *  row in either mode — used for `{#each}` keys, paging dedupe and selection.
 *  The two id spaces can collide numerically, which is safe only because the
 *  selection is cleared whenever the grouping mode changes. */
export function rowKey(t: ThreadRow): number {
  return t.messageId ?? t.id;
}

let listenersAttached = false;
/** A refresh arrived before there was an account (or a folder) to refresh. The
 *  engines start syncing in Rust's `setup`, so their first `folders:updated`
 *  routinely beats the webview's own boot queries; dropping it used to cost the
 *  user a full poll interval of staring at an empty list. */
let missedRefresh = false;
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
  // Back from the tray. Hiding the window only hides the webview, so nothing
  // here re-runs on its own — and a view left empty by a failed read would
  // stay empty however many times the user reopened it. Cheap local queries,
  // so they run only when there is something to put right.
  await listen("window:shown", () => {
    if (state.loadFailed || state.threads.length === 0) void retryLoad();
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
      kind === "rsvp"
        ? "ops.rsvp_failed"
        : kind === "send"
          ? "ops.send_failed"
          : kind === "move"
            ? "ops.move_failed"
            : "ops.failed";
    showOpError(t(key));
    void refreshThreads();
    void refreshFolders();
  });
}

function activeAccount(): Account | null {
  return state.accounts.find((a) => a.id === state.activeAccountId) ?? null;
}

/** Every read of the cache the list depends on goes through here. A rejected
 *  query used to end as an unhandled rejection in `skim-frontend.log` while the
 *  UI quietly claimed the folder was empty; now it says so, and can be retried.
 *  Returns null when the read failed — the caller keeps what it had.
 *
 *  Only a page of rows arriving clears the flag again (`shown`), never a folder
 *  read: refreshes run in pairs, and a folder query that outlives its sibling's
 *  failure must not talk the list back into trusting itself. */
async function guard<T>(read: () => Promise<T>): Promise<T | null> {
  try {
    return await read();
  } catch (e) {
    state.loadFailed = true;
    reportError("mail.load", e);
    return null;
  }
}

/** The rows the list is about to render came out of the cache — whatever went
 *  wrong before, the view is honest again. */
function shown(rows: ThreadRow[]) {
  state.threads = rows;
  state.fetched = rows.length;
  state.loadFailed = false;
}

/** Read the folders and the current page again after a failed load. Local
 *  SQLite queries, so it costs nothing to offer and nothing to take. */
async function retryLoad() {
  await refreshFolders();
  await refreshThreads();
}

async function refreshFolders() {
  const accountId = state.activeAccountId;
  if (accountId === null) {
    missedRefresh = true;
    return;
  }
  const folders = await guard(() =>
    accountId === UNIFIED ? api.listUnifiedFolders() : api.listFolders(accountId),
  );
  if (folders === null) return;
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
  state.fetched = 0;
  clearSelection();
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
function fetchPage(folderId: number, offset: number, limit = PAGE): Promise<ThreadRow[]> {
  if (folderId < 0) {
    const virtual = state.folders.find((f) => f.id === folderId);
    if (!virtual) return Promise.resolve([]);
    const label = virtual.role === null ? virtual.displayName : null;
    return state.groupThreads
      ? api.listUnifiedThreads(virtual.role, label, offset, limit)
      : api.listUnifiedMessages(virtual.role, label, offset, limit);
  }
  return state.groupThreads
    ? api.listThreads(folderId, offset, limit)
    : api.listMessages(folderId, offset, limit);
}

/** Deepest a refresh will re-read. A list scrolled thousands of rows down does
 *  not need all of them re-fetched every time a message arrives. */
const MAX_REFRESH = 500;

async function refreshThreads() {
  const folderId = state.selectedFolderId;
  if (folderId === null) {
    missedRefresh = true;
    return;
  }
  // Reload as deep as the list already is, not just the first page: otherwise
  // every `mail:updated` — including the one each bulk action emits — snaps a
  // scrolled list back to 100 rows under the user.
  const depth = Math.min(
    Math.max(PAGE, Math.ceil(state.threads.length / PAGE) * PAGE),
    MAX_REFRESH,
  );
  const rows = await guard(() => fetchPage(folderId, 0, depth));
  if (rows === null) return;
  shown(rows);
  pruneSelection();
}

async function selectFolder(id: number) {
  state.selectedFolderId = id;
  state.selectedThreadId = null;
  state.selectedMessageId = null;
  clearSelection();
  state.threadsLoading = true;
  try {
    const rows = await guard(() => fetchPage(id, 0));
    // A failed read must not leave the previous folder's mail under the new
    // folder's name — the list says "couldn't load" instead.
    if (rows === null) {
      state.threads = [];
      state.fetched = 0;
    } else {
      shown(rows);
    }
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
    const more = await fetchPage(folderId, state.fetched);
    // The user may have switched folders (or grouping) mid-fetch — these rows
    // belong to the previous view, don't append them to the new one.
    if (state.selectedFolderId !== folderId || state.groupThreads !== grouped) return;
    state.fetched += more.length;
    // A concurrent refresh can shift the offset; drop rows we already show.
    const seen = new Set(state.threads.map(rowKey));
    state.threads = [...state.threads, ...more.filter((t) => !seen.has(rowKey(t)))];
  } finally {
    loadingMore = false;
  }
}

function clearSelection() {
  state.selectedKeys = [];
  state.anchorKey = null;
}

/** Drop keys whose row is no longer on screen. `refreshThreads` truncates the
 *  list back to one page, so without this a selection could outlive its rows
 *  and act on mail the user can't see. */
function pruneSelection() {
  if (state.selectedKeys.length === 0) return;
  const live = new Set(state.threads.map(rowKey));
  state.selectedKeys = state.selectedKeys.filter((k) => live.has(k));
  if (state.anchorKey !== null && !live.has(state.anchorKey)) state.anchorKey = null;
}

/** Tick or untick one row. With `extend`, cover everything between the last
 *  ticked row and this one instead — a Shift+click range. */
function toggleRow(key: number, extend = false) {
  const anchor = state.anchorKey;
  if (extend && anchor !== null && anchor !== key) {
    const keys = state.threads.map(rowKey);
    const from = keys.indexOf(anchor);
    const to = keys.indexOf(key);
    if (from !== -1 && to !== -1) {
      const range = keys.slice(Math.min(from, to), Math.max(from, to) + 1);
      // A range always adds; dragging back over it should not clear what the
      // user just covered.
      const merged = new Set(state.selectedKeys);
      for (const k of range) merged.add(k);
      state.selectedKeys = [...merged];
      state.anchorKey = key;
      return;
    }
  }
  state.selectedKeys = selectedSet.has(key)
    ? state.selectedKeys.filter((k) => k !== key)
    : [...state.selectedKeys, key];
  state.anchorKey = key;
}

/** Select every row currently loaded — deliberately not "every message in the
 *  folder". Skim has no undo, so a bulk action must never reach mail that was
 *  never on screen. */
function selectAllLoaded() {
  state.selectedKeys = state.threads.map(rowKey);
  state.anchorKey = null;
}

/** Toggle thread grouping and reload the current folder in the new mode. */
async function setGroupThreads(on: boolean) {
  if (state.groupThreads === on) return;
  state.groupThreads = on;
  state.selectedThreadId = null;
  state.selectedMessageId = null;
  // Row keys mean different things in the two modes, so a selection cannot
  // survive the switch.
  clearSelection();
  if (state.selectedFolderId !== null) {
    state.threadsLoading = true;
    try {
      state.threads = await fetchPage(state.selectedFolderId, 0);
      state.fetched = state.threads.length;
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

  // ── Bulk selection ────────────────────────────────────────────────────────
  /** The ticked rows themselves. Derived from the live list, so a row that has
   *  gone (refresh, optimistic removal) can never be acted on. */
  get selectedRows(): ThreadRow[] {
    return state.selectedKeys.length === 0
      ? []
      : state.threads.filter((t) => selectedSet.has(rowKey(t)));
  },
  get selectionCount() {
    return this.selectedRows.length;
  },
  /** True once anything is ticked — the list shows checkboxes on every row and
   *  the header turns into the action bar. */
  get selecting() {
    return state.selectedKeys.length > 0;
  },
  /** Move is the one bulk action that cannot span mailboxes: IMAP has no
   *  cross-account MOVE, so `move_targets` returns nothing for such a set. */
  get selectionSpansAccounts() {
    const rows = this.selectedRows;
    return rows.length > 1 && rows.some((t) => t.accountId !== rows[0].accountId);
  },
  isSelected: (key: number) => selectedSet.has(key),
  /** Every loaded row belonging to a thread, for the actions that still work
   *  one conversation at a time. */
  rowKeysForThread: (threadId: number) =>
    state.threads.filter((t) => t.id === threadId).map(rowKey),
  toggleRow,
  selectAllLoaded,
  clearSelection,

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
  /** Surface a failure the user should know about, from outside this store. */
  noteError(message: string) {
    showOpError(message);
  },
  get threadsLoading() {
    return state.threadsLoading;
  },
  get loadFailed() {
    return state.loadFailed;
  },
  retryLoad,

  /** App start: find the accounts and begin listening. Returns as soon as the
   *  shell knows which mailboxes exist — that is all it needs to draw itself.
   *  Folders and the first page of mail are queries against a database the
   *  startup sync is busy filling, so they can take a while; they land on their
   *  own rather than holding the whole window back. */
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
    state.booted = true;
    if (state.activeAccountId === null) return;
    // The list holds its loading state from here until the folders land, so a
    // mailbox that has plenty is never captioned "nothing here" on the way in.
    state.threadsLoading = true;
    void (async () => {
      try {
        await refreshFolders();
        // A cold-start toast click may have queued a thread to open (the
        // mail:open-thread event fired before listeners were attached). It
        // reads the folders, so it waits for them — not for boot().
        const pending = await api.takePendingOpen();
        if (pending) await openLocation(pending.folderId, pending.threadId, pending.messageId);
      } catch {
        showOpError(t("ops.failed"));
      } finally {
        state.threadsLoading = false;
      }
      // Sync events that fired while there was still no mailbox (or no folder)
      // to refresh. There is now — collect them into one pass, instead of
      // leaving the list a poll interval behind the mail it already has.
      if (missedRefresh) {
        missedRefresh = false;
        await refreshFolders();
        await refreshThreads();
      }
    })();
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

  /** Optimistically drop a thread from the visible list (archive/delete).
   *  Every row of the thread goes: the callers all act on the whole thread. */
  removeThreadFromList(threadId: number) {
    const gone = state.threads.filter((t) => t.id === threadId).map(rowKey);
    state.threads = state.threads.filter((t) => t.id !== threadId);
    if (state.selectedThreadId === threadId) {
      state.selectedThreadId = null;
      state.selectedMessageId = null;
    }
    if (gone.length > 0) {
      const dropped = new Set(gone);
      state.selectedKeys = state.selectedKeys.filter((k) => !dropped.has(k));
    }
  },

  /** Optimistically drop specific rows — the bulk path, which acts on exactly
   *  the rows that were ticked rather than on whole threads. */
  removeRowsFromList(keys: number[]) {
    const dropped = new Set(keys);
    const wasHighlighted = state.threads.some(
      (t) =>
        dropped.has(rowKey(t)) &&
        (state.groupThreads
          ? t.id === state.selectedThreadId
          : t.messageId === state.selectedMessageId),
    );
    state.threads = state.threads.filter((t) => !dropped.has(rowKey(t)));
    if (wasHighlighted) {
      state.selectedThreadId = null;
      state.selectedMessageId = null;
    }
    state.selectedKeys = state.selectedKeys.filter((k) => !dropped.has(k));
    if (state.anchorKey !== null && dropped.has(state.anchorKey)) state.anchorKey = null;
  },

  /** Optimistically patch a thread row in the visible list. */
  patchThreadRow(threadId: number, patch: Partial<ThreadRow>) {
    state.threads = state.threads.map((t) => (t.id === threadId ? { ...t, ...patch } : t));
  },

  /** Optimistically patch specific rows — the bulk counterpart, which touches
   *  exactly the ticked rows rather than every row of their threads. */
  patchRows(keys: number[], patch: Partial<ThreadRow>) {
    const touched = new Set(keys);
    state.threads = state.threads.map((t) => (touched.has(rowKey(t)) ? { ...t, ...patch } : t));
  },
};
