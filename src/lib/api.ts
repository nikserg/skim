// Typed wrappers around the Tauri IPC surface — one function per command.
import { invoke } from "@tauri-apps/api/core";
import type {
  Account,
  Draft,
  Folder,
  RenderedBody,
  SearchHit,
  ServerPreset,
  ThreadDetail,
  ThreadRow,
} from "./types";

export interface AddAccountInput {
  email: string;
  displayName?: string | null;
  provider: string;
  imapHost: string;
  imapPort: number;
  smtpHost: string;
  smtpPort: number;
  smtpSecurity: string;
}

export const api = {
  // accounts
  autoconfigLookup: (email: string) =>
    invoke<ServerPreset | null>("autoconfig_lookup", { email }),
  googleOauthAvailable: () => invoke<boolean>("google_oauth_available"),
  listAccounts: () => invoke<Account[]>("list_accounts"),
  addAccount: (input: AddAccountInput, password: string) =>
    invoke<Account>("add_account", { input, password }),
  startGoogleOauth: () => invoke<Account>("start_google_oauth"),
  removeAccount: (accountId: string) => invoke<void>("remove_account", { accountId }),

  // mail
  listFolders: (accountId: string) => invoke<Folder[]>("list_folders", { accountId }),
  listThreads: (folderId: number, offset = 0, limit = 100) =>
    invoke<ThreadRow[]>("list_threads", { folderId, offset, limit }),
  getThread: (threadId: number) => invoke<ThreadDetail>("get_thread", { threadId }),
  getMessageBody: (messageId: number, showImages?: boolean) =>
    invoke<RenderedBody>("get_message_body", { messageId, showImages: showImages ?? null }),
  allowRemoteImages: (senderAddr: string) =>
    invoke<void>("allow_remote_images", { senderAddr }),
  markRead: (messageIds: number[], read: boolean) =>
    invoke<void>("mark_read", { messageIds, read }),
  setStarred: (messageIds: number[], starred: boolean) =>
    invoke<void>("set_starred", { messageIds, starred }),
  archiveMessages: (messageIds: number[]) => invoke<void>("archive_messages", { messageIds }),
  deleteMessages: (messageIds: number[]) => invoke<void>("delete_messages", { messageIds }),
  saveAttachment: (attachmentId: number) =>
    invoke<string | null>("save_attachment", { attachmentId }),
  openAttachment: (attachmentId: number) => invoke<void>("open_attachment", { attachmentId }),
  syncNow: (accountId?: string) => invoke<void>("sync_now", { accountId: accountId ?? null }),

  // search
  searchMessages: (query: string, limit = 20) =>
    invoke<SearchHit[]>("search_messages", { query, limit }),
  threadMessageIds: (threadId: number) =>
    invoke<number[]>("thread_message_ids", { threadId }),

  // compose
  createDraft: () => invoke<Draft>("create_draft"),
  getDraft: (draftId: number) => invoke<Draft>("get_draft", { draftId }),
  updateDraft: (draft: Draft) => invoke<void>("update_draft", { draft }),
  deleteDraft: (draftId: number) => invoke<void>("delete_draft", { draftId }),
  getReplyTemplate: (messageId: number, mode: "reply" | "reply_all" | "forward") =>
    invoke<Draft>("get_reply_template", { messageId, mode }),
  sendDraft: (draftId: number) => invoke<void>("send_draft", { draftId }),
  openComposeWindow: (draftId: number) => invoke<void>("open_compose_window", { draftId }),

  // settings
  getSettings: () => invoke<Record<string, string>>("get_settings"),
  setSetting: (key: string, value: string) => invoke<void>("set_setting", { key, value }),
};

export function errorMessage(e: unknown): string {
  if (e && typeof e === "object" && "message" in e) return String(e.message);
  return String(e);
}
