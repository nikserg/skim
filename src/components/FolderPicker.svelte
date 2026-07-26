<script lang="ts">
  // "Move to…": filter the account's folders, or create one on the spot.
  // Deliberately the same panel from all three entry points (the toolbar
  // button, V, and the palette) — a folder list is long enough to need a
  // filter, and a filter box is a palette, so it may as well look like one.
  import { api } from "../lib/api";
  import { folderIcon, folderLabel } from "../lib/folders";
  import { t } from "../lib/i18n/index.svelte";
  import { mail } from "../lib/stores/mail.svelte";
  import { ui } from "../lib/stores/ui.svelte";
  import type { Folder } from "../lib/types";

  let filter = $state("");
  let active = $state(0);
  let targets = $state<Folder[]>([]);
  let inputEl: HTMLInputElement | undefined = $state();

  const request = $derived(ui.movePicker);

  // Load the destinations each time the picker opens: which folders are on
  // offer depends on where the message currently is.
  $effect(() => {
    const req = request;
    if (!req) return;
    filter = "";
    active = 0;
    targets = [];
    queueMicrotask(() => inputEl?.focus());
    void api
      .moveTargets(req.messageIds)
      .then((folders) => {
        // A slow answer for a picker the user already closed (or reopened for
        // another message) must not land here.
        if (ui.movePicker === req) targets = folders;
      })
      .catch(() => {});
  });

  const query = $derived(filter.trim());

  const matches = $derived(
    query === ""
      ? targets
      : targets.filter((f) => folderLabel(f).toLowerCase().includes(query.toLowerCase())),
  );

  // The folder being read isn't among the targets (you can't file mail where it
  // already is), so it needs naming separately — otherwise typing its name would
  // offer to "create" it. Works in the unified view too, where the selected
  // folder is virtual but carries the same name.
  const currentName = $derived(
    mail.selectedFolder ? folderLabel(mail.selectedFolder).toLowerCase() : null,
  );

  // Offer to create only when the typed name isn't already a folder — and never
  // as the default highlight, so Enter can't invent a folder while a real match
  // is on screen.
  const canCreate = $derived(
    query !== "" &&
      query.toLowerCase() !== currentName &&
      !targets.some((f) => folderLabel(f).toLowerCase() === query.toLowerCase()),
  );
  const createIndex = $derived(canCreate ? matches.length : -1);
  const total = $derived(matches.length + (canCreate ? 1 : 0));

  function close() {
    ui.closeMove();
  }

  function activate(index: number) {
    const req = request;
    if (!req) return;
    const create = index === createIndex;
    const folder = create ? null : matches[index];
    if (!create && !folder) return;
    close();
    // Optimistic, exactly like archive: the row leaves the list now and the
    // queued op catches the server up. A move that fails for good surfaces as
    // an ops:failed notice, which also refreshes the list back.
    mail.removeThreadFromList(req.threadId);
    void api.moveMessages(req.messageIds, folder?.id ?? null, create ? query : undefined);
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      active = Math.min(active + 1, total - 1);
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      active = Math.max(active - 1, 0);
    } else if (e.key === "Enter") {
      e.preventDefault();
      activate(active);
    }
  }

  // Typing shrinks the list under the cursor; keep the highlight on a real row.
  $effect(() => {
    if (active >= total) active = Math.max(0, total - 1);
  });

  function onWindowKeydown(e: KeyboardEvent) {
    if (request && e.key === "Escape") {
      e.preventDefault();
      close();
    }
  }
</script>

<svelte:window onkeydown={onWindowKeydown} />

{#if request}
  <div
    class="overlay"
    role="presentation"
    onclick={close}
  >
    <div class="panel" role="presentation" onclick={(e) => e.stopPropagation()}>
      <div class="input-row">
        <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.3" stroke-linejoin="round">
          <path d="M1.5 3.5h4l1.5 2h7.5v7h-13v-9z" />
        </svg>
        <input
          bind:this={inputEl}
          bind:value={filter}
          onkeydown={onKeydown}
          placeholder={t("move.filter")}
          spellcheck="false"
          aria-label={t("move.title")}
        />
        <kbd>ESC</kbd>
      </div>

      <div class="items" role="listbox" aria-label={t("move.title")}>
        {#each matches as folder, i (folder.id)}
          <button
            class="item"
            class:active={active === i}
            role="option"
            aria-selected={active === i}
            onclick={() => activate(i)}
            onmouseenter={() => (active = i)}
          >
            {#if folder.role}
              <svg class="icon" width="15" height="15" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.2" stroke-linejoin="round">
                <path d={folderIcon(folder.role)} />
              </svg>
            {:else}
              <span class="icon dot-wrap"><span class="dot"></span></span>
            {/if}
            <span class="label">{folderLabel(folder)}</span>
          </button>
        {/each}

        {#if canCreate}
          <button
            class="item create"
            class:active={active === createIndex}
            role="option"
            aria-selected={active === createIndex}
            onclick={() => activate(createIndex)}
            onmouseenter={() => (active = createIndex)}
          >
            <span class="icon plus">+</span>
            <span class="label">{t("move.create", { name: query })}</span>
          </button>
        {/if}

        {#if total === 0}
          <div class="empty">{t("move.empty")}</div>
        {/if}
      </div>
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
    width: 460px;
    max-width: calc(100vw - 48px);
    max-height: 60vh;
    background: var(--surface-raised);
    border: 1px solid var(--hairline-strong);
    border-radius: var(--radius-l);
    box-shadow: var(--shadow-pop);
    display: flex;
    flex-direction: column;
    overflow: hidden;
    height: fit-content;
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

  .items {
    overflow-y: auto;
    padding: 8px;
  }
  .item {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    text-align: left;
    padding: 8px 10px;
    border-radius: var(--radius-s);
    font-size: 13.5px;
  }
  .item.active {
    background: var(--selected);
  }
  .icon {
    width: 15px;
    flex-shrink: 0;
    color: var(--text-faint);
    text-align: center;
  }
  .label {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* Same hollow ring the sidebar uses for a user label. */
  .dot-wrap {
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .dot {
    width: 7px;
    height: 7px;
    border: 1.2px solid currentColor;
    border-radius: 50%;
  }

  /* Creating is the fallback, not the offer — it reads quieter than the
     folders that already exist. */
  .create .label {
    color: var(--text-dim);
  }
  .plus {
    font-size: 14px;
    line-height: 1;
  }

  .empty {
    padding: 24px;
    text-align: center;
    color: var(--text-faint);
    font-size: 13px;
  }
</style>
