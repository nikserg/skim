<script lang="ts">
  import { t } from "../lib/i18n/index.svelte";
  import { ui } from "../lib/stores/ui.svelte";

  interface Row {
    label: string;
    keys: string[];
  }
  interface Group {
    title: string;
    rows: Row[];
  }

  // Key captions are the physical keycaps (Latin) — they match e.code and the
  // engraving on any keyboard regardless of layout, so they are NOT localized.
  const groups = $derived<Group[]>([
    {
      title: t("shortcuts.nav"),
      rows: [
        { label: t("shortcuts.next"), keys: ["J"] },
        { label: t("shortcuts.prev"), keys: ["K"] },
        { label: t("shortcuts.clear"), keys: ["Esc"] },
      ],
    },
    {
      title: t("shortcuts.actions"),
      rows: [
        { label: t("reading.archive"), keys: ["E"] },
        { label: t("reading.delete"), keys: ["Del"] },
        { label: t("reading.star"), keys: ["S"] },
        { label: t("reading.toggle_read"), keys: ["U"] },
        { label: t("reading.reply"), keys: ["R"] },
        { label: t("reading.reply_all"), keys: ["A"] },
        { label: t("reading.forward"), keys: ["F"] },
      ],
    },
    {
      // Brand name, matching the hardcoded AI-toggle title in ReadingPane — not localized.
      title: "Skim AI",
      rows: [
        { label: t("ai.draft_reply"), keys: ["D"] },
        { label: t("ai.summarize"), keys: ["M"] },
        { label: t("ai.ask_about"), keys: ["Q"] },
      ],
    },
    {
      title: t("shortcuts.global"),
      rows: [
        { label: t("nav.search"), keys: ["Ctrl K", "/"] },
        { label: t("nav.compose"), keys: ["Ctrl N"] },
        { label: t("palette.toggle_sidebar"), keys: ["."] },
        { label: t("shortcuts.title"), keys: ["?"] },
      ],
    },
  ]);

  function onWindowKeydown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      ui.closeShortcuts();
    }
  }
</script>

<svelte:window onkeydown={onWindowKeydown} />

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
<div class="overlay" onclick={() => ui.closeShortcuts()}>
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div class="panel" onclick={(e) => e.stopPropagation()}>
    <div class="head">
      <span class="title">{t("shortcuts.title")}</span>
      <kbd>ESC</kbd>
    </div>
    <div class="groups">
      {#each groups as group (group.title)}
        <div class="group">
          <div class="microlabel section">{group.title}</div>
          {#each group.rows as row (row.label)}
            <div class="row">
              <span class="label">{row.label}</span>
              <span class="keys">
                {#each row.keys as key, i (key)}
                  {#if i > 0}<span class="or">·</span>{/if}
                  <kbd>{key}</kbd>
                {/each}
              </span>
            </div>
          {/each}
        </div>
      {/each}
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
    padding-top: 12vh;
    z-index: 100;
  }
  .panel {
    width: 520px;
    max-width: calc(100vw - 48px);
    max-height: 70vh;
    background: var(--surface-raised);
    border: 1px solid var(--hairline-strong);
    border-radius: var(--radius-l);
    box-shadow: var(--shadow-pop);
    display: flex;
    flex-direction: column;
    overflow: hidden;
    height: fit-content;
  }

  .head {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 14px 16px;
    border-bottom: 1px solid var(--hairline);
  }
  .title {
    flex: 1;
    font-size: 14px;
    font-weight: 700;
  }

  .groups {
    overflow-y: auto;
    padding: 8px;
  }
  .section {
    padding: 10px 10px 4px;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 7px 10px;
    border-radius: var(--radius-s);
    font-size: 13.5px;
  }
  .label {
    flex: 1;
  }
  .keys {
    display: flex;
    align-items: center;
    gap: 5px;
    flex-shrink: 0;
  }
  .or {
    color: var(--text-faint);
    font-size: 11px;
  }

  kbd {
    font-family: var(--font-mono);
    font-size: 10px;
    color: var(--text-faint);
    border: 1px solid var(--hairline-strong);
    border-radius: 4px;
    padding: 2px 6px;
  }
</style>
