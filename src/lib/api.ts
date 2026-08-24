// Typed wrappers around the Tauri IPC surface — one function per command.
import { Channel, invoke } from "@tauri-apps/api/core";
import { t } from "./i18n/index.svelte";
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

/** Whether a provider's one-click OAuth is offered, and whether its app has
 *  cleared provider-side verification (`verified` matters only when `available`). */
export interface OauthAvailability {
  available: boolean;
  verified: boolean;
}

export const api = {
  // accounts
  autoconfigLookup: (email: string) =>
    invoke<ServerPreset | null>("autoconfig_lookup", { email }),
  googleOauthAvailable: () => invoke<OauthAvailability>("google_oauth_available"),
  microsoftOauthAvailable: () => invoke<OauthAvailability>("microsoft_oauth_available"),
  listAccounts: () => invoke<Account[]>("list_accounts"),
  addAccount: (input: AddAccountInput, password: string) =>
    invoke<Account>("add_account", { input, password }),
  startGoogleOauth: () => invoke<Account>("start_google_oauth"),
  startMicrosoftOauth: () => invoke<Account>("start_microsoft_oauth"),
  removeAccount: (accountId: string) => invoke<void>("remove_account", { accountId }),
  inboxUnreadCounts: () => invoke<Record<string, number>>("inbox_unread_counts"),

  // mail
  listFolders: (accountId: string) => invoke<Folder[]>("list_folders", { accountId }),
  folderAccountId: (folderId: number) => invoke<string>("folder_account_id", { folderId }),
  listThreads: (folderId: number, offset = 0, limit = 100) =>
    invoke<ThreadRow[]>("list_threads", { folderId, offset, limit }),
  listMessages: (folderId: number, offset = 0, limit = 100) =>
    invoke<ThreadRow[]>("list_messages", { folderId, offset, limit }),
  listUnifiedFolders: () => invoke<Folder[]>("list_unified_folders"),
  listUnifiedThreads: (role: string | null, label: string | null, offset = 0, limit = 100) =>
    invoke<ThreadRow[]>("list_unified_threads", { role, label, offset, limit }),
  listUnifiedMessages: (role: string | null, label: string | null, offset = 0, limit = 100) =>
    invoke<ThreadRow[]>("list_unified_messages", { role, label, offset, limit }),
  folderRef: (folderId: number) =>
    invoke<{ role: string | null; displayName: string }>("folder_ref", { folderId }),
  getThread: (threadId: number) => invoke<ThreadDetail>("get_thread", { threadId }),
  /** `translated: false` asks for the original even when a translation is cached. */
  getMessageBody: (messageId: number, showImages?: boolean, translated?: boolean) =>
    invoke<RenderedBody>("get_message_body", {
      messageId,
      showImages: showImages ?? null,
      translated: translated ?? null,
    }),
  allowRemoteImages: (senderAddr: string) =>
    invoke<void>("allow_remote_images", { senderAddr }),
  markRead: (messageIds: number[], read: boolean) =>
    invoke<void>("mark_read", { messageIds, read }),
  setStarred: (messageIds: number[], starred: boolean) =>
    invoke<void>("set_starred", { messageIds, starred }),
  archiveMessages: (messageIds: number[]) => invoke<void>("archive_messages", { messageIds }),
  deleteMessages: (messageIds: number[]) => invoke<void>("delete_messages", { messageIds }),
  reportSpam: (messageIds: number[]) => invoke<void>("report_spam", { messageIds }),
  /** Folders these messages can be filed into (their own account's, minus the
   *  ones they already sit in and the roles that aren't real destinations). */
  moveTargets: (messageIds: number[]) => invoke<Folder[]>("move_targets", { messageIds }),
  /** File messages into `destFolderId`, or into a folder named `newFolderName`
   *  that is created as part of the move. Exactly one of the two is given. */
  moveMessages: (messageIds: number[], destFolderId: number | null, newFolderName?: string) =>
    invoke<void>("move_messages", {
      messageIds,
      destFolderId,
      newFolderName: newFolderName ?? null,
    }),
  /** How much mail a folder holds — deleting is only offered for an empty one. */
  folderMessageCount: (folderId: number) => invoke<number>("folder_message_count", { folderId }),
  renameFolder: (folderId: number, newName: string) =>
    invoke<void>("rename_folder", { folderId, newName }),
  deleteFolder: (folderId: number) => invoke<void>("delete_folder", { folderId }),
  unsubscribe: (messageId: number) =>
    invoke<"submitted" | "opened">("unsubscribe", { messageId }),
  saveAttachment: (attachmentId: number) =>
    invoke<string | null>("save_attachment", { attachmentId }),
  openAttachment: (attachmentId: number) => invoke<void>("open_attachment", { attachmentId }),
  syncNow: (accountId?: string) => invoke<void>("sync_now", { accountId: accountId ?? null }),
  takePendingOpen: () =>
    invoke<{ folderId: number; threadId: number; messageId: number } | null>("take_pending_open"),
  rsvpInvite: (messageId: number, response: "accepted" | "declined" | "tentative") =>
    invoke<void>("rsvp_invite", { messageId, response }),
  openInviteIcs: (messageId: number) => invoke<void>("open_invite_ics", { messageId }),

  // search
  searchMessages: (query: string, limit = 20, accountId?: string) =>
    invoke<SearchHit[]>("search_messages", { query, limit, accountId: accountId ?? null }),
  threadMessageIds: (threadId: number) =>
    invoke<number[]>("thread_message_ids", { threadId }),
  /** Same, for many threads at once — one round trip for a bulk selection. */
  threadMessageIdsBulk: (threadIds: number[]) =>
    invoke<number[]>("thread_message_ids_bulk", { threadIds }),

  // compose
  createDraft: (accountId?: string) =>
    invoke<Draft>("create_draft", { accountId: accountId ?? null }),
  getDraft: (draftId: number) => invoke<Draft>("get_draft", { draftId }),
  updateDraft: (draft: Draft) => invoke<void>("update_draft", { draft }),
  setDraftAccount: (draftId: number, accountId: string) =>
    invoke<void>("set_draft_account", { draftId, accountId }),
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
  /** Open a cited email in the main window (from a popped-out AI chat, which
   *  has no mail UI of its own). Raises the main window too. */
  openThreadInMain: (folderId: number, threadId: number | null, messageId: number) =>
    invoke<void>("open_thread_in_main", { folderId, threadId, messageId }),
  suggestAddresses: (query: string) =>
    invoke<AddressSuggestion[]>("suggest_addresses", { query }),

  // settings
  getSettings: () => invoke<Record<string, string>>("get_settings"),
  setSetting: (key: string, value: string) => invoke<void>("set_setting", { key, value }),
  logFrontendError: (message: string) => invoke<void>("log_frontend_error", { message }),
  /** Park the sync engines and checkpoint the database before the updater
   *  hands over to the installer — which kills this process outright. */
  prepareUpdate: () => invoke<void>("prepare_update"),
};

/** Record a frontend failure in `skim-frontend.log`, next to the panic log.
 *  Best-effort by definition: the reporting path must never become the thing
 *  that throws. `where` names the site, since a minified stack rarely does. */
export function reportError(where: string, e: unknown): void {
  if (!("__TAURI_INTERNALS__" in window)) return;
  const detail = e instanceof Error && e.stack ? e.stack : errorMessage(e);
  void api.logFrontendError(`${where}: ${detail}`).catch(() => {});
}

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
  | { type: "reasoning" }
  | { type: "progress"; current: number; total: number }
  | { type: "toolCall"; id: string; kind: string; arg: string }
  | { type: "toolDone"; id: string; count: number | null }
  | { type: "done"; citations: Citation[] }
  | { type: "error"; code: string; message: string };

export interface AiHandlers {
  delta: (text: string) => void;
  done: (citations: Citation[]) => void;
  error: (code: string, message: string) => void;
  /** The model started reasoning: nothing to render, but it is working and
   *  not still loading. Sent once per round, by every provider, and required
   *  on purpose: a surface that shows a waiting state has to answer for it. */
  reasoning: () => void;
  progress?: (current: number, total: number) => void;
  toolCall?: (id: string, kind: string, arg: string) => void;
  toolDone?: (id: string, count: number | null) => void;
}

export type AiProvider = "anthropic" | "openrouter" | "custom";

export interface AiKeyStatus {
  provider: AiProvider;
  anthropic: boolean;
  openrouter: boolean;
  /** The custom endpoint counts as configured once a base URL is set. */
  custom: boolean;
}

/** A pickable model: an OpenRouter catalog entry, or an Ollama server's
 *  installed model (where both fields carry the tag). */
export interface AiModel {
  id: string;
  name: string;
}

export const aiApi = {
  /** Open a window onto a chat the main window owns. The window addresses the
   *  chat by this id; `title` only names the window. */
  openWindow: (sessionId: number, title: string) =>
    invoke<void>("open_ai_window", { sessionId, title }),
  setKey: (provider: AiProvider, key: string) => invoke<void>("ai_set_key", { provider, key }),
  /** Configure the OpenAI-compatible endpoint; an empty key is fine. */
  setCustom: (baseUrl: string, key: string, model: string) =>
    invoke<void>("ai_set_custom", { baseUrl, key, model }),
  keyStatus: () => invoke<AiKeyStatus>("ai_key_status"),
  clearKey: (provider: AiProvider) => invoke<void>("ai_clear_key", { provider }),
  orModels: () => invoke<AiModel[]>("openrouter_models"),
  /** Installed models on an Ollama server, if the given (custom-provider) base
   *  URL turns out to be one — any error means "not Ollama", handled by the caller. */
  ollamaModels: (url: string) => invoke<AiModel[]>("ollama_models", { url }),
};

/** What to show the user for a failed AI request. Codes the backend gives a
 *  specific meaning get a specific message; anything else falls through to what
 *  the provider said. */
export function aiErrorText(code: string, message: string): string {
  if (code === "ai_key") return t("ai.needs_key");
  if (code === "ai_truncated") return t("ai.truncated");
  if (code === "ai_no_answer") return t("ai.no_answer");
  return message || t("ai.no_answer");
}

// Requests still running in this window. Closing the window (a popped-out chat,
// a compose window) tears down the webview without unmounting anything, so the
// backend would keep spending tokens on an answer nobody will ever read.
const inFlight = new Set<string>();
if (typeof window !== "undefined") {
  window.addEventListener("beforeunload", () => {
    for (const requestId of inFlight) void invoke("ai_cancel", { requestId }).catch(() => {});
  });
}

/** Start a streaming AI request. Returns a cancel function. */
export function aiStream(
  command:
    | "ai_compose"
    | "ai_ask"
    | "ai_chat"
    | "ai_analyze_style"
    | "ai_recap"
    | "ai_translate",
  args: Record<string, unknown>,
  on: AiHandlers,
): () => void {
  const requestId = crypto.randomUUID();
  let cancelled = false;
  inFlight.add(requestId);
  const channel = new Channel<AiEvent>();
  channel.onmessage = (event) => {
    if (cancelled) return;
    if (event.type === "done" || event.type === "error") inFlight.delete(requestId);
    switch (event.type) {
      case "delta":
        on.delta(event.text);
        break;
      case "reasoning":
        on.reasoning();
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
    inFlight.delete(requestId);
    void invoke("ai_cancel", { requestId }).catch(() => {});
  };
}
