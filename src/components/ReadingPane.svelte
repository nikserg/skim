<script lang="ts">
  import { t } from "../lib/i18n/index.svelte";
  import { mail } from "../lib/stores/mail.svelte";

  const thread = $derived(mail.selectedThread);

  function initial(name: string): string {
    return (name || "?").charAt(0).toUpperCase();
  }

  function formatFull(unix: number): string {
    return new Date(unix * 1000).toLocaleString(undefined, {
      month: "short",
      day: "numeric",
      hour: "numeric",
      minute: "2-digit",
    });
  }
</script>

<section class="pane">
  {#if !thread}
    <div class="placeholder">
      <div class="ghost">✉</div>
      {t("reading.no_selection")}
    </div>
  {:else}
    <div class="scroll">
      <h1 class="subject">{thread.subject || "—"}</h1>

      <div class="meta">
        <span class="avatar">{initial(thread.fromName)}</span>
        <div class="who">
          <div class="from">
            {thread.fromName}
            <span class="addr">&lt;{thread.fromAddr}&gt;</span>
          </div>
          <div class="microlabel">{formatFull(thread.date)}</div>
        </div>
      </div>

      <!-- Full message bodies land in phase 4; the snippet keeps the pane useful meanwhile. -->
      <div class="body" style="user-select: text; cursor: text;">
        {thread.snippet}
      </div>
    </div>
  {/if}
</section>

<style>
  .pane {
    flex: 1;
    display: flex;
    flex-direction: column;
    background: var(--surface);
    min-width: 0;
  }

  .placeholder {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 12px;
    color: var(--text-faint);
    font-size: 13px;
  }
  .ghost {
    font-size: 28px;
    opacity: 0.4;
  }

  .scroll {
    flex: 1;
    overflow-y: auto;
    padding: 28px 36px;
    max-width: 780px;
  }

  .subject {
    font-size: 21px;
    font-weight: 800;
    letter-spacing: -0.02em;
    line-height: 1.25;
  }

  .meta {
    display: flex;
    align-items: center;
    gap: 12px;
    margin-top: 20px;
  }
  .avatar {
    width: 36px;
    height: 36px;
    border-radius: 50%;
    background: var(--selected);
    display: grid;
    place-items: center;
    font-weight: 700;
    font-size: 14px;
    flex-shrink: 0;
  }
  .who {
    flex: 1;
    min-width: 0;
  }
  .from {
    font-weight: 600;
    font-size: 13.5px;
  }
  .addr {
    color: var(--text-faint);
    font-weight: 400;
  }

  .body {
    margin-top: 24px;
    font-size: 14px;
    line-height: 1.65;
    white-space: pre-wrap;
    color: var(--text-dim);
  }
</style>
