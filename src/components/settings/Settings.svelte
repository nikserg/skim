<script lang="ts">
  import { aiApi, api, errorMessage } from "../../lib/api";
  import { LOCALES, getLocale, setLocale, t, type Locale } from "../../lib/i18n/index.svelte";
  import { ai } from "../../lib/stores/ai.svelte";
  import { mail } from "../../lib/stores/mail.svelte";
  import { ui } from "../../lib/stores/ui.svelte";
  import type { Theme } from "../../lib/types";

  let { onclose }: { onclose: () => void } = $props();

  let aiKeyInput = $state("");
  let aiBusy = $state(false);
  let aiError = $state("");
  let model = $state("claude-sonnet-5");
  let imagesPolicy = $state("block");
  let confirmingRemove = $state(false);

  $effect(() => {
    void api.getSettings().then((s) => {
      if (s.ai_model) model = s.ai_model;
      if (s.images_policy) imagesPolicy = s.images_policy;
    });
  });

  const MODELS = [
    { id: "claude-sonnet-5", labelKey: "settings.model_default" },
    { id: "claude-opus-4-8", labelKey: "settings.model_opus" },
    { id: "claude-haiku-4-5-20251001", labelKey: "settings.model_haiku" },
  ];

  async function chooseLocale(code: Locale) {
    await setLocale(code);
    void api.setSetting("locale", code).catch(() => {});
  }

  function setTheme(theme: Theme) {
    ui.setTheme(theme);
    void api.setSetting("theme", theme).catch(() => {});
  }

  async function setModel(id: string) {
    model = id;
    await api.setSetting("ai_model", id);
  }

  async function setImages(policy: string) {
    imagesPolicy = policy;
    await api.setSetting("images_policy", policy);
  }

  async function saveAiKey() {
    aiBusy = true;
    aiError = "";
    try {
      await aiApi.setKey(aiKeyInput);
      aiKeyInput = "";
      await ai.refresh();
    } catch (e) {
      aiError = errorMessage(e);
    } finally {
      aiBusy = false;
    }
  }

  async function removeAiKey() {
    await aiApi.clearKey();
    await ai.refresh();
  }

  async function removeAccount() {
    if (!mail.account) return;
    await api.removeAccount(mail.account.id);
    window.location.reload();
  }
</script>

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
<div class="overlay" onclick={onclose}>
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div class="panel" onclick={(e) => e.stopPropagation()}>
    <header>
      <h2>{t("settings.title")}</h2>
      <button class="close" onclick={onclose} aria-label={t("settings.close")}>
        <svg width="11" height="11" viewBox="0 0 10 10"><path d="M0 0L10 10M10 0L0 10" stroke="currentColor" stroke-width="1.2" /></svg>
      </button>
    </header>

    <div class="body">
      {#if mail.account}
        <section>
          <div class="microlabel">{t("settings.account")}</div>
          <div class="row">
            <span class="avatar">{mail.account.email.charAt(0).toUpperCase()}</span>
            <div class="grow">
              <div class="strong">{mail.account.email}</div>
              <div class="dim">{mail.account.imapHost}</div>
            </div>
            {#if confirmingRemove}
              <button class="danger" onclick={removeAccount}>{t("settings.confirm_remove")}</button>
              <button class="ghost" onclick={() => (confirmingRemove = false)}>{t("settings.cancel")}</button>
            {:else}
              <button class="ghost" onclick={() => (confirmingRemove = true)}>
                {t("settings.remove_account")}
              </button>
            {/if}
          </div>
          {#if confirmingRemove}
            <div class="warn">{t("settings.remove_confirm")}</div>
          {/if}
        </section>
      {/if}

      <section>
        <div class="microlabel">{t("settings.language")}</div>
        <div class="chips">
          {#each LOCALES as l (l.code)}
            <button
              class="chip"
              class:active={getLocale() === l.code}
              onclick={() => chooseLocale(l.code)}
            >
              {l.label}
            </button>
          {/each}
        </div>
      </section>

      <section>
        <div class="microlabel">{t("settings.theme")}</div>
        <div class="chips">
          {#each ["light", "dark", "system"] as themeOption (themeOption)}
            <button
              class="chip"
              class:active={ui.theme === themeOption}
              onclick={() => setTheme(themeOption as Theme)}
            >
              {t(`theme.${themeOption}`)}
            </button>
          {/each}
        </div>
      </section>

      <section>
        <div class="microlabel">{t("settings.images")}</div>
        <div class="chips">
          <button class="chip" class:active={imagesPolicy === "block"} onclick={() => setImages("block")}>
            {t("settings.images_block")}
          </button>
          <button class="chip" class:active={imagesPolicy === "always"} onclick={() => setImages("always")}>
            {t("settings.images_always")}
          </button>
        </div>
      </section>

      <section class="ai-section">
        <div class="microlabel ai-label">✦ {t("settings.ai")}</div>
        {#if ai.keyPresent}
          <div class="row">
            <span class="ok">●</span>
            <span class="grow">{t("settings.ai_key_present")}</span>
            <button class="ghost" onclick={removeAiKey}>{t("settings.ai_key_remove")}</button>
          </div>
          <div class="microlabel model-label">{t("settings.ai_model")}</div>
          <div class="models">
            {#each MODELS as m (m.id)}
              <button class="model" class:active={model === m.id} onclick={() => setModel(m.id)}>
                {t(m.labelKey)}
              </button>
            {/each}
          </div>
          <div class="dim note">{t("settings.ai_note")}</div>
        {:else}
          <div class="row">
            <input
              bind:value={aiKeyInput}
              placeholder="sk-ant-…"
              spellcheck="false"
              autocomplete="off"
              class="key-input"
            />
            <button class="ghost" disabled={aiBusy || !aiKeyInput.trim()} onclick={saveAiKey}>
              {aiBusy ? t("onb.ai_verifying") : t("onb.save")}
            </button>
          </div>
          {#if aiError}
            <div class="warn">{aiError}</div>
          {/if}
        {/if}
      </section>

      <div class="about microlabel">Skim v0.1 · {t("onb.footer")} · MIT</div>
    </div>
  </div>
</div>

<style>
  .overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.35);
    display: flex;
    justify-content: center;
    align-items: flex-start;
    padding-top: 9vh;
    z-index: 100;
  }
  .panel {
    width: 560px;
    max-width: calc(100vw - 48px);
    max-height: 78vh;
    background: var(--surface-raised);
    border: 1px solid var(--hairline-strong);
    border-radius: var(--radius-l);
    box-shadow: var(--shadow-pop);
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 16px 20px 12px;
  }
  h2 {
    font-size: 17px;
    font-weight: 800;
    letter-spacing: -0.02em;
  }
  .close {
    width: 28px;
    height: 28px;
    display: grid;
    place-items: center;
    border-radius: var(--radius-s);
    color: var(--text-dim);
  }
  .close:hover {
    background: var(--hover);
    color: var(--text);
  }

  .body {
    overflow-y: auto;
    padding: 0 20px 20px;
    display: flex;
    flex-direction: column;
    gap: 22px;
  }
  section {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .row {
    display: flex;
    align-items: center;
    gap: 12px;
  }
  .grow {
    flex: 1;
    min-width: 0;
  }
  .strong {
    font-weight: 600;
    font-size: 13.5px;
  }
  .dim {
    color: var(--text-faint);
    font-size: 12px;
  }
  .avatar {
    width: 32px;
    height: 32px;
    border-radius: 50%;
    background: var(--selected);
    display: grid;
    place-items: center;
    font-weight: 700;
    font-size: 13px;
  }

  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }
  .chip {
    padding: 5px 11px;
    border-radius: 999px;
    font-size: 12.5px;
    color: var(--text-dim);
    border: 1px solid transparent;
  }
  .chip:hover {
    background: var(--hover);
    color: var(--text);
  }
  .chip.active {
    background: var(--text);
    color: var(--bg);
    font-weight: 600;
  }

  .ghost {
    padding: 6px 12px;
    border-radius: var(--radius-s);
    border: 1px solid var(--hairline-strong);
    font-size: 12.5px;
    color: var(--text-dim);
    flex-shrink: 0;
  }
  .ghost:hover:not(:disabled) {
    background: var(--hover);
    color: var(--text);
  }
  .ghost:disabled {
    opacity: 0.5;
  }
  .danger {
    padding: 6px 12px;
    border-radius: var(--radius-s);
    background: var(--danger);
    color: #fff;
    font-size: 12.5px;
    font-weight: 600;
    flex-shrink: 0;
  }
  .warn {
    color: var(--danger);
    font-size: 12.5px;
    line-height: 1.45;
  }

  .ai-section {
    border: 1px solid var(--accent-dim);
    border-radius: var(--radius-m);
    padding: 14px;
  }
  .ai-label {
    color: var(--accent);
  }
  .ok {
    color: var(--success);
    font-size: 10px;
  }
  .key-input {
    flex: 1;
    padding: 8px 10px;
    border: 1px solid var(--hairline-strong);
    border-radius: var(--radius-s);
    font-family: var(--font-mono);
    font-size: 12.5px;
    user-select: text;
  }
  .model-label {
    margin-top: 4px;
  }
  .models {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .model {
    text-align: left;
    padding: 8px 12px;
    border-radius: var(--radius-s);
    border: 1px solid var(--hairline);
    font-size: 13px;
    color: var(--text-dim);
  }
  .model:hover {
    background: var(--hover);
  }
  .model.active {
    border-color: var(--accent);
    color: var(--text);
  }
  .note {
    font-size: 11.5px;
  }

  .about {
    text-align: center;
    padding-top: 4px;
  }
</style>
