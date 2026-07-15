<script lang="ts">
  import { getVersion } from "@tauri-apps/api/app";
  import { disable, enable, isEnabled } from "@tauri-apps/plugin-autostart";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { aiApi, aiStream, api, errorMessage } from "../../lib/api";
  import type { AiProvider } from "../../lib/api";
  import { LOCALES, getLocale, setLocale, t, type Locale } from "../../lib/i18n/index.svelte";
  import { ai } from "../../lib/stores/ai.svelte";
  import { mail } from "../../lib/stores/mail.svelte";
  import { ui } from "../../lib/stores/ui.svelte";
  import type { Lightness, Temperature } from "../../lib/types";

  let { onclose }: { onclose: () => void } = $props();

  // Each matrix cell previews a *different* theme than the active one, so the
  // colors can't come from CSS variables — they're literals mirroring tokens.css
  // (and the Skim Theme Matrix design mock).
  const THEME_PREVIEWS: Record<
    string,
    {
      bg: string;
      surface: string;
      text: string;
      accent: string;
      accentSoft: string;
      accentBorder: string;
      line: string;
      line2: string;
    }
  > = {
    "cold-light": {
      bg: "#f7f7f6",
      surface: "#ffffff",
      text: "#17171b",
      accent: "#6b46f2",
      accentSoft: "#eae7fd",
      accentBorder: "rgba(107,70,242,0.55)",
      line: "rgba(17,17,20,0.16)",
      line2: "rgba(17,17,20,0.12)",
    },
    "warm-light": {
      bg: "#f1eee6",
      surface: "#fbfaf5",
      text: "#1c1712",
      accent: "#6d5296",
      accentSoft: "#ece5f2",
      accentBorder: "rgba(109,82,150,0.55)",
      line: "rgba(28,23,18,0.16)",
      line2: "rgba(28,23,18,0.12)",
    },
    "cold-dark": {
      bg: "#0d0d10",
      surface: "#141418",
      text: "#ececef",
      accent: "#8b6cf7",
      accentSoft: "rgba(139,108,247,0.22)",
      accentBorder: "rgba(139,108,247,0.6)",
      line: "rgba(236,236,239,0.2)",
      line2: "rgba(236,236,239,0.14)",
    },
    "warm-dark": {
      bg: "#14110d",
      surface: "#1a1712",
      text: "#ece7dd",
      accent: "#a58fca",
      accentSoft: "rgba(165,143,202,0.2)",
      accentBorder: "rgba(165,143,202,0.6)",
      line: "rgba(236,231,221,0.2)",
      line2: "rgba(236,231,221,0.14)",
    },
  };

  let aiKeyInput = $state("");
  let aiBusy = $state(false);
  let aiError = $state("");
  let model = $state("claude-sonnet-5");
  let providerTab = $state<AiProvider>("anthropic");
  let orModel = $state("anthropic/claude-sonnet-5");
  let orCustom = $state("");
  let imagesPolicy = $state("block");
  let notifications = $state("on");
  let groupThreads = $state("on");
  let autostart = $state(true);
  let confirmingRemove = $state(false);
  let appVersion = $state("");

  // AI writer profile
  let aiName = $state("");
  let aiStyle = $state("auto");
  let aiInstructions = $state("");

  // "My style" — AI-distilled personal writing style
  let styleProfile = $state("");
  let styleScanning = $state(false);
  let styleProgress = $state<{ current: number; total: number } | null>(null);
  let styleError = $state("");
  let cancelScan: (() => void) | null = null;

  $effect(() => {
    void api.getSettings().then((s) => {
      if (s.ai_model) model = s.ai_model;
      if (s.ai_provider === "openrouter" || s.ai_provider === "anthropic") {
        providerTab = s.ai_provider;
      }
      if (s.openrouter_model) orModel = s.openrouter_model;
      if (s.images_policy) imagesPolicy = s.images_policy;
      if (s.notifications) notifications = s.notifications;
      if (s.group_threads) groupThreads = s.group_threads;
      if (s.ai_user_name) aiName = s.ai_user_name;
      if (s.ai_style) aiStyle = s.ai_style;
      if (s.ai_instructions) aiInstructions = s.ai_instructions;
      if (s.ai_style_profile) styleProfile = s.ai_style_profile;
    });
    void isEnabled()
      .then((on) => (autostart = on))
      .catch(() => {});
    void getVersion()
      .then((v) => (appVersion = v))
      .catch(() => {});
  });

  const STYLES = ["auto", "formal", "friendly", "concise", "sarcastic", "enthusiastic"];

  async function setAutostart(on: boolean) {
    autostart = on;
    try {
      if (on) await enable();
      else await disable();
      // Remembered so startup can restore the Run key after a reinstall.
      await api.setSetting("autostart", on ? "1" : "0");
    } catch {
      autostart = !on;
    }
  }

  async function setAiStyle(style: string) {
    aiStyle = style;
    await api.setSetting("ai_style", style);
  }

  async function chooseMyStyle() {
    aiStyle = "mine";
    await api.setSetting("ai_style", "mine");
    // First activation: distill the style from sent mail.
    if (!styleProfile.trim() && !styleScanning) scanStyle();
  }

  function scanStyle() {
    cancelScan?.();
    styleScanning = true;
    styleError = "";
    styleProfile = "";
    styleProgress = null;
    cancelScan = aiStream(
      "ai_analyze_style",
      {},
      {
        progress: (current, total) => (styleProgress = { current, total }),
        delta: (text) => {
          styleProgress = null;
          styleProfile += text;
        },
        done: () => {
          styleScanning = false;
          styleProgress = null;
        },
        error: (code, message) => {
          styleScanning = false;
          styleProgress = null;
          styleError = code === "ai_no_sent" ? t("settings.style_mine_no_sent") : message;
        },
      },
    );
  }

  function saveStyleProfile() {
    if (styleScanning) return;
    void api.setSetting("ai_style_profile", styleProfile.trim());
  }

  function saveAiName() {
    void api.setSetting("ai_user_name", aiName.trim());
  }

  function saveAiInstructions() {
    void api.setSetting("ai_instructions", aiInstructions.trim());
  }

  const MODELS = [
    { id: "claude-sonnet-5", labelKey: "settings.model_default" },
    { id: "claude-opus-4-8", labelKey: "settings.model_opus" },
    { id: "claude-haiku-4-5-20251001", labelKey: "settings.model_haiku" },
  ];

  // Cross-vendor picks for OpenRouter; any other slug goes in the custom field.
  const OR_MODELS = [
    { id: "anthropic/claude-sonnet-5", label: "Claude Sonnet 5" },
    { id: "openai/gpt-5.1", label: "ChatGPT · GPT-5.1" },
    { id: "google/gemini-3-pro", label: "Gemini 3 Pro" },
    { id: "x-ai/grok-4.1", label: "Grok 4.1" },
  ];
  const orIsPreset = $derived(OR_MODELS.some((m) => m.id === orModel));

  const tabHasKey = $derived(providerTab === "openrouter" ? ai.openrouter : ai.anthropic);

  async function chooseProviderTab(p: AiProvider) {
    providerTab = p;
    aiError = "";
    // Switching to an already-configured provider activates it.
    const hasKey = p === "openrouter" ? ai.openrouter : ai.anthropic;
    if (hasKey && ai.provider !== p) {
      await api.setSetting("ai_provider", p);
      await ai.refresh();
    }
  }

  async function setOrModel(id: string) {
    orModel = id;
    orCustom = "";
    await api.setSetting("openrouter_model", id);
  }

  function saveOrCustom() {
    const v = orCustom.trim();
    if (!v) return;
    orModel = v;
    void api.setSetting("openrouter_model", v);
  }

  async function chooseLocale(code: Locale) {
    await setLocale(code);
    void api.setSetting("locale", code).catch(() => {});
  }

  function setTheme(temperature: Temperature, lightness: Lightness) {
    ui.setTheme(temperature, lightness);
    void api.setSetting("theme", `${temperature}-${lightness}`).catch(() => {});
  }

  async function setModel(id: string) {
    model = id;
    await api.setSetting("ai_model", id);
  }

  async function setImages(policy: string) {
    imagesPolicy = policy;
    await api.setSetting("images_policy", policy);
  }

  async function setNotifications(value: string) {
    notifications = value;
    await api.setSetting("notifications", value);
  }

  async function setGroupThreads(value: string) {
    groupThreads = value;
    await api.setSetting("group_threads", value);
    // Re-render the folder list in the new mode immediately.
    await mail.setGroupThreads(value === "on");
  }

  async function saveAiKey() {
    aiBusy = true;
    aiError = "";
    try {
      await aiApi.setKey(providerTab, aiKeyInput);
      aiKeyInput = "";
      await ai.refresh();
    } catch (e) {
      aiError = errorMessage(e);
    } finally {
      aiBusy = false;
    }
  }

  async function removeAiKey() {
    await aiApi.clearKey(providerTab);
    await ai.refresh();
  }

  async function removeAccount() {
    if (!mail.account) return;
    await api.removeAccount(mail.account.id);
    window.location.reload();
  }
</script>

<!-- A binary on/off setting: label on the left, sliding switch on the right. -->
{#snippet toggleRow(label: string, on: boolean, toggle: () => void)}
  <div class="toggle-row">
    <span class="microlabel">{label}</span>
    <button
      type="button"
      class="switch"
      class:on
      role="switch"
      aria-checked={on}
      aria-label={label}
      onclick={toggle}
    >
      <span class="knob"></span>
    </button>
  </div>
{/snippet}

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
        <div class="theme-matrix">
          <!-- header row: temperature axis -->
          <div></div>
          <div class="axis">{t("theme.cold")}</div>
          <div class="axis">{t("theme.warm")}</div>

          {#each [{ light: "light" as Lightness }, { light: "dark" as Lightness }] as row (row.light)}
            <div class="axis right">{t(`theme.${row.light}`)}</div>
            {#each ["cold" as Temperature, "warm" as Temperature] as temp (temp)}
              {@const p = THEME_PREVIEWS[`${temp}-${row.light}`]}
              <button
                class="cell"
                class:selected={ui.temperature === temp && ui.lightness === row.light}
                style:--p-bg={p.bg}
                style:--p-surface={p.surface}
                style:--p-text={p.text}
                style:--p-accent={p.accent}
                style:--p-accent-soft={p.accentSoft}
                style:--p-accent-border={p.accentBorder}
                style:--p-line={p.line}
                style:--p-line-2={p.line2}
                onclick={() => setTheme(temp, row.light)}
                aria-label={`${t(`theme.${temp}`)} · ${t(`theme.${row.light}`)}`}
                aria-pressed={ui.temperature === temp && ui.lightness === row.light}
              >
                <span class="preview">
                  <span class="p-sidebar">
                    <span class="p-dot"></span>
                    <span class="p-bar"></span>
                    <span class="p-bar short"></span>
                  </span>
                  <span class="p-main">
                    <span class="p-title"></span>
                    <span class="p-line"></span>
                    <span class="p-line short"></span>
                    <span class="p-pill"></span>
                  </span>
                </span>
                {#if ui.temperature === temp && ui.lightness === row.light}
                  <span class="check" aria-hidden="true">
                    <svg width="10" height="10" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M2.5 6.5L5 9l4.5-5.5" /></svg>
                  </span>
                {/if}
              </button>
            {/each}
          {/each}
        </div>
      </section>

      <section class="toggles">
        {@render toggleRow(t("settings.autostart"), autostart, () => setAutostart(!autostart))}
        {@render toggleRow(t("settings.notifications"), notifications === "on", () =>
          setNotifications(notifications === "on" ? "off" : "on"),
        )}
        {@render toggleRow(t("settings.group_threads"), groupThreads === "on", () =>
          setGroupThreads(groupThreads === "on" ? "off" : "on"),
        )}
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

        <div class="microlabel model-label">{t("settings.ai_provider")}</div>
        <div class="tabs">
          <button
            class="tab"
            class:active={providerTab === "anthropic"}
            onclick={() => chooseProviderTab("anthropic")}
          >
            Claude · Anthropic
          </button>
          <button
            class="tab"
            class:active={providerTab === "openrouter"}
            onclick={() => chooseProviderTab("openrouter")}
          >
            OpenRouter
          </button>
        </div>

        {#if tabHasKey}
          <div class="row">
            <span class="ok">●</span>
            <span class="grow">
              {providerTab === "openrouter"
                ? t("settings.ai_key_present_or")
                : t("settings.ai_key_present")}
            </span>
            <button class="ghost" onclick={removeAiKey}>{t("settings.ai_key_remove")}</button>
          </div>
          <div class="microlabel model-label">{t("settings.ai_model")}</div>
          {#if providerTab === "anthropic"}
            <div class="models">
              {#each MODELS as m (m.id)}
                <button class="model" class:active={model === m.id} onclick={() => setModel(m.id)}>
                  {t(m.labelKey)}
                </button>
              {/each}
            </div>
          {:else}
            <div class="models">
              {#each OR_MODELS as m (m.id)}
                <button
                  class="model"
                  class:active={orModel === m.id}
                  onclick={() => setOrModel(m.id)}
                >
                  {m.label}
                  <span class="model-slug">{m.id}</span>
                </button>
              {/each}
              <label class="writer-field or-custom" class:custom-active={!orIsPreset}>
                <span class="microlabel">{t("settings.or_custom")}</span>
                <input
                  bind:value={orCustom}
                  onblur={saveOrCustom}
                  onkeydown={(e) => e.key === "Enter" && saveOrCustom()}
                  placeholder={orIsPreset ? t("settings.or_custom_ph") : orModel}
                  spellcheck="false"
                  autocomplete="off"
                />
              </label>
            </div>
          {/if}

          <div class="microlabel model-label">{t("settings.ai_writer")}</div>
          <div class="writer">
            <label class="writer-field">
              <span class="microlabel">{t("settings.ai_name")}</span>
              <input
                bind:value={aiName}
                onblur={saveAiName}
                placeholder={mail.account?.email.split("@")[0] ?? ""}
                spellcheck="false"
              />
              <span class="dim hint">{t("settings.ai_name_hint")}</span>
            </label>
            <div class="writer-field">
              <span class="microlabel">{t("settings.ai_style")}</span>
              <div class="chips">
                {#each STYLES as style (style)}
                  <button
                    class="chip"
                    class:active={aiStyle === style}
                    onclick={() => setAiStyle(style)}
                  >
                    {t(`settings.style_${style}`)}
                  </button>
                {/each}
                <button
                  class="chip mine"
                  class:active-mine={aiStyle === "mine"}
                  onclick={chooseMyStyle}
                >
                  ✦ {t("settings.style_mine")}
                </button>
              </div>

              {#if aiStyle === "mine"}
                <div class="mine-panel">
                  {#if styleScanning}
                    <div class="mine-progress">
                      <span class="spinner"></span>
                      {#if styleProgress}
                        {t("settings.style_mine_scan")} {styleProgress.current}/{styleProgress.total}
                      {:else}
                        {t("settings.style_mine_writing")}
                      {/if}
                    </div>
                  {/if}
                  <textarea
                    bind:value={styleProfile}
                    onblur={saveStyleProfile}
                    readonly={styleScanning}
                    rows="7"
                    spellcheck="false"
                    placeholder={styleScanning ? "" : t("settings.style_mine_ph")}
                  ></textarea>
                  {#if !styleScanning}
                    <div class="mine-row">
                      <span class="dim hint">{t("settings.style_mine_hint")}</span>
                      <button class="ghost" onclick={scanStyle}>
                        ✦ {t("settings.style_mine_rescan")}
                      </button>
                    </div>
                  {/if}
                  {#if styleError}
                    <div class="warn">{styleError}</div>
                  {/if}
                </div>
              {/if}
            </div>
            <label class="writer-field">
              <span class="microlabel">{t("settings.ai_instructions")}</span>
              <textarea
                bind:value={aiInstructions}
                onblur={saveAiInstructions}
                placeholder={t("settings.ai_instructions_ph")}
                rows="3"
                spellcheck="false"
              ></textarea>
            </label>
          </div>

          <div class="dim note">
            {providerTab === "openrouter" ? t("settings.ai_note_or") : t("settings.ai_note")}
          </div>
        {:else}
          <div class="row">
            <input
              bind:value={aiKeyInput}
              placeholder={providerTab === "openrouter" ? "sk-or-…" : "sk-ant-…"}
              spellcheck="false"
              autocomplete="off"
              class="key-input"
            />
            <button class="ghost" disabled={aiBusy || !aiKeyInput.trim()} onclick={saveAiKey}>
              {aiBusy ? t("onb.ai_verifying") : t("onb.save")}
            </button>
          </div>
          <div class="dim key-hint">
            {t("onb.ai_no_key")}
            {#if providerTab === "openrouter"}
              <button class="key-link" onclick={() => openUrl("https://openrouter.ai/settings/keys")}>
                openrouter.ai
              </button>
            {:else}
              <button
                class="key-link"
                onclick={() => openUrl("https://console.anthropic.com/settings/keys")}
              >
                console.anthropic.com
              </button>
            {/if}
          </div>
          {#if aiError}
            <div class="warn">{aiError}</div>
          {/if}
        {/if}
      </section>

      <div class="about microlabel">
        Skim{appVersion ? ` v${appVersion}` : ""} · {t("onb.footer")} · MIT ·
        <button class="gh-link" onclick={() => openUrl("https://github.com/nikserg/skim")}>
          GitHub
        </button>
      </div>
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

  /* On/off preferences: compact rows with a sliding switch, one section. */
  .toggles {
    gap: 2px;
  }
  .toggle-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 5px 0;
  }
  .switch {
    width: 34px;
    height: 20px;
    border-radius: 999px;
    background: var(--selected);
    position: relative;
    flex-shrink: 0;
    transition: background 0.16s ease;
  }
  .switch .knob {
    position: absolute;
    top: 3px;
    left: 3px;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    background: var(--text-faint);
    transition:
      transform 0.16s ease,
      background 0.16s ease;
  }
  .switch:hover .knob {
    background: var(--text-dim);
  }
  .switch.on {
    background: var(--text);
  }
  .switch.on .knob {
    transform: translateX(14px);
    background: var(--bg);
  }
  .switch:focus-visible {
    outline: 2px solid var(--accent-dim);
    outline-offset: 2px;
  }

  /* Theme matrix: temperature (columns) × lightness (rows), live mini-previews. */
  .theme-matrix {
    display: grid;
    grid-template-columns: 64px 1fr 1fr;
    gap: 10px 12px;
    align-items: center;
  }
  .theme-matrix .axis {
    font-family: var(--font-mono);
    font-size: 10px;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--text-faint);
    text-align: center;
  }
  .theme-matrix .axis.right {
    text-align: right;
  }
  .cell {
    position: relative;
    padding: 0;
    border: none;
    background: none;
    border-radius: 9px;
    cursor: pointer;
  }
  .cell .preview {
    display: flex;
    height: 84px;
    border-radius: 9px;
    overflow: hidden;
    border: 1px solid var(--hairline-strong);
    box-shadow: 0 2px 8px rgba(0, 0, 0, 0.07);
  }
  .cell.selected .preview {
    border-color: var(--text);
    box-shadow:
      0 0 0 2px var(--text),
      0 2px 8px rgba(0, 0, 0, 0.1);
  }
  .p-sidebar {
    width: 36px;
    background: var(--p-bg);
    padding: 9px 6px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .p-dot {
    width: 11px;
    height: 11px;
    border-radius: 50%;
    background: var(--p-accent);
  }
  .p-sidebar .p-bar {
    height: 4px;
    border-radius: 2px;
    background: var(--p-line);
  }
  .p-sidebar .p-bar.short {
    width: 70%;
    background: var(--p-line-2);
  }
  .p-main {
    flex: 1;
    background: var(--p-surface);
    padding: 10px 9px;
    display: flex;
    flex-direction: column;
  }
  .p-title {
    height: 6px;
    width: 64%;
    border-radius: 2px;
    background: var(--p-text);
  }
  .p-main .p-line {
    height: 4px;
    width: 92%;
    border-radius: 2px;
    background: var(--p-line);
    margin-top: 8px;
  }
  .p-main .p-line.short {
    width: 78%;
    background: var(--p-line-2);
    margin-top: 5px;
  }
  .p-pill {
    height: 11px;
    width: 40px;
    border-radius: 6px;
    background: var(--p-accent-soft);
    border: 1px solid var(--p-accent-border);
    margin-top: 10px;
  }
  .check {
    position: absolute;
    top: -7px;
    right: -7px;
    width: 20px;
    height: 20px;
    border-radius: 50%;
    background: var(--text);
    color: var(--bg);
    display: grid;
    place-items: center;
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
  .tabs {
    display: flex;
    gap: 4px;
    border-bottom: 1px solid var(--hairline);
    padding-bottom: 0;
  }
  .tab {
    padding: 7px 12px 9px;
    font-size: 13px;
    color: var(--text-dim);
    border-bottom: 2px solid transparent;
    margin-bottom: -1px;
  }
  .tab:hover {
    color: var(--text);
  }
  .tab.active {
    color: var(--text);
    font-weight: 600;
    border-bottom-color: var(--accent);
  }
  .model-slug {
    display: block;
    font-family: var(--font-mono);
    font-size: 10.5px;
    color: var(--text-faint);
    margin-top: 2px;
  }
  .or-custom input {
    font-family: var(--font-mono);
    font-size: 12px;
  }
  .or-custom.custom-active input {
    border-color: var(--accent);
  }

  /* "My style" — violet, like every AI moment */
  .chip.mine {
    color: var(--accent);
    border-color: var(--accent-dim);
  }
  .chip.mine:hover {
    background: var(--accent-soft);
    color: var(--accent);
  }
  .chip.active-mine {
    background: var(--accent);
    color: var(--on-accent);
    font-weight: 600;
    border-color: var(--accent);
  }
  .mine-panel {
    display: flex;
    flex-direction: column;
    gap: 8px;
    margin-top: 8px;
    border: 1px solid var(--accent-dim);
    border-radius: var(--radius-m);
    padding: 10px;
  }
  .mine-panel textarea {
    border: none;
    padding: 2px;
    font-size: 12.5px;
    line-height: 1.55;
    resize: vertical;
    user-select: text;
    font-family: inherit;
    background: transparent;
  }
  .mine-panel textarea[readonly] {
    color: var(--text-dim);
  }
  .mine-progress {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--accent);
    font-size: 12.5px;
  }
  .mine-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
  }
  .spinner {
    width: 12px;
    height: 12px;
    border: 2px solid var(--accent-dim);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
    flex-shrink: 0;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
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
  .writer {
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  .writer-field {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .writer-field input,
  .writer-field textarea {
    padding: 8px 10px;
    border: 1px solid var(--hairline-strong);
    border-radius: var(--radius-s);
    font-size: 13px;
    user-select: text;
    resize: vertical;
    font-family: inherit;
  }
  .writer-field input:focus,
  .writer-field textarea:focus {
    border-color: var(--accent-dim);
  }
  .hint {
    font-size: 11.5px;
  }
  .key-hint {
    font-size: 12px;
  }
  .key-link {
    color: var(--accent);
    font-family: var(--font-mono);
    font-size: 11.5px;
    text-decoration: underline;
    text-underline-offset: 3px;
  }

  .about {
    text-align: center;
    padding-top: 4px;
  }

  .gh-link {
    color: inherit;
    font: inherit;
    text-decoration: underline;
    text-underline-offset: 2px;
    opacity: 0.85;
  }

  .gh-link:hover {
    color: var(--text);
    opacity: 1;
  }
</style>
