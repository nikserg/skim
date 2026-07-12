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

export interface SkimError {
  code: string;
  message: string;
}

export type Theme = "light" | "dark" | "system";

export type SyncState = "syncing" | "idle" | "error" | "offline";
