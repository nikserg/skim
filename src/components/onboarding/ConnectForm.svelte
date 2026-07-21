<script lang="ts">
  // Email-first connect form, shared by onboarding (first mailbox) and
  // settings (adding another). Emits the connected account via `onconnected`.
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { api, errorMessage, errorCode, type AddAccountInput, type OauthAvailability } from "../../lib/api";
  import { t } from "../../lib/i18n/index.svelte";
  import type { Account, ServerPreset } from "../../lib/types";

  let { onconnected }: { onconnected: (account: Account) => void } = $props();

  let google = $state<OauthAvailability>({ available: false, verified: false });
  let microsoft = $state<OauthAvailability>({ available: false, verified: false });
  let email = $state("");
  let password = $state("");
  let preset = $state<ServerPreset | null>(null);
  // The connect screen is email-first: the method zone stays hidden until the
  // user has entered an address we can detect a provider for.
  let touched = $state(false);
  let emailInput = $state<HTMLInputElement | null>(null);
  // Secondary reveals: Google one-click (limited) under the Gmail app-password
  // block, and a password fallback under the Microsoft button.
  let showGoogleOauth = $state(false);
  let showOutlookPassword = $state(false);
  let showAdvanced = $state(false);
  let imapHost = $state("");
  let imapPort = $state(993);
  let smtpHost = $state("");
  let smtpPort = $state(587);
  let smtpSecurity = $state("starttls");
  let busy: "none" | "google" | "microsoft" | "password" = $state("none");
  let error = $state("");
  // Set when Microsoft sign-in works but the Outlook mailbox has IMAP disabled;
  // shows a fix-it prompt with a link to the setting and a retry.
  let imapSetupNeeded = $state(false);

  // Deep link to Outlook.com's "Forwarding and IMAP" settings pane.
  const OUTLOOK_IMAP_SETTINGS_URL =
    "https://outlook.live.com/mail/0/options/mail/forwarding";

  $effect(() => {
    const unavailable = { available: false, verified: false };
    void api.googleOauthAvailable().then((v) => (google = v)).catch(() => (google = unavailable));
    void api
      .microsoftOauthAvailable()
      .then((v) => (microsoft = v))
      .catch(() => (microsoft = unavailable));
    queueMicrotask(() => emailInput?.focus());
  });

  /** A connect attempt failed — show the friendly text for known codes. */
  function showError(e: unknown) {
    error = errorCode(e) === "account_exists" ? t("accounts.exists") : errorMessage(e);
  }

  async function onEmailChange() {
    touched = email.includes("@");
    preset = await api.autoconfigLookup(email).catch(() => null);
    if (preset) {
      imapHost = preset.imapHost;
      imapPort = preset.imapPort;
      smtpHost = preset.smtpHost;
      smtpPort = preset.smtpPort;
      smtpSecurity = preset.smtpSecurity;
    } else if (email.includes("@")) {
      const domain = email.split("@")[1] ?? "";
      if (!imapHost) imapHost = `imap.${domain}`;
      if (!smtpHost) smtpHost = `smtp.${domain}`;
      showAdvanced = true;
    }
  }

  // The detected provider drives which sign-in method the screen leads with.
  const provider = $derived(preset?.provider ?? null);

  // Where each provider lets users create an app password.
  const APP_PW_LINKS: Record<string, string> = {
    gmail: "https://myaccount.google.com/apppasswords",
    yahoo: "https://login.yahoo.com/account/security",
    icloud: "https://account.apple.com/account/manage",
  };
  function openAppPasswords() {
    const url = APP_PW_LINKS[provider ?? ""];
    if (url) void openUrl(url);
  }

  async function connectGoogle() {
    busy = "google";
    error = "";
    imapSetupNeeded = false;
    try {
      const account = await api.startGoogleOauth();
      onconnected(account);
    } catch (e) {
      showError(e);
    } finally {
      busy = "none";
    }
  }

  async function connectMicrosoft() {
    busy = "microsoft";
    error = "";
    imapSetupNeeded = false;
    try {
      const account = await api.startMicrosoftOauth();
      onconnected(account);
    } catch (e) {
      // Outlook mailbox with IMAP switched off: guide the user to the setting
      // instead of showing a raw error, and let them retry in place.
      if (errorCode(e) === "imap_disabled") {
        imapSetupNeeded = true;
      } else {
        showError(e);
      }
    } finally {
      busy = "none";
    }
  }

  function openOutlookSettings() {
    void openUrl(OUTLOOK_IMAP_SETTINGS_URL);
  }

  async function connectPassword(ev: SubmitEvent) {
    ev.preventDefault();
    if (!email.trim()) return;
    // Enter from the email field (before a password exists) should reveal the
    // provider's method zone — the same thing blurring the field does — instead
    // of silently doing nothing.
    if (!password) {
      await onEmailChange();
      return;
    }
    busy = "password";
    error = "";
    imapSetupNeeded = false;
    try {
      const input: AddAccountInput = {
        email: email.trim(),
        provider: preset?.provider ?? "custom",
        imapHost: imapHost.trim(),
        imapPort,
        smtpHost: smtpHost.trim(),
        smtpPort,
        smtpSecurity,
      };
      const account = await api.addAccount(input, password);
      onconnected(account);
    } catch (e) {
      showError(e);
    } finally {
      busy = "none";
    }
  }

  const providerLabel = $derived(
    preset?.provider === "gmail"
      ? "Gmail"
      : preset?.provider === "outlook"
        ? "Outlook"
        : preset?.provider === "yahoo"
          ? "Yahoo"
          : preset?.provider === "icloud"
            ? "iCloud"
            : "",
  );
</script>

{#snippet passwordFields(showHint: boolean)}
  <label>
    <span class="microlabel">{t("onb.password")}</span>
    <input type="password" bind:value={password} required autocomplete="off" />
  </label>

  {#if showHint && preset?.needsAppPassword}
    <div class="hint">{t("onb.app_password_hint", { provider: providerLabel })}</div>
  {/if}

  <button type="button" class="linkish advanced-toggle" onclick={() => (showAdvanced = !showAdvanced)}>
    {t("onb.advanced")} {showAdvanced ? "▴" : "▾"}
  </button>
  {#if showAdvanced}
    <div class="grid">
      <label>
        <span class="microlabel">{t("onb.imap_host")}</span>
        <input bind:value={imapHost} spellcheck="false" />
      </label>
      <label class="narrow">
        <span class="microlabel">{t("onb.port")}</span>
        <input type="number" bind:value={imapPort} />
      </label>
      <label>
        <span class="microlabel">{t("onb.smtp_host")}</span>
        <input bind:value={smtpHost} spellcheck="false" />
      </label>
      <label class="narrow">
        <span class="microlabel">{t("onb.port")}</span>
        <input type="number" bind:value={smtpPort} />
      </label>
    </div>
  {/if}

  {#if error}
    <div class="error">{error}</div>
  {/if}

  <button class="primary" type="submit" disabled={busy !== "none" || !email || !password}>
    {busy === "password" ? t("onb.connecting") : t("onb.connect_btn")}
  </button>
{/snippet}

{#snippet imapSetupPanel()}
  <div class="imap-setup">
    <div class="imap-setup-title">{t("onb.imap_off_title")}</div>
    <p>{t("onb.imap_off_body")}</p>
    <div class="imap-setup-actions">
      <button type="button" class="primary" onclick={openOutlookSettings}>
        {t("onb.open_outlook_settings")} ↗
      </button>
      <button type="button" class="linkish" onclick={connectMicrosoft} disabled={busy !== "none"}>
        {busy === "microsoft" ? t("onb.connecting") : t("onb.retry")}
      </button>
    </div>
  </div>
{/snippet}

<form onsubmit={connectPassword}>
  <label>
    <span class="microlabel">{t("onb.email")}</span>
    <input
      type="email"
      bind:this={emailInput}
      bind:value={email}
      onblur={onEmailChange}
      required
      spellcheck="false"
      autocomplete="off"
    />
  </label>

  {#if !touched}
    <p class="email-prompt">{t("onb.email_prompt")}</p>
  {:else if provider === "gmail"}
    <!-- Gmail: app password leads (reliable). One-click Google is offered
         only as a limited secondary — our restricted-scope app is
         unverified, so Google signs users out ~weekly. -->
    <div class="app-pw">
      <div class="app-pw-title">{t("onb.gmail_pw_title")}</div>
      <ol class="app-pw-steps">
        <li>{t("onb.gmail_pw_step_2fa")}</li>
        <li>{t("onb.gmail_pw_step_create")}</li>
        <li>{t("onb.gmail_pw_step_paste")}</li>
      </ol>
      <button type="button" class="linkish" onclick={openAppPasswords}>
        {t("onb.open_google_app_passwords")} ↗
      </button>
    </div>
    {@render passwordFields(false)}

    {#if google.available}
      <div class="divider"><span class="microlabel">{t("onb.or")}</span></div>
      {#if showGoogleOauth}
        <button class="primary google" onclick={connectGoogle} disabled={busy !== "none"}>
          {#if busy === "google"}
            {t("onb.waiting_google")}
          {:else}
            <span class="g-badge">G</span>
            {t("onb.continue_google")} →
          {/if}
        </button>
        {#if !google.verified}
          <div class="caveat">{t("onb.google_unverified_note")}</div>
        {/if}
      {:else}
        <button type="button" class="linkish center" onclick={() => (showGoogleOauth = true)}>
          {t("onb.prefer_one_click_google")}
        </button>
      {/if}
    {/if}
  {:else if provider === "outlook"}
    <!-- Outlook/O365: OAuth is the only reliable path — Microsoft is
         retiring Basic Auth (app passwords) for Exchange Online. -->
    {#if microsoft.available}
      <button class="primary microsoft" onclick={connectMicrosoft} disabled={busy !== "none"}>
        {#if busy === "microsoft"}
          {t("onb.waiting_microsoft")}
        {:else}
          <span class="ms-badge" aria-hidden="true">
            <span></span><span></span><span></span><span></span>
          </span>
          {t("onb.continue_microsoft")} →
        {/if}
      </button>
      {#if !microsoft.verified}
        <div class="caveat">{t("onb.ms_unverified_note")}</div>
      {/if}
      <div class="oauth-note microlabel">🔒 {t("onb.oauth_note")}</div>

      {#if imapSetupNeeded}
        {@render imapSetupPanel()}
      {/if}

      {#if showOutlookPassword}
        <div class="divider"><span class="microlabel">{t("onb.or")}</span></div>
        {@render passwordFields(true)}
      {:else}
        <button type="button" class="linkish center" onclick={() => (showOutlookPassword = true)}>
          {t("onb.use_password_instead")}
        </button>
      {/if}
    {:else}
      {@render passwordFields(true)}
    {/if}
  {:else if provider === "yahoo" || provider === "icloud"}
    {#if APP_PW_LINKS[provider]}
      <button type="button" class="linkish" onclick={openAppPasswords}>
        {t("onb.open_app_passwords")} ↗
      </button>
    {/if}
    {@render passwordFields(true)}
  {:else}
    {@render passwordFields(false)}
  {/if}
</form>

<style>
  .primary {
    margin-top: 26px;
    width: 100%;
    padding: 12px 18px;
    border-radius: var(--radius-m);
    background: var(--text);
    color: var(--bg);
    font-weight: 700;
    font-size: 14px;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 10px;
  }
  .primary:hover:not(:disabled) {
    opacity: 0.88;
  }
  .primary:disabled {
    opacity: 0.5;
    cursor: default;
  }

  .g-badge {
    width: 20px;
    height: 20px;
    border-radius: 50%;
    background: var(--bg);
    color: var(--text);
    display: grid;
    place-items: center;
    font-size: 11px;
    font-weight: 800;
  }
  .primary.microsoft {
    margin-top: 12px;
  }
  .ms-badge {
    width: 16px;
    height: 16px;
    display: grid;
    grid-template-columns: 1fr 1fr;
    grid-template-rows: 1fr 1fr;
    gap: 2px;
    flex-shrink: 0;
  }
  .ms-badge span {
    display: block;
  }
  .ms-badge span:nth-child(1) {
    background: #f25022;
  }
  .ms-badge span:nth-child(2) {
    background: #7fba00;
  }
  .ms-badge span:nth-child(3) {
    background: #00a4ef;
  }
  .ms-badge span:nth-child(4) {
    background: #ffb900;
  }
  .oauth-note {
    text-align: center;
    margin-top: 10px;
  }

  .linkish {
    color: var(--text);
    text-decoration: underline;
    text-underline-offset: 3px;
    font-size: 13px;
  }
  .linkish.center {
    align-self: center;
  }

  /* Email-first connect: prompt shown before a provider is detected. */
  .email-prompt {
    font-size: 12.5px;
    color: var(--text-faint);
    line-height: 1.5;
  }

  /* App-password guidance (Gmail leads with this). */
  .app-pw {
    display: flex;
    flex-direction: column;
    gap: 8px;
    background: var(--hover);
    border-radius: var(--radius-s);
    padding: 12px 14px;
  }
  .app-pw-title {
    font-weight: 600;
    font-size: 13.5px;
  }
  .app-pw-steps {
    margin: 0;
    padding-left: 18px;
    font-size: 12.5px;
    color: var(--text-dim);
    line-height: 1.7;
  }
  .app-pw .linkish {
    align-self: flex-start;
    font-size: 12.5px;
  }

  /* Honest, self-clearing caveat under a limited/unverified OAuth button. */
  .caveat {
    font-size: 12.5px;
    color: var(--text-dim);
    line-height: 1.5;
    padding-left: 2px;
  }

  .divider {
    display: flex;
    align-items: center;
    gap: 12px;
    margin: 26px 0 18px;
  }
  .divider::before,
  .divider::after {
    content: "";
    flex: 1;
    height: 1px;
    background: var(--hairline);
  }

  form {
    display: flex;
    flex-direction: column;
    gap: 14px;
    /* The form sits directly under the host's subtitle (no OAuth buttons
       above), so it needs its own breathing room before the email field. */
    margin-top: 24px;
  }
  label {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  input {
    padding: 10px 12px;
    border: 1px solid var(--hairline-strong);
    border-radius: var(--radius-s);
    background: var(--surface);
    font-size: 14px;
    user-select: text;
  }
  input:focus {
    border-color: var(--text-faint);
  }

  .hint {
    font-size: 12.5px;
    color: var(--text-dim);
    background: var(--hover);
    border-radius: var(--radius-s);
    padding: 10px 12px;
    line-height: 1.5;
  }

  .imap-setup {
    display: flex;
    flex-direction: column;
    gap: 8px;
    background: var(--hover);
    border: 1px solid var(--hairline-strong);
    border-radius: var(--radius-s);
    padding: 12px 14px;
  }
  .imap-setup-title {
    font-weight: 600;
    font-size: 14px;
  }
  .imap-setup p {
    margin: 0;
    font-size: 12.5px;
    color: var(--text-dim);
    line-height: 1.5;
  }
  .imap-setup-actions {
    display: flex;
    align-items: center;
    gap: 14px;
    margin-top: 4px;
  }
  .imap-setup-actions .primary {
    width: auto;
    padding: 8px 14px;
    font-size: 13px;
  }
  .advanced-toggle {
    align-self: flex-start;
    text-decoration: none;
    color: var(--text-dim);
  }
  .grid {
    display: grid;
    grid-template-columns: 1fr 110px;
    gap: 10px 12px;
  }

  .error {
    color: var(--danger);
    font-size: 13px;
    line-height: 1.45;
  }
</style>
