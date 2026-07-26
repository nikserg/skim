<script lang="ts">
  // Rename or delete one of the user's own folders. Opened from the small
  // button next to the list header, so the thing being edited is the thing
  // named right above it.
  //
  // Deleting is offered only while the folder is empty. On every server except
  // Gmail an IMAP DELETE takes the mail inside with it, Skim has no undo, and
  // the case this exists for is "I mistyped a folder into existence" — which is
  // always an empty one.
  import { api, errorMessage } from "../lib/api";
  import { t } from "../lib/i18n/index.svelte";
  import { ui } from "../lib/stores/ui.svelte";

  let name = $state("");
  let count = $state<number | null>(null);
  let busy = $state(false);
  let error = $state<string | null>(null);
  let inputEl: HTMLInputElement | undefined = $state();

  const folder = $derived(ui.folderEditor);

  $effect(() => {
    const f = folder;
    if (!f) return;
    name = f.name;
    count = null;
    error = null;
    busy = false;
    queueMicrotask(() => {
      inputEl?.focus();
      inputEl?.select();
    });
    void api
      .folderMessageCount(f.id)
      .then((n) => {
        if (ui.folderEditor?.id === f.id) count = n;
      })
      .catch(() => {});
  });

  const trimmed = $derived(name.trim());
  const canRename = $derived(trimmed !== "" && trimmed !== folder?.name && !busy);
  // `null` means the count hasn't arrived — stay on the safe side until it does.
  const canDelete = $derived(count === 0 && !busy);

  function close() {
    ui.closeFolderEditor();
  }

  async function rename() {
    const f = folder;
    if (!f || !canRename) return;
    busy = true;
    error = null;
    try {
      await api.renameFolder(f.id, trimmed);
      close();
    } catch (e) {
      error = errorMessage(e);
      busy = false;
    }
  }

  async function remove() {
    const f = folder;
    if (!f || !canDelete) return;
    busy = true;
    error = null;
    try {
      await api.deleteFolder(f.id);
      close();
    } catch (e) {
      error = errorMessage(e);
      busy = false;
    }
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.key === "Enter") {
      e.preventDefault();
      void rename();
    }
  }

  function onWindowKeydown(e: KeyboardEvent) {
    if (folder && e.key === "Escape") {
      e.preventDefault();
      close();
    }
  }
</script>

<svelte:window onkeydown={onWindowKeydown} />

{#if folder}
  <div class="overlay" role="presentation" onclick={close}>
    <div class="panel" role="presentation" onclick={(e) => e.stopPropagation()}>
      <div class="input-row">
        <input
          bind:this={inputEl}
          bind:value={name}
          onkeydown={onKeydown}
          spellcheck="false"
          aria-label={t("folder.rename")}
        />
        <kbd>ESC</kbd>
      </div>

      <div class="actions">
        <button class="btn danger" onclick={remove} disabled={!canDelete}>
          {t("folder.delete")}
        </button>
        <div class="spacer"></div>
        <button class="btn primary" onclick={rename} disabled={!canRename}>
          {t("folder.rename")}
          <kbd>⏎</kbd>
        </button>
      </div>

      {#if error}
        <p class="note error">{error}</p>
      {:else if count !== null && count > 0}
        <!-- Say why deleting is off rather than leaving a dead button. -->
        <p class="note">{t("folder.delete_blocked", { n: count })}</p>
      {/if}
    </div>
  </div>
{/if}

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
    width: 420px;
    max-width: calc(100vw - 48px);
    background: var(--surface-raised);
    border: 1px solid var(--hairline-strong);
    border-radius: var(--radius-l);
    box-shadow: var(--shadow-pop);
    display: flex;
    flex-direction: column;
    height: fit-content;
    overflow: hidden;
  }

  .input-row {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 14px 16px;
    border-bottom: 1px solid var(--hairline);
    color: var(--text-dim);
  }
  .input-row input {
    flex: 1;
    font-size: 15px;
    color: var(--text);
    user-select: text;
  }
  kbd {
    font-family: var(--font-mono);
    font-size: 10px;
    color: var(--text-faint);
    border: 1px solid var(--hairline-strong);
    border-radius: 4px;
    padding: 2px 6px;
  }

  .actions {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 12px 16px;
  }
  .spacer {
    flex: 1;
  }
  .btn {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 7px 14px;
    border-radius: var(--radius-s);
    border: 1px solid var(--hairline-strong);
    font-size: 13px;
    color: var(--text);
  }
  .btn:hover:not(:disabled) {
    background: var(--hover);
  }
  .btn:disabled {
    opacity: 0.4;
    cursor: default;
  }
  .btn.primary {
    background: var(--text);
    border-color: var(--text);
    color: var(--bg);
  }
  .btn.primary:hover:not(:disabled) {
    opacity: 0.85;
    background: var(--text);
  }
  .btn.primary kbd {
    color: var(--bg);
    border-color: transparent;
    padding: 0;
  }
  .btn.danger {
    border-color: transparent;
    color: var(--text-dim);
  }
  .btn.danger:hover:not(:disabled) {
    color: var(--danger);
  }

  .note {
    margin: 0;
    padding: 0 16px 14px;
    font-size: 12px;
    color: var(--text-faint);
  }
  .note.error {
    color: var(--danger);
  }
</style>
