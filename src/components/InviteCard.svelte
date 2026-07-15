<script lang="ts">
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { getLocale, t } from "../lib/i18n/index.svelte";
  import type { InviteView } from "../lib/types";

  let {
    invite,
    onRsvp,
  }: {
    invite: InviteView;
    onRsvp: (response: "accepted" | "declined" | "tentative") => Promise<void>;
  } = $props();

  // Optimistic local answer layered over the stored one; reverted on error.
  let localResponse = $state<string | null>(null);
  let sending = $state(false);
  let failed = $state(false);
  const myResponse = $derived(localResponse ?? invite.myResponse);

  async function respond(response: "accepted" | "declined" | "tentative") {
    if (sending || invite.method !== "request") return;
    const previous = localResponse;
    localResponse = response;
    sending = true;
    failed = false;
    try {
      await onRsvp(response);
    } catch {
      localResponse = previous;
      failed = true;
    } finally {
      sending = false;
    }
  }

  const dateFmt = $derived(
    new Intl.DateTimeFormat(getLocale(), {
      weekday: "short",
      day: "numeric",
      month: "short",
      year: "numeric",
    }),
  );
  const timeFmt = $derived(
    new Intl.DateTimeFormat(getLocale(), { hour: "2-digit", minute: "2-digit" }),
  );

  function allDayLabel(): string {
    if (!invite.startDate) return "";
    // "YYYY-MM-DD" parsed as local midnight to avoid UTC day shifts.
    const start = new Date(`${invite.startDate}T00:00:00`);
    const end = invite.endDate ? new Date(`${invite.endDate}T00:00:00`) : start;
    if (end.getTime() <= start.getTime()) return dateFmt.format(start);
    return `${dateFmt.format(start)} – ${dateFmt.format(end)}`;
  }

  function timedLabel(): string {
    if (invite.startsAt == null) return "";
    const start = new Date(invite.startsAt * 1000);
    const end = invite.endsAt != null ? new Date(invite.endsAt * 1000) : start;
    const sameDay = start.toDateString() === end.toDateString();
    const day = dateFmt.format(start);
    if (end.getTime() === start.getTime()) return `${day}, ${timeFmt.format(start)}`;
    if (sameDay) return `${day}, ${timeFmt.format(start)} – ${timeFmt.format(end)}`;
    return `${day}, ${timeFmt.format(start)} – ${dateFmt.format(end)}, ${timeFmt.format(end)}`;
  }

  const whenLabel = $derived(invite.isAllDay ? allDayLabel() : timedLabel());

  const recurrenceLabel = $derived.by(() => {
    if (!invite.rrule) return null;
    const rule = new Map(
      invite.rrule
        .split(";")
        .map((kv) => kv.split("=") as [string, string])
        .map(([k, v]) => [k.toUpperCase(), v ?? ""]),
    );
    if ((rule.get("INTERVAL") ?? "1") !== "1") return t("invite.recurring");
    switch (rule.get("FREQ")) {
      case "DAILY":
        return t("invite.repeats_daily");
      case "WEEKLY":
        return t("invite.repeats_weekly");
      case "MONTHLY":
        return t("invite.repeats_monthly");
      case "YEARLY":
        return t("invite.repeats_yearly");
      default:
        return t("invite.recurring");
    }
  });

  const organizerLabel = $derived.by(() => {
    if (invite.organizerName && invite.organizerEmail)
      return `${invite.organizerName} <${invite.organizerEmail}>`;
    return invite.organizerName ?? invite.organizerEmail ?? null;
  });

  const responseLabel = $derived.by(() => {
    switch (myResponse) {
      case "accepted":
        return t("invite.you_accepted");
      case "declined":
        return t("invite.you_declined");
      case "tentative":
        return t("invite.you_tentative");
      default:
        return null;
    }
  });

  const replyLine = $derived.by(() => {
    if (invite.method !== "reply") return null;
    const who = invite.replyAttendee ?? "";
    switch (invite.replyPartstat) {
      case "accepted":
        return t("invite.reply_accepted", { who });
      case "declined":
        return t("invite.reply_declined", { who });
      case "tentative":
        return t("invite.reply_tentative", { who });
      default:
        return t("invite.reply_other", { who });
    }
  });
</script>

<div class="invite-card" class:cancelled={invite.method === "cancel"}>
  <div class="head">
    <span class="glyph" aria-hidden="true">
      <svg width="15" height="15" viewBox="0 0 16 16" fill="none">
        <rect x="1.5" y="2.5" width="13" height="12" rx="2" stroke="currentColor" stroke-width="1.2" />
        <path d="M1.5 6h13" stroke="currentColor" stroke-width="1.2" />
        <path d="M5 1v3M11 1v3" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" />
      </svg>
    </span>
    <span class="title">{invite.summary ?? t("invite.untitled")}</span>
    {#if invite.method === "cancel"}
      <span class="pill pill-cancel">{t("invite.cancelled")}</span>
    {:else if responseLabel}
      <span class="pill">{responseLabel}</span>
    {/if}
  </div>

  {#if whenLabel}
    <div class="row">
      <span class="label microlabel">{t("invite.when")}</span>
      <span class:struck={invite.method === "cancel"}>
        {whenLabel}{#if recurrenceLabel}&ensp;·&ensp;{recurrenceLabel}{/if}
      </span>
    </div>
  {/if}
  {#if invite.location}
    <div class="row">
      <span class="label microlabel">{t("invite.where")}</span>
      {#if /^https?:\/\//i.test(invite.location)}
        <button class="location-link" onclick={() => openUrl(invite.location!)}>
          {invite.location}
        </button>
      {:else}
        <span>{invite.location}</span>
      {/if}
    </div>
  {/if}
  {#if organizerLabel}
    <div class="row">
      <span class="label microlabel">{t("invite.organizer")}</span>
      <span>{organizerLabel}</span>
    </div>
  {/if}
  {#if invite.attendeeCount > 1}
    <div class="row">
      <span class="label microlabel">{t("invite.guests")}</span>
      <span>{t("invite.guests_n", { n: invite.attendeeCount })}</span>
    </div>
  {/if}

  {#if invite.method === "reply" && replyLine}
    <div class="row reply-line">{replyLine}</div>
  {/if}

  {#if invite.method === "request" && invite.canRsvp}
    <div class="rsvp">
      <span class="rsvp-label microlabel">{t("invite.going")}</span>
      <button
        class="rsvp-btn"
        class:active={myResponse === "accepted"}
        disabled={sending}
        onclick={() => respond("accepted")}
      >
        {t("invite.yes")}
      </button>
      <button
        class="rsvp-btn"
        class:active={myResponse === "declined"}
        disabled={sending}
        onclick={() => respond("declined")}
      >
        {t("invite.no")}
      </button>
      <button
        class="rsvp-btn"
        class:active={myResponse === "tentative"}
        disabled={sending}
        onclick={() => respond("tentative")}
      >
        {t("invite.maybe")}
      </button>
      {#if failed}
        <span class="rsvp-error">{t("invite.rsvp_failed")}</span>
      {/if}
    </div>
  {/if}
</div>

<style>
  .invite-card {
    margin-top: 14px;
    padding: 14px 16px;
    border: 1px solid var(--hairline-strong);
    border-radius: var(--radius-m);
    font-size: 13px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .invite-card.cancelled {
    color: var(--text-dim);
  }

  .head {
    display: flex;
    align-items: center;
    gap: 10px;
    min-width: 0;
  }
  .glyph {
    display: grid;
    place-items: center;
    color: var(--text-dim);
    flex-shrink: 0;
  }
  .cancelled .glyph {
    color: var(--text-faint);
  }
  .title {
    font-weight: 700;
    font-size: 14px;
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .pill {
    padding: 3px 10px;
    border-radius: 999px;
    border: 1px solid var(--hairline-strong);
    color: var(--text);
    font-size: 11.5px;
    font-weight: 600;
    white-space: nowrap;
    flex-shrink: 0;
  }
  .pill-cancel {
    border-color: var(--hairline-strong);
    color: var(--text-dim);
  }

  .row {
    display: flex;
    gap: 10px;
    align-items: baseline;
    color: var(--text-dim);
    min-width: 0;
  }
  .row .label {
    width: 76px;
    flex-shrink: 0;
    font-size: 10.5px;
  }
  .row span:not(.label) {
    min-width: 0;
    overflow-wrap: anywhere;
  }
  .struck {
    text-decoration: line-through;
  }
  .location-link {
    color: var(--text);
    text-decoration: underline;
    text-underline-offset: 3px;
    font-size: 13px;
    text-align: left;
    min-width: 0;
    overflow-wrap: anywhere;
  }
  .reply-line {
    color: var(--text);
  }

  .rsvp {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
    margin-top: 2px;
  }
  .rsvp-label {
    margin-right: 4px;
  }
  .rsvp-btn {
    padding: 6px 16px;
    border-radius: 999px;
    border: 1px solid var(--hairline-strong);
    color: var(--text);
    font-size: 12.5px;
    font-weight: 600;
    white-space: nowrap;
  }
  .rsvp-btn:hover:not(:disabled) {
    background: var(--hover);
    border-color: var(--text-faint);
  }
  .rsvp-btn:disabled {
    opacity: 0.6;
  }
  .rsvp-btn.active {
    border-color: var(--text-faint);
    color: var(--text);
    background: var(--selected);
  }
  .rsvp-error {
    color: var(--text-dim);
    font-size: 12px;
  }
</style>
