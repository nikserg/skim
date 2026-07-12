// Shared types mirroring the Rust IPC surface.

export interface Folder {
  id: number;
  role: string | null;
  displayName: string;
  unreadCount: number;
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
  labels: string[];
}

export interface Address {
  name: string | null;
  addr: string;
}

export interface MessageDetail {
  id: number;
  from: Address;
  to: Address[];
  cc: Address[];
  subject: string;
  date: number;
  isRead: boolean;
  isStarred: boolean;
  bodyText: string | null;
}

export type Theme = "light" | "dark" | "system";
