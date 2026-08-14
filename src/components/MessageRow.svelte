<script lang="ts">
  import { getLocale, t } from "../lib/i18n/index.svelte";
  import { mail } from "../lib/stores/mail.svelte";
  import type { ThreadRow } from "../lib/types";

  let {
    thread,
    selected = false,
    checked = false,
    onselect,
    ontoggle,
  }: {
    thread: ThreadRow;
    selected?: boolean;
    checked?: boolean;
    onselect?: (id: number) => void;
    ontoggle?: (extend: boolean) => void;
  } = $props();

  // Which mailbox this row came from — shown only in the unified view, where
  // rows from every account interleave.
  const badge = $derived(mail.unified ? mail.accountBadge(thread.accountId) : null);

  function formatDate(unix: number): string {
    const locale = getLocale();
    const d = new Date(unix * 1000);
    const now = new Date();
    const sameDay = d.toDateString() === now.toDateString();
    if (sameDay)
      return d.toLocaleTimeString(locale, { hour: "numeric", minute: "2-digit" });
    const days = (now.getTime() - d.getTime()) / 86400000;
    if (days < 7)
      return d.toLocaleDateString(locale, { weekday: "short" });
    return d.toLocaleDateString(locale, { month: "short", day: "numeric" });
  }
</script>

<!-- The checkbox is a sibling of the row button, not a child: a button cannot
     nest inside a button. It sits at the end of the date line, where the row
     already has slack — a leading column would indent the subject and snippet
     on every row forever, to pay for something wanted only now and then. -->
<div class="row-wrap" class:unread={!thread.isRead} class:selected class:checked>
  <button
    class="check"
    aria-label={t("select.toggle")}
    aria-pressed={checked}
    onclick={(e) => ontoggle?.(e.shiftKey)}
  >
    <svg width="9" height="9" viewBox="0 0 10 10" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
      <path d="M1.5 5.2l2.4 2.4L8.5 2.6" />
    </svg>
  </button>
  <button class="row" onclick={() => onselect?.(thread.id)}>
    <div class="line1">
      <span class="from">
        {#if !thread.isRead}<span class="unread-dot"></span>{/if}
        {#if badge}
          <span class="acct" style:background="var(--acct-{badge.color})">{badge.letter}</span>
        {/if}
        {thread.fromName}
        {#if thread.messageCount > 1}<span class="mcount">{thread.messageCount}</span>{/if}
      </span>
      <span class="date">{formatDate(thread.date)}</span>
    </div>
    <div class="subject">
      {#if thread.isStarred}<span class="star">★</span>{/if}
      {thread.subject}
    </div>
    <div class="snippet">{thread.snippet}</div>
  </button>
</div>

<style>
  /* Only a positioning context for the checkbox: the row button below still
     draws the whole row, so its height and hairline are untouched. */
  .row-wrap {
    position: relative;
  }

  .row {
    display: block;
    width: 100%;
    text-align: left;
    padding: 12px 16px;
    border-bottom: 1px solid var(--hairline);
    transition: background 0.08s;
  }
  /* Hover lives on the wrapper: the checkbox sits outside the button, and
     pointing at it must not drop the row's tint. */
  .row-wrap:hover .row {
    background: var(--hover);
  }
  .row-wrap.selected .row {
    background: var(--selected);
  }
  .row-wrap.checked .row {
    background: var(--hover);
  }

  /* Parked at the end of the date line. The slot is always reserved — the date
     is simply set in from the edge — so revealing the box shifts nothing. */
  .check {
    position: absolute;
    top: 13px;
    right: 16px;
    width: 13px;
    height: 13px;
    border: 1px solid var(--text-faint);
    border-radius: 3px;
    color: transparent;
    display: flex;
    align-items: center;
    justify-content: center;
    opacity: 0;
    transition:
      opacity 0.08s,
      border-color 0.08s;
  }
  /* Revealed only when it can be used: pointing at the row, tabbing to it, or
     once a selection exists (then every row shows one). */
  .row-wrap:hover .check,
  .check:focus-visible,
  :global(.selecting) .check {
    opacity: 1;
  }
  .row-wrap.checked .check {
    opacity: 1;
    border-color: var(--text);
    background: var(--text);
    color: var(--surface);
  }

  @media (prefers-reduced-motion: reduce) {
    .row,
    .check {
      transition: none;
    }
  }

  .line1 {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    gap: 8px;
    /* Keeps the date clear of the checkbox slot. Only this line is set in —
       the subject and snippet still run the full width of the row. */
    padding-right: 21px;
  }
  .from {
    font-size: 13.5px;
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .unread .from {
    color: var(--text);
    font-weight: 700;
  }
  .unread-dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--text);
    flex-shrink: 0;
  }
  .mcount {
    font-family: var(--font-mono);
    font-size: 10px;
    color: var(--text-faint);
  }
  /* The mailbox mark: the address's first letter in a colored disc. The letter
     is inked in --surface (white on light themes, near-black on dark), which
     reads against the mid-tone light / pastel dark --acct-* fills. */
  .acct {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    font-family: var(--font-mono);
    font-size: 9px;
    font-weight: 600;
    line-height: 1;
    color: var(--surface);
    flex-shrink: 0;
  }
  .date {
    font-family: var(--font-mono);
    font-size: 10.5px;
    color: var(--text-faint);
    flex-shrink: 0;
  }

  .subject {
    font-size: 13.5px;
    margin-top: 2px;
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .unread .subject {
    font-weight: 600;
  }
  .star {
    color: var(--text-faint);
    margin-right: 2px;
  }

  .snippet {
    font-size: 12.5px;
    color: var(--text-faint);
    margin-top: 2px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

</style>
