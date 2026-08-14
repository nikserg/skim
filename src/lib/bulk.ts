// Actions over the ticked rows. Shared by the selection bar's buttons and the
// keyboard handler so both go through exactly one code path.
import { api } from "./api";
import { mail, rowKey } from "./stores/mail.svelte";
import { ui } from "./stores/ui.svelte";
import type { ThreadRow } from "./types";

export type BulkAction = "archive" | "delete" | "spam" | "read";

/** SQLite binds a bounded number of variables per statement, so a very large
 *  selection is resolved in a few calls rather than one enormous one. */
const CHUNK = 500;

/** The message ids behind a set of rows.
 *
 *  Grouping off means each row *is* a message and already carries its id, so
 *  the action stays on exactly the rows that were ticked. Grouping on means a
 *  row stands for a whole conversation, whose messages have to be looked up —
 *  batched, because doing it per row would be one IPC round trip per tick. */
async function messageIdsFor(rows: ThreadRow[]): Promise<number[]> {
  const ids = rows.filter((r) => r.messageId != null).map((r) => r.messageId as number);
  const threadIds = rows.filter((r) => r.messageId == null).map((r) => r.id);
  for (let i = 0; i < threadIds.length; i += CHUNK) {
    ids.push(...(await api.threadMessageIdsBulk(threadIds.slice(i, i + CHUNK))));
  }
  return ids;
}

/** Apply a bulk action to the current selection. Optimistic, like every other
 *  mutation in Skim: the rows leave now and the queued op catches the server
 *  up, with `ops:failed` refreshing the list back if it never lands. */
export async function bulkAct(action: BulkAction): Promise<void> {
  const rows = mail.selectedRows;
  if (rows.length === 0) return;
  const keys = rows.map(rowKey);
  // Resolve before the optimistic removal — `selectedRows` reads the live list.
  const ids = await messageIdsFor(rows);
  if (ids.length === 0) return;

  if (action === "read") {
    // One decision made for the user rather than two buttons: anything unread
    // in the selection means "mark read", otherwise flip them back to unread.
    const read = rows.some((r) => !r.isRead);
    mail.patchRows(keys, { isRead: read });
    void api.markRead(ids, read);
    mail.clearSelection();
    return;
  }

  mail.removeRowsFromList(keys);
  if (action === "archive") void api.archiveMessages(ids);
  else if (action === "delete") void api.deleteMessages(ids);
  else void api.reportSpam(ids);
  mail.clearSelection();
}

/** Open the folder picker for the selection. Move is the one action IMAP
 *  cannot do across mailboxes, so a mixed-account selection never gets here. */
export async function bulkMove(): Promise<void> {
  const rows = mail.selectedRows;
  if (rows.length === 0 || mail.selectionSpansAccounts) return;
  const ids = await messageIdsFor(rows);
  if (ids.length === 0) return;
  ui.openMove({ rowKeys: rows.map(rowKey), messageIds: ids });
}
