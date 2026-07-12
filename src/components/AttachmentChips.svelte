<script lang="ts">
  import { api } from "../lib/api";
  import { t } from "../lib/i18n/index.svelte";
  import type { AttachmentMeta } from "../lib/types";

  let { attachments }: { attachments: AttachmentMeta[] } = $props();

  function formatSize(bytes: number): string {
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(0)} KB`;
    return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
  }
</script>

<div class="chips">
  <div class="microlabel">{t("reading.attachments")}</div>
  <div class="row">
    {#each attachments as a (a.id)}
      <div class="chip">
        <button class="name" onclick={() => api.openAttachment(a.id)} title={t("reading.open")}>
          <svg width="13" height="13" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.2">
            <path d="M10 2.5H4.5v11h7V6L10 2.5zM10 2.5V6h1.5" />
          </svg>
          {a.filename ?? "attachment"}
          <span class="size">{formatSize(a.size)}</span>
        </button>
        <button class="save" onclick={() => api.saveAttachment(a.id)} title={t("reading.save")}>
          <svg width="13" height="13" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.2">
            <path d="M8 2v8m0 0L5 7m3 3l3-3M3 12v2h10v-2" />
          </svg>
        </button>
      </div>
    {/each}
  </div>
</div>

<style>
  .chips {
    margin-top: 16px;
  }
  .row {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    margin-top: 8px;
  }
  .chip {
    display: flex;
    align-items: center;
    border: 1px solid var(--hairline-strong);
    border-radius: var(--radius-s);
    overflow: hidden;
  }
  .name {
    display: flex;
    align-items: center;
    gap: 7px;
    padding: 7px 10px;
    font-size: 12.5px;
    color: var(--text);
  }
  .name:hover {
    background: var(--hover);
  }
  .size {
    color: var(--text-faint);
    font-family: var(--font-mono);
    font-size: 10.5px;
  }
  .save {
    padding: 7px 9px;
    border-left: 1px solid var(--hairline);
    color: var(--text-dim);
    display: grid;
    place-items: center;
  }
  .save:hover {
    background: var(--hover);
    color: var(--text);
  }
</style>
