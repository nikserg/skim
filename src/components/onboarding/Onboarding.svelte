<script lang="ts">
  import { getVersion } from "@tauri-apps/api/app";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { aiApi, api, errorMessage, type AiProvider, type OrModel } from "../../lib/api";
  import { getLocale, LOCALES, setLocale, t, type Locale } from "../../lib/i18n/index.svelte";
  import { mail } from "../../lib/stores/mail.svelte";
  import type { Account } from "../../lib/types";
  import ConnectForm from "./ConnectForm.svelte";

  let { oncomplete }: { oncomplete: (account: Account) => void } = $props();

  let step: "welcome" | "connect" | "ai" = $state("welcome");
  let locale: Locale = $state(getLocale());
  let connectedAccount = $state<Account | null>(null);
  let appVersion = $state("");
  void getVersion()
    .then((v) => (appVersion = v))
    .catch(() => {});

  // AI step
  let aiKey = $state("");
  let aiProvider = $state<AiProvider>("anthropic");
  let aiBusy = $state(false);
  let aiVerified = $state(false);
  let aiError = $state("");
  // The user-supplied OpenAI-compatible endpoint.
  let customBaseUrl = $state("");
  let customModel = $state("");
  // Silent Ollama detection, mirroring Settings: an installed, tool-capable
  // catalog turns into clickable chips; any error just means an empty list.
  let ollamaModels = $state<OrModel[]>([]);
  let ollamaStatus = $state<"idle" | "some" | "none">("idle");
  let ollamaGeneration = 0;

  function chooseAiProvider(p: AiProvider) {
    aiProvider = p;
    aiError = "";
    if (p === "custom") void detectOllama();
  }

  async function detectOllama() {
    const url = customBaseUrl.trim();
    if (!url) {
      // Bump the generation here too, so a still-in-flight probe for the
      // previous URL can't land afterwards and repaint chips for a field
      // that's now empty.
      ++ollamaGeneration;
      ollamaModels = [];
      ollamaStatus = "idle";
      return;
    }
    const generation = ++ollamaGeneration;
    try {
      const models = await aiApi.ollamaModels(url);
      if (generation !== ollamaGeneration) return;
      ollamaModels = models;
      ollamaStatus = models.length > 0 ? "some" : "none";
    } catch {
      if (generation !== ollamaGeneration) return;
      ollamaModels = [];
      ollamaStatus = "idle";
    }
  }

  async function accountConnected(account: Account) {
    connectedAccount = account;
    // The AI key lives in Credential Manager, not per-account — if one was set
    // during an earlier onboarding it's still there. Don't ask for it again.
    const status = await aiApi.keyStatus().catch(() => null);
    if (status && (status.anthropic || status.openrouter || status.custom)) {
      finish();
      return;
    }
    step = "ai";
  }

  const aiReady = $derived(
    aiProvider === "custom" ? !!customBaseUrl.trim() && !!customModel.trim() : !!aiKey.trim(),
  );

  async function enableAi() {
    aiBusy = true;
    aiError = "";
    try {
      if (aiProvider === "custom") {
        await aiApi.setCustom(customBaseUrl, aiKey, customModel);
      } else {
        await aiApi.setKey(aiProvider, aiKey);
      }
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

  async function chooseLocale(code: Locale) {
    locale = code;
    await setLocale(code);
    void api.setSetting("locale", code).catch(() => {});
  }

  function toConnect() {
    step = "connect";
  }

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

      <div class="footer microlabel">{t("onb.footer")}{appVersion ? ` · v${appVersion}` : ""}</div>
    </div>
  {:else if step === "ai"}
    <div class="card connect">
      <div class="microlabel step-label">
        {t("onb.step", { n: 2, total: 2 })} · {t("onb.ai_optional")}
      </div>
      <h2 class="ai-title">✦ {t("onb.ai_title")}</h2>
      <p class="subtitle">
        {aiProvider === "custom"
          ? t("onb.ai_subtitle_custom")
          : aiProvider === "openrouter"
            ? t("onb.ai_subtitle_or")
            : t("onb.ai_subtitle")}
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
        <button
          class="provider-tab"
          class:active={aiProvider === "custom"}
          onclick={() => chooseAiProvider("custom")}
        >
          {t("settings.provider_custom")}
        </button>
      </div>

      {#if aiProvider === "custom"}
        <label class="ai-key">
          <span class="microlabel">{t("settings.custom_base_url")}</span>
          <input
            bind:value={customBaseUrl}
            onblur={detectOllama}
            placeholder={t("settings.custom_base_url_ph")}
            spellcheck="false"
            autocomplete="off"
          />
        </label>
        <label class="ai-key">
          <span class="microlabel">{t("settings.custom_key")}</span>
          <input bind:value={aiKey} placeholder="sk-…" spellcheck="false" autocomplete="off" />
        </label>
        <label class="ai-key">
          <span class="microlabel">{t("settings.custom_model")}</span>
          <input
            bind:value={customModel}
            placeholder={t("settings.custom_model_ph")}
            spellcheck="false"
            autocomplete="off"
          />
        </label>
        {#if ollamaStatus === "some"}
          <div class="ai-key">
            <span class="microlabel">{t("settings.custom_models_detected")}</span>
            <div class="lang-row">
              {#each ollamaModels as m (m.id)}
                <button
                  type="button"
                  class="lang"
                  class:active={customModel === m.id}
                  onclick={() => {
                    if (aiBusy) return;
                    customModel = m.id;
                    // The URL is known-good (detection succeeded) and the key
                    // is optional — a chip pick can finish the setup outright.
                    // Ollama's OpenAI API lives under /v1; complete a bare root
                    // so generation doesn't 404 later.
                    const base = customBaseUrl.trim().replace(/\/+$/, "");
                    customBaseUrl = base.endsWith("/v1") ? base : `${base}/v1`;
                    void enableAi();
                  }}
                >
                  {m.name}
                </button>
              {/each}
            </div>
          </div>
        {:else if ollamaStatus === "none"}
          <div class="key-hint">{t("settings.custom_no_tool_models")}</div>
        {/if}
        <div class="key-hint">{t("settings.custom_hint")}</div>
      {:else}
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
      {/if}

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
        <button class="primary ai-enable" onclick={enableAi} disabled={aiBusy || !aiReady}>
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

      <ConnectForm onconnected={(account) => void accountConnected(account)} />
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

  .linkish {
    color: var(--text);
    text-decoration: underline;
    text-underline-offset: 3px;
    font-size: 13px;
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
