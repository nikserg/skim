// Typed wrappers around the Tauri IPC surface — one function per command.
import { Channel, invoke } from "@tauri-apps/api/core";
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
  rsvpInvite: (messageId: number, response: "accepted" | "declined" | "tentative") =>
    invoke<void>("rsvp_invite", { messageId, response }),

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
  suggestAddresses: (query: string) =>
    invoke<AddressSuggestion[]>("suggest_addresses", { query }),

  // settings
  getSettings: () => invoke<Record<string, string>>("get_settings"),
  setSetting: (key: string, value: string) => invoke<void>("set_setting", { key, value }),
};

export interface AddressSuggestion {
  name: string | null;
  addr: string;
}

export function errorMessage(e: unknown): string {
  if (e && typeof e === "object" && "message" in e) return String(e.message);
  return String(e);
}

// ---- AI (streaming over IPC channels) ----

export interface Citation {
  index: number;
  messageId: number;
  threadId: number | null;
  folderId: number;
  subject: string;
  from: string;
}

export type AiEvent =
  | { type: "delta"; text: string }
  | { type: "progress"; current: number; total: number }
  | { type: "done"; citations: Citation[] }
  | { type: "error"; code: string; message: string };

export interface AiHandlers {
  delta: (text: string) => void;
  done: (citations: Citation[]) => void;
  error: (code: string, message: string) => void;
  progress?: (current: number, total: number) => void;
}

export type AiProvider = "anthropic" | "openrouter";

export interface AiKeyStatus {
  provider: AiProvider;
  anthropic: boolean;
  openrouter: boolean;
}

export const aiApi = {
  setKey: (provider: AiProvider, key: string) => invoke<void>("ai_set_key", { provider, key }),
  keyStatus: () => invoke<AiKeyStatus>("ai_key_status"),
  clearKey: (provider: AiProvider) => invoke<void>("ai_clear_key", { provider }),
};

/** Start a streaming AI request. Returns a cancel function. */
export function aiStream(
  command:
    | "ai_summarize"
    | "ai_draft"
    | "ai_adjust_draft"
    | "ai_compose"
    | "ai_ask"
    | "ai_chat"
    | "ai_analyze_style"
    | "ai_recap",
  args: Record<string, unknown>,
  on: AiHandlers,
): () => void {
  const requestId = crypto.randomUUID();
  let cancelled = false;
  const channel = new Channel<AiEvent>();
  channel.onmessage = (event) => {
    if (cancelled) return;
    if (event.type === "delta") on.delta(event.text);
    else if (event.type === "progress") on.progress?.(event.current, event.total);
    else if (event.type === "done") on.done(event.citations);
    else on.error(event.code, event.message);
  };
  void invoke(command, { ...args, requestId, channel }).catch((e) => {
    if (!cancelled) on.error("ai", errorMessage(e));
  });
  return () => {
    cancelled = true;
    void invoke("ai_cancel", { requestId }).catch(() => {});
  };
}
