// Typed wrappers around the Tauri IPC surface — one function per command.
import { Channel, invoke } from "@tauri-apps/api/core";
import type {
  Account,
  Draft,
  DraftAttachment,
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
  microsoftOauthAvailable: () => invoke<boolean>("microsoft_oauth_available"),
  listAccounts: () => invoke<Account[]>("list_accounts"),
  addAccount: (input: AddAccountInput, password: string) =>
    invoke<Account>("add_account", { input, password }),
  startGoogleOauth: () => invoke<Account>("start_google_oauth"),
  startMicrosoftOauth: () => invoke<Account>("start_microsoft_oauth"),
  removeAccount: (accountId: string) => invoke<void>("remove_account", { accountId }),

  // mail
  listFolders: (accountId: string) => invoke<Folder[]>("list_folders", { accountId }),
  listThreads: (folderId: number, offset = 0, limit = 100) =>
    invoke<ThreadRow[]>("list_threads", { folderId, offset, limit }),
  listMessages: (folderId: number, offset = 0, limit = 100) =>
    invoke<ThreadRow[]>("list_messages", { folderId, offset, limit }),
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
  reportSpam: (messageIds: number[]) => invoke<void>("report_spam", { messageIds }),
  unsubscribe: (messageId: number) =>
    invoke<"submitted" | "opened">("unsubscribe", { messageId }),
  saveAttachment: (attachmentId: number) =>
    invoke<string | null>("save_attachment", { attachmentId }),
  openAttachment: (attachmentId: number) => invoke<void>("open_attachment", { attachmentId }),
  syncNow: (accountId?: string) => invoke<void>("sync_now", { accountId: accountId ?? null }),
  takePendingOpen: () =>
    invoke<{ folderId: number; threadId: number } | null>("take_pending_open"),
  rsvpInvite: (messageId: number, response: "accepted" | "declined" | "tentative") =>
    invoke<void>("rsvp_invite", { messageId, response }),
  openInviteIcs: (messageId: number) => invoke<void>("open_invite_ics", { messageId }),

  // search
  searchMessages: (query: string, limit = 20) =>
    invoke<SearchHit[]>("search_messages", { query, limit }),
  threadMessageIds: (threadId: number) =>
    invoke<number[]>("thread_message_ids", { threadId }),

  // compose
  createDraft: () => invoke<Draft>("create_draft"),
  getDraft: (draftId: number) => invoke<Draft>("get_draft", { draftId }),
  updateDraft: (draft: Draft) => invoke<void>("update_draft", { draft }),
  saveServerDraft: (draft: Draft) => invoke<void>("save_server_draft", { draft }),
  editDraft: (messageId: number) => invoke<Draft>("edit_draft", { messageId }),
  deleteDraft: (draftId: number) => invoke<void>("delete_draft", { draftId }),
  getReplyTemplate: (messageId: number, mode: "reply" | "reply_all" | "forward") =>
    invoke<Draft>("get_reply_template", { messageId, mode }),
  sendDraft: (draftId: number) => invoke<void>("send_draft", { draftId }),
  addDraftAttachment: (
    draftId: number,
    filename: string,
    mimeType: string,
    data: number[],
  ) =>
    invoke<DraftAttachment>("add_draft_attachment", { draftId, filename, mimeType, data }),
  listDraftAttachments: (draftId: number) =>
    invoke<DraftAttachment[]>("list_draft_attachments", { draftId }),
  removeDraftAttachment: (attachmentId: number) =>
    invoke<void>("remove_draft_attachment", { attachmentId }),
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

/** The stable `code` a backend `SkimError` carries, for branching on error kind. */
export function errorCode(e: unknown): string {
  if (e && typeof e === "object" && "code" in e) return String(e.code);
  return "";
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
  | { type: "toolCall"; id: string; kind: string; arg: string }
  | { type: "toolDone"; id: string; count: number | null }
  | { type: "done"; citations: Citation[] }
  | { type: "error"; code: string; message: string };

export interface AiHandlers {
  delta: (text: string) => void;
  done: (citations: Citation[]) => void;
  error: (code: string, message: string) => void;
  progress?: (current: number, total: number) => void;
  toolCall?: (id: string, kind: string, arg: string) => void;
  toolDone?: (id: string, count: number | null) => void;
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
    switch (event.type) {
      case "delta":
        on.delta(event.text);
        break;
      case "progress":
        on.progress?.(event.current, event.total);
        break;
      case "toolCall":
        on.toolCall?.(event.id, event.kind, event.arg);
        break;
      case "toolDone":
        on.toolDone?.(event.id, event.count);
        break;
      case "done":
        on.done(event.citations);
        break;
      case "error":
        on.error(event.code, event.message);
        break;
    }
  };
  void invoke(command, { ...args, requestId, channel }).catch((e) => {
    if (!cancelled) on.error("ai", errorMessage(e));
  });
  return () => {
    cancelled = true;
    void invoke("ai_cancel", { requestId }).catch(() => {});
  };
}
