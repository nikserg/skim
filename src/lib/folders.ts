// Presentation of a folder: its localized name and its icon. Shared so the
// sidebar, the list header, the palette and the move picker never disagree
// about what a folder is called or what it looks like.
import { t } from "./i18n/index.svelte";
import type { Folder } from "./types";

const roleKey: Record<string, string> = {
  inbox: "nav.inbox",
  starred: "nav.starred",
  sent: "nav.sent",
  drafts: "nav.drafts",
  archive: "nav.archive",
  trash: "nav.trash",
  junk: "nav.junk",
  all: "nav.all_mail",
};

/** 16×16 path data, drawn as a stroked outline. */
export const roleIcon: Record<string, string> = {
  inbox: "M2 8l5-5h2l5 5v4a1 1 0 0 1-1 1H3a1 1 0 0 1-1-1V8zm0 0h3.5a2.5 2.5 0 0 0 5 0H14",
  starred: "M8 1.5l2 4.1 4.5.6-3.3 3.2.8 4.5L8 11.8l-4 2.1.8-4.5L1.5 6.2 6 5.6 8 1.5z",
  sent: "M14 2L2 7l4.5 2L8 14l6-12zM6.5 9L14 2",
  drafts: "M3 2h7l3 3v9H3V2zm7 0v3h3M5.5 8h5M5.5 11h5",
  archive: "M2 3h12v3H2V3zm1 3v7h10V6M6.5 9h3",
  trash: "M3 4h10M6.5 4V2.5h3V4M4.5 4l.5 9.5h6l.5-9.5M6.7 6.5v5M9.3 6.5v5",
  junk: "M8 2a6 6 0 1 0 0 12A6 6 0 0 0 8 2zM3.5 3.5l9 9",
};

/** A folder's path, split into its own segments. Servers nest with their own
 *  separator ("/" on Gmail, "." on Dovecot), and many of them root every
 *  mailbox under INBOX — that leading segment is the row already drawn at the
 *  top of the sidebar, not a folder of the user's, so it goes. */
export function folderSegments(folder: Pick<Folder, "displayName" | "delimiter">): string[] {
  const delimiter = folder.delimiter || "/";
  const segments = folder.displayName.split(delimiter);
  return segments.length > 1 && segments[0].toUpperCase() === "INBOX"
    ? segments.slice(1)
    : segments;
}

/** What to call a folder: the translated role name, or the server's own name
 *  for a user label — its full path minus the INBOX container, always spelled
 *  with "/" (which is what the user types when creating one). */
export function folderLabel(folder: Pick<Folder, "role" | "displayName" | "delimiter">): string {
  const key = folder.role ? roleKey[folder.role] : undefined;
  return key ? t(key) : folderSegments(folder).join("/");
}

/** One row of the sidebar's folder list: the folder, how deep to indent it, and
 *  the part of its path worth printing. */
export interface FolderNode {
  folder: Folder;
  depth: number;
  label: string;
}

/** Order user folders as a tree: children right below their parent, indented,
 *  labelled with only the segments the row above doesn't already show. A folder
 *  whose parent isn't in the list (an unselectable container, say) keeps its
 *  whole path and sits at the root — the row never hides what nothing else
 *  spells out. */
export function folderTree(folders: Folder[]): FolderNode[] {
  const paths = folders.map((folder) => folderSegments(folder));
  const order = folders.map((_, i) => i);
  order.sort((a, b) => {
    const x = paths[a];
    const y = paths[b];
    for (let i = 0; i < Math.min(x.length, y.length); i++) {
      const cmp = x[i].localeCompare(y[i]);
      if (cmp !== 0) return cmp;
    }
    return x.length - y.length;
  });

  // Parents always come first in that order, so their depth is known by the
  // time a child looks it up.
  const depthByPath = new Map<string, number>();
  const key = (segments: string[]) => segments.join("\u0000").toLowerCase();
  return order.map((i) => {
    const segments = paths[i];
    let depth = 0;
    let cut = 0;
    for (let n = segments.length - 1; n > 0; n--) {
      const parent = depthByPath.get(key(segments.slice(0, n)));
      if (parent !== undefined) {
        depth = parent + 1;
        cut = n;
        break;
      }
    }
    depthByPath.set(key(segments), depth);
    return { folder: folders[i], depth, label: segments.slice(cut).join("/") };
  });
}

/** Icon path for a folder, falling back to the inbox glyph. */
export function folderIcon(role: string | null): string {
  return roleIcon[role ?? "inbox"] ?? roleIcon.inbox;
}

/** What to call the user's own folders in this mailbox. Gmail has no folders —
 *  every one of its mailboxes is a label, and that is the word its users know.
 *  Everywhere else "labels" would be a lie, so the heading follows the provider
 *  rather than picking one term and being wrong half the time. Mixed accounts in
 *  the unified view fall back to the neutral one. */
export function ownFoldersHeading(providers: string[]): string {
  const allGmail = providers.length > 0 && providers.every((p) => p === "gmail");
  return t(allGmail ? "nav.labels" : "nav.folders");
}
