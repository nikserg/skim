// Typed wrappers around the Tauri IPC surface — one function per command.
import { invoke } from "@tauri-apps/api/core";
import type { Account, Folder, ServerPreset, ThreadRow } from "./types";

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
  syncNow: (accountId?: string) => invoke<void>("sync_now", { accountId: accountId ?? null }),

  // settings
  getSettings: () => invoke<Record<string, string>>("get_settings"),
  setSetting: (key: string, value: string) => invoke<void>("set_setting", { key, value }),
};

export function errorMessage(e: unknown): string {
  if (e && typeof e === "object" && "message" in e) return String(e.message);
  return String(e);
}
