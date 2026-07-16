<script lang="ts">
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { aiApi, api, errorMessage, type AddAccountInput, type AiProvider } from "../../lib/api";
  import { getLocale, LOCALES, setLocale, t, type Locale } from "../../lib/i18n/index.svelte";
  import { mail } from "../../lib/stores/mail.svelte";
  import type { Account, ServerPreset } from "../../lib/types";

  let { oncomplete }: { oncomplete: (account: Account) => void } = $props();

  let step: "welcome" | "connect" | "ai" = $state("welcome");
  let locale: Locale = $state(getLocale());
  let connectedAccount = $state<Account | null>(null);

  // AI step
  let aiKey = $state("");
  let aiProvider = $state<AiProvider>("anthropic");
  let aiBusy = $state(false);
  let aiVerified = $state(false);
  let aiError = $state("");

  function chooseAiProvider(p: AiProvider) {
    aiProvider = p;
    aiError = "";
  }

  async function accountConnected(account: Account) {
    connectedAccount = account;
    // The AI key lives in Credential Manager, not per-account — if one was set
    // during an earlier onboarding it's still there. Don't ask for it again.
    const status = await aiApi.keyStatus().catch(() => null);
    if (status && (status.anthropic || status.openrouter)) {
      finish();
      return;
    }
    step = "ai";
  }

  async function enableAi() {
    aiBusy = true;
    aiError = "";
    try {
      await aiApi.setKey(aiProvider, aiKey);
      aiVerified = true;
      setTimeout(() => finish(), 400);
    } catch (e) {
      aiError = errorMessage(e);
    } finally {
      aiBusy = false;
    }
  }

  function finish() {
    if (connectedAccount) oncomplete(connectedAccount);
  }

  // connect form
  let oauthAvailable = $state(false);
  let msOauthAvailable = $state(false);
  let email = $state("");
  let password = $state("");
  let preset = $state<ServerPreset | null>(null);
  let showAdvanced = $state(false);
  let imapHost = $state("");
  let imapPort = $state(993);
  let smtpHost = $state("");
  let smtpPort = $state(587);
  let smtpSecurity = $state("starttls");
  let busy: "none" | "google" | "microsoft" | "password" = $state("none");
  let error = $state("");

  async function chooseLocale(code: Locale) {
    locale = code;
    await setLocale(code);
    void api.setSetting("locale", code).catch(() => {});
  }

  async function toConnect() {
    step = "connect";
    oauthAvailable = await api.googleOauthAvailable().catch(() => false);
    msOauthAvailable = await api.microsoftOauthAvailable().catch(() => false);
  }

  async function onEmailChange() {
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

  async function connectGoogle() {
    busy = "google";
    error = "";
    try {
      const account = await api.startGoogleOauth();
      await accountConnected(account);
    } catch (e) {
      error = errorMessage(e);
    } finally {
      busy = "none";
    }
  }

  async function connectMicrosoft() {
    busy = "microsoft";
    error = "";
    try {
      const account = await api.startMicrosoftOauth();
      await accountConnected(account);
    } catch (e) {
      error = errorMessage(e);
    } finally {
      busy = "none";
    }
  }

  async function connectPassword(ev: SubmitEvent) {
    ev.preventDefault();
    busy = "password";
    error = "";
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
      await accountConnected(account);
    } catch (e) {
      error = errorMessage(e);
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

  void mail; // keep import for future use
</script>

<div class="onboarding" data-tauri-drag-region>
  {#if step === "welcome"}
    <div class="card welcome">
      <div class="wordmark">
        <svg class="mark" width="22" height="22" viewBox="0 0 96 96" aria-hidden="true">
          <rect width="96" height="96" rx="22" fill="#eadfc2" />
          <rect x="25" y="39" width="46" height="8.5" rx="4.25" fill="#0d0d10" />
          <rect x="25" y="55" width="29" height="8.5" rx="4.25" fill="#0d0d10" fill-opacity="0.55" />
        </svg>
        Skim
      </div>
      <h1>
        {t("onb.tagline_1")}<br />
        <em>{t("onb.tagline_2")}</em>
      </h1>
      <p class="subtitle">{t("onb.subtitle")}</p>
      <button class="primary" onclick={toConnect}>{t("onb.get_started")}</button>

      <div class="langs">
        <div class="microlabel">{t("onb.language")}</div>
        <div class="lang-row">
          {#each LOCALES as l (l.code)}
            <button
              class="lang"
              class:active={locale === l.code}
              onclick={() => chooseLocale(l.code)}
            >
              {l.label}
            </button>
          {/each}
        </div>
      </div>

      <div class="footer microlabel">{t("onb.footer")} · v0.1</div>
    </div>
  {:else if step === "ai"}
    <div class="card connect">
      <div class="microlabel step-label">
        {t("onb.step", { n: 2, total: 2 })} · {t("onb.ai_optional")}
      </div>
      <h2 class="ai-title">✦ {t("onb.ai_title")}</h2>
      <p class="subtitle">
        {aiProvider === "openrouter" ? t("onb.ai_subtitle_or") : t("onb.ai_subtitle")}
      </p>

      <div class="provider-tabs">
        <button
          class="provider-tab"
          class:active={aiProvider === "anthropic"}
          onclick={() => chooseAiProvider("anthropic")}
        >
          Claude · Anthropic
        </button>
        <button
          class="provider-tab"
          class:active={aiProvider === "openrouter"}
          onclick={() => chooseAiProvider("openrouter")}
        >
          OpenRouter
        </button>
      </div>

      <label class="ai-key">
        <span class="microlabel">
          {aiProvider === "openrouter" ? t("onb.ai_key_label_or") : t("onb.ai_key_label")}
        </span>
        <input
          bind:value={aiKey}
          placeholder={aiProvider === "openrouter" ? "sk-or-…" : "sk-ant-…"}
          spellcheck="false"
          autocomplete="off"
        />
      </label>
      <div class="key-hint">
        {t("onb.ai_no_key")}
        {#if aiProvider === "openrouter"}
          <button class="linkish" onclick={() => openUrl("https://openrouter.ai/settings/keys")}>
            openrouter.ai
          </button>
          <div class="key-hint-detail">{t("onb.ai_key_where_or")}</div>
        {:else}
          <button
            class="linkish"
            onclick={() => openUrl("https://console.anthropic.com/settings/keys")}
          >
            console.anthropic.com
          </button>
          <div class="key-hint-detail">{t("onb.ai_key_where")}</div>
        {/if}
      </div>

      <ul class="features">
        <li>{t("onb.ai_feature_draft")}</li>
        <li>{t("onb.ai_feature_summarize")}</li>
        <li>{t("onb.ai_feature_ask")}</li>
      </ul>

      {#if aiError}
        <div class="error">{aiError}</div>
      {/if}

      <div class="ai-actions">
        <button class="linkish" onclick={finish}>{t("onb.ai_skip")}</button>
        <button class="primary ai-enable" onclick={enableAi} disabled={aiBusy || !aiKey.trim()}>
          {aiVerified
            ? t("onb.ai_verified")
            : aiBusy
              ? t("onb.ai_verifying")
              : `${t("onb.ai_enable")} →`}
        </button>
      </div>
    </div>
  {:else}
    <div class="card connect">
      <div class="microlabel step-label">{t("onb.step", { n: 1, total: 2 })}</div>
      <h2>{t("onb.connect_title")}</h2>
      <p class="subtitle">{t("onb.connect_subtitle")}</p>

      {#if oauthAvailable}
        <button class="primary google" onclick={connectGoogle} disabled={busy !== "none"}>
          {#if busy === "google"}
            {t("onb.waiting_google")}
          {:else}
            <span class="g-badge">G</span>
            {t("onb.continue_google")} →
          {/if}
        </button>
      {/if}
      {#if msOauthAvailable}
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
      {/if}
      {#if oauthAvailable || msOauthAvailable}
        <div class="oauth-note microlabel">🔒 {t("onb.oauth_note")}</div>
      {/if}

      <div class="divider"><span class="microlabel">{t("onb.or_password")}</span></div>

      <form onsubmit={connectPassword}>
        <label>
          <span class="microlabel">{t("onb.email")}</span>
          <input
            type="email"
            bind:value={email}
            onblur={onEmailChange}
            required
            spellcheck="false"
            autocomplete="off"
          />
        </label>
        <label>
          <span class="microlabel">{t("onb.password")}</span>
          <input type="password" bind:value={password} required autocomplete="off" />
        </label>

        {#if preset?.needsAppPassword}
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
      </form>
    </div>
  {/if}
</div>

<style>
  .onboarding {
    flex: 1;
    display: grid;
    place-items: center;
    overflow-y: auto;
    padding: 32px;
  }
  .card {
    width: 460px;
    max-width: 100%;
  }

  .wordmark {
    font-weight: 800;
    font-size: 15px;
    margin-bottom: 48px;
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .mark {
    flex-shrink: 0;
  }
  h1 {
    font-size: 44px;
    font-weight: 800;
    letter-spacing: -0.03em;
    line-height: 1.05;
  }
  h1 em {
    font-style: normal;
    color: var(--text-dim);
  }
  h2 {
    font-size: 26px;
    font-weight: 800;
    letter-spacing: -0.02em;
    margin-top: 8px;
  }
  .subtitle {
    margin-top: 14px;
    color: var(--text-dim);
    font-size: 14px;
    line-height: 1.55;
    max-width: 400px;
  }

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

  .langs {
    margin-top: 56px;
  }
  .lang-row {
    display: flex;
    flex-wrap: wrap;
    gap: 4px 2px;
    margin-top: 8px;
  }
  .lang {
    padding: 5px 10px;
    border-radius: 999px;
    font-size: 12.5px;
    color: var(--text-dim);
  }
  .lang:hover {
    background: var(--hover);
    color: var(--text);
  }
  .lang.active {
    background: var(--text);
    color: var(--bg);
    font-weight: 600;
  }

  .footer {
    margin-top: 40px;
  }
  .step-label {
    margin-bottom: 4px;
  }

  /* AI step — the one place violet is allowed */
  .ai-title {
    color: var(--accent);
  }
  .provider-tabs {
    display: flex;
    gap: 4px;
    margin-top: 20px;
    border-bottom: 1px solid var(--hairline);
  }
  .provider-tab {
    padding: 7px 12px 9px;
    font-size: 13px;
    color: var(--text-dim);
    border-bottom: 2px solid transparent;
    margin-bottom: -1px;
  }
  .provider-tab:hover {
    color: var(--text);
  }
  .provider-tab.active {
    color: var(--text);
    font-weight: 600;
    border-bottom-color: var(--accent);
  }
  .ai-key {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin-top: 16px;
  }
  .ai-key input {
    padding: 10px 12px;
    border: 1px solid var(--accent-dim);
    border-radius: var(--radius-s);
    background: var(--surface);
    font-family: var(--font-mono);
    font-size: 13px;
    user-select: text;
  }
  .ai-key input:focus {
    border-color: var(--accent);
  }
  .key-hint {
    margin-top: 10px;
    font-size: 12.5px;
    color: var(--text-dim);
    line-height: 1.5;
  }
  .key-hint .linkish {
    color: var(--accent);
    font-family: var(--font-mono);
    font-size: 12px;
  }
  .key-hint-detail {
    margin-top: 4px;
    color: var(--text-faint);
    font-size: 12px;
  }

  .features {
    margin: 20px 0 0 18px;
    color: var(--text-dim);
    font-size: 13.5px;
    line-height: 2;
  }
  .ai-actions {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-top: 28px;
    gap: 16px;
  }
  .ai-enable {
    margin-top: 0;
    width: auto;
    padding: 12px 28px;
    background: var(--accent);
    color: var(--on-accent);
  }
  .error {
    color: var(--danger);
    font-size: 13px;
    margin-top: 14px;
    line-height: 1.45;
  }
</style>
