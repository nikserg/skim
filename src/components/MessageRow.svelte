<script lang="ts">
  import type { ThreadRow } from "../lib/types";

  let {
    thread,
    selected = false,
    onselect,
  }: {
    thread: ThreadRow;
    selected?: boolean;
    onselect?: (id: number) => void;
  } = $props();

  function formatDate(unix: number): string {
    const d = new Date(unix * 1000);
    const now = new Date();
    const sameDay = d.toDateString() === now.toDateString();
    if (sameDay)
      return d.toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });
    const days = (now.getTime() - d.getTime()) / 86400000;
    if (days < 7)
      return d.toLocaleDateString(undefined, { weekday: "short" });
    return d.toLocaleDateString(undefined, { month: "short", day: "numeric" });
  }
</script>

<button
  class="row"
  class:unread={!thread.isRead}
  class:selected
  onclick={() => onselect?.(thread.id)}
>
  <div class="line1">
    <span class="from">
      {#if !thread.isRead}<span class="unread-dot"></span>{/if}
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

<style>
  .row {
    display: block;
    width: 100%;
    text-align: left;
    padding: 12px 16px;
    border-bottom: 1px solid var(--hairline);
    transition: background 0.08s;
  }
  .row:hover {
    background: var(--hover);
  }
  .row.selected {
    background: var(--selected);
  }

  .line1 {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    gap: 8px;
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
