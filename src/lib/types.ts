// Shared types mirroring the Rust IPC surface.

export interface Account {
  id: string;
  email: string;
  displayName: string | null;
  provider: string;
  imapHost: string;
  imapPort: number;
  smtpHost: string;
  smtpPort: number;
  smtpSecurity: string;
  authKind: string;
}

export interface ServerPreset {
  provider: string;
  imapHost: string;
  imapPort: number;
  smtpHost: string;
  smtpPort: number;
  smtpSecurity: string;
  needsAppPassword: boolean;
  supportsOauth: boolean;
}

export interface Folder {
  id: number;
  accountId: string;
  imapName: string;
  role: string | null;
  displayName: string;
  unreadCount: number;
  sortOrder: number;
}

export interface ThreadRow {
  id: number;
  fromName: string;
  fromAddr: string;
  subject: string;
  snippet: string;
  date: number; // unix seconds
  isRead: boolean;
  isStarred: boolean;
  hasAttachments: boolean;
  messageCount: number;
}

export interface Address {
  name: string | null;
  addr: string;
}

export interface MessageMeta {
  id: number;
  folderId: number;
  threadId: number | null;
  subject: string;
  from: Address;
  to: Address[];
  cc: Address[];
  date: number;
  snippet: string;
  isRead: boolean;
  isStarred: boolean;
  hasAttachments: boolean;
  bodyState: number;
}

export interface ThreadDetail {
  id: number;
  subject: string;
  messages: MessageMeta[];
}

export interface AttachmentMeta {
  id: number;
  messageId: number;
  filename: string | null;
  mimeType: string | null;
  size: number;
  isInline: boolean;
}

export interface InviteView {
  method: "request" | "cancel" | "reply";
  uid: string;
  sequence: number;
  summary: string | null;
  location: string | null;
  organizerName: string | null;
  organizerEmail: string | null;
  startsAt: number | null; // unix seconds, null for all-day
  endsAt: number | null;
  isAllDay: boolean;
  startDate: string | null; // "YYYY-MM-DD", inclusive range
  endDate: string | null;
  rrule: string | null;
  attendeeCount: number;
  myResponse: "accepted" | "declined" | "tentative" | null;
  replyAttendee: string | null;
  replyPartstat: string | null;
  canRsvp: boolean;
}

export interface RenderedBody {
  messageId: number;
  html: string;
  blockedImages: number;
  fromAddr: string | null;
  attachments: AttachmentMeta[];
  invite: InviteView | null;
}

export interface SearchHit {
  messageId: number;
  threadId: number | null;
  folderId: number;
  subject: string;
  fromName: string;
  fromAddr: string;
  date: number;
  snippet: string;
}

export interface Draft {
  id: number;
  accountId: string;
  replyToMessageId: number | null;
  mode: string;
  to: string;
  cc: string;
  bcc: string;
  subject: string;
  body: string;
}

/** A file staged on a draft. Metadata only — the bytes stay in the backend. */
export interface DraftAttachment {
  id: number;
  draftId: number;
  filename: string;
  mimeType: string;
  size: number;
}

export interface SkimError {
  code: string;
  message: string;
}

export type Theme = "light" | "dark" | "system";

export type SyncState = "syncing" | "idle" | "error" | "offline";
