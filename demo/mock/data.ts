// Demo fixtures + scripted AI responses.
//
// Everything the mocked IPC layer serves lives here: a small, believable
// (but entirely fake) mailbox and canned AI output for each feature we show
// off. Nothing here talks to a real server, account, or model.

const NOW = Math.floor(Date.now() / 1000);
const H = 3600;
const D = 86400;

export const ACCOUNT = {
  id: "acc-1",
  email: "alex@brightwave.io",
  displayName: "Alex Morgan",
  provider: "generic",
  imapHost: "imap.brightwave.io",
  imapPort: 993,
  smtpHost: "smtp.brightwave.io",
  smtpPort: 465,
  smtpSecurity: "ssl",
  authKind: "password",
};

export const FOLDERS = [
  { id: 1, accountId: "acc-1", imapName: "INBOX", role: "inbox", displayName: "Inbox", unreadCount: 3, sortOrder: 0 },
  { id: 2, accountId: "acc-1", imapName: "Starred", role: "starred", displayName: "Starred", unreadCount: 0, sortOrder: 1 },
  { id: 3, accountId: "acc-1", imapName: "Sent", role: "sent", displayName: "Sent", unreadCount: 0, sortOrder: 2 },
  { id: 4, accountId: "acc-1", imapName: "Drafts", role: "drafts", displayName: "Drafts", unreadCount: 0, sortOrder: 3 },
  { id: 5, accountId: "acc-1", imapName: "Archive", role: "archive", displayName: "Archive", unreadCount: 0, sortOrder: 4 },
  { id: 6, accountId: "acc-1", imapName: "Trash", role: "trash", displayName: "Trash", unreadCount: 0, sortOrder: 5 },
];

// --- Inbox threads (the list on the left) ---------------------------------
export const INBOX_THREADS = [
  {
    id: 101,
    fromName: "Anna Weber",
    fromAddr: "anna.weber@northwind.example",
    subject: "Q3 launch — final checklist & open questions",
    snippet: "Pulling the last threads together before Thursday. Three things still need an owner…",
    date: NOW - 2 * H,
    isRead: false,
    isStarred: false,
    hasAttachments: true,
    messageCount: 5,
  },
  {
    id: 102,
    fromName: "Marcus Lee",
    fromAddr: "marcus@acme-partners.example",
    subject: "Contract redline — v3 ready for your review",
    snippet: "Legal signed off on everything except section 4.2. I left two comments where…",
    date: NOW - 5 * H,
    isRead: false,
    isStarred: false,
    hasAttachments: true,
    messageCount: 2,
  },
  {
    id: 103,
    fromName: "Priya Nair",
    fromAddr: "priya@brightwave.io",
    subject: "Design review moved to Friday 10:00",
    snippet: "Heads up — I pushed the onboarding review to Friday so everyone can join. Calendar…",
    date: NOW - 9 * H,
    isRead: false,
    isStarred: false,
    hasAttachments: false,
    messageCount: 1,
  },
  {
    id: 104,
    fromName: "Stripe",
    fromAddr: "receipts@stripe.example",
    subject: "Your receipt from Brightwave Inc. — $2,400.00",
    snippet: "Thanks for your payment. This receipt is for your records. Invoice #INV-2043…",
    date: NOW - 1 * D - 3 * H,
    isRead: true,
    isStarred: false,
    hasAttachments: false,
    messageCount: 1,
  },
  {
    id: 105,
    fromName: "Jordan Fisher",
    fromAddr: "jordan.fisher@northwind.example",
    subject: "Re: Podcast invite — recording next week?",
    snippet: "Loved the last episode. Would you be up for a 40-minute recording on Wednesday or…",
    date: NOW - 1 * D - 8 * H,
    isRead: true,
    isStarred: true,
    hasAttachments: false,
    messageCount: 3,
  },
  {
    id: 106,
    fromName: "GitHub",
    fromAddr: "notifications@github.example",
    subject: "[brightwave/app] 4 new pull requests need review",
    snippet: "A summary of activity in repositories you watch. #418 Fix flaky sync test, #419…",
    date: NOW - 2 * D,
    isRead: true,
    isStarred: false,
    hasAttachments: false,
    messageCount: 1,
  },
  {
    id: 107,
    fromName: "Sofia Ramos",
    fromAddr: "sofia@brightwave.io",
    subject: "Offsite logistics — hotel + travel",
    snippet: "Booking closes Monday. Please confirm your travel dates so I can lock in the group…",
    date: NOW - 3 * D,
    isRead: true,
    isStarred: false,
    hasAttachments: true,
    messageCount: 4,
  },
];

// Threads shown in other folders (kept short — the demo mostly lives in Inbox).
export const THREADS_BY_FOLDER: Record<number, typeof INBOX_THREADS> = {
  1: INBOX_THREADS,
  2: [INBOX_THREADS[4]], // Starred
  3: [
    {
      id: 301,
      fromName: "Alex Morgan",
      fromAddr: "alex@brightwave.io",
      subject: "Re: Contract redline — v3 ready for your review",
      snippet: "Thanks Marcus — 4.2 looks good on my side. One small tweak to the payment terms…",
      date: NOW - 4 * H,
      isRead: true,
      isStarred: false,
      hasAttachments: false,
      messageCount: 1,
    },
  ],
  4: [],
  5: [],
  6: [],
};

// --- Second mailbox (unified-inbox demo; served only when the recorder sets
// localStorage "skimdemo.multiaccount" = "on") -------------------------------
export const ACCOUNT2 = {
  id: "acc-2",
  email: "morgan.alex@fastmail.example",
  displayName: "Alex Morgan",
  provider: "generic",
  imapHost: "imap.fastmail.example",
  imapPort: 993,
  smtpHost: "smtp.fastmail.example",
  smtpPort: 465,
  smtpSecurity: "ssl",
  authKind: "password",
};

export const FOLDERS2 = [
  { id: 11, accountId: "acc-2", imapName: "INBOX", role: "inbox", displayName: "Inbox", unreadCount: 2, sortOrder: 0 },
  { id: 13, accountId: "acc-2", imapName: "Sent", role: "sent", displayName: "Sent", unreadCount: 0, sortOrder: 2 },
  { id: 16, accountId: "acc-2", imapName: "Trash", role: "trash", displayName: "Trash", unreadCount: 0, sortOrder: 5 },
];

const ACC2_INBOX_THREADS = [
  {
    id: 201,
    accountId: "acc-2",
    fromName: "Lena Kovač",
    fromAddr: "lena@studio-k.example",
    subject: "Ceramics class — spot opened up for Saturday",
    snippet: "You're off the waitlist! The wheel-throwing intro has a free spot this Saturday at…",
    date: NOW - 3 * H,
    isRead: false,
    isStarred: false,
    hasAttachments: false,
    messageCount: 1,
  },
  {
    id: 202,
    accountId: "acc-2",
    fromName: "City Utilities",
    fromAddr: "billing@cityutilities.example",
    subject: "Your March statement is ready",
    snippet: "Your statement for March is now available. Amount due: $84.20 by April 15…",
    date: NOW - 7 * H,
    isRead: false,
    isStarred: false,
    hasAttachments: true,
    messageCount: 1,
  },
  {
    id: 203,
    accountId: "acc-2",
    fromName: "Tomas Rivera",
    fromAddr: "tomas.r@gmail.example",
    subject: "Re: Climbing on Sunday?",
    snippet: "The forecast looks perfect. Meet at the north wall around 9, then coffee after?",
    date: NOW - 1 * D - 5 * H,
    isRead: true,
    isStarred: false,
    hasAttachments: false,
    messageCount: 2,
  },
];

// Drilling into the second mailbox lists its inbox by real folder id.
THREADS_BY_FOLDER[11] = ACC2_INBOX_THREADS;

// The one logical folder set the unified view shows. Virtual ids mirror the
// backend's fixed role ids (inbox −1, starred −2, sent −3, …).
export const UNIFIED_FOLDERS = [
  { id: -1, accountId: "*", imapName: "", role: "inbox", displayName: "Inbox", unreadCount: 5, sortOrder: 0 },
  { id: -2, accountId: "*", imapName: "", role: "starred", displayName: "Starred", unreadCount: 0, sortOrder: 1 },
  { id: -3, accountId: "*", imapName: "", role: "sent", displayName: "Sent", unreadCount: 0, sortOrder: 2 },
  { id: -4, accountId: "*", imapName: "", role: "drafts", displayName: "Drafts", unreadCount: 0, sortOrder: 3 },
  { id: -5, accountId: "*", imapName: "", role: "archive", displayName: "Archive", unreadCount: 0, sortOrder: 4 },
  { id: -6, accountId: "*", imapName: "", role: "trash", displayName: "Trash", unreadCount: 0, sortOrder: 5 },
];

/** Rows for a virtual folder: both mailboxes' counterparts merged by date. */
export function unifiedList(role: string | null): typeof INBOX_THREADS {
  const tag = (rows: typeof INBOX_THREADS, accountId: string) =>
    rows.map((t) => ({ accountId, ...t }));
  const byRole: Record<string, typeof INBOX_THREADS> = {
    inbox: [...tag(INBOX_THREADS, "acc-1"), ...ACC2_INBOX_THREADS],
    starred: tag([INBOX_THREADS[4]], "acc-1"),
    sent: tag(THREADS_BY_FOLDER[3], "acc-1"),
  };
  return (byRole[role ?? ""] ?? []).slice().sort((a, b) => b.date - a.date);
}

/** Role + name of a real folder — the unified view maps citations through it. */
export function folderRef(folderId: number): { role: string | null; displayName: string } {
  const f = [...FOLDERS, ...FOLDERS2].find((f) => f.id === folderId);
  return { role: f?.role ?? null, displayName: f?.displayName ?? "" };
}

// --- Message bodies (the reading pane) ------------------------------------
const HERO_BODY = `
<p>Hi Alex,</p>
<p>Pulling the last threads together before <strong>Thursday's launch</strong>. Most of the plan is locked, but three things still need a clear owner and I don't want them to slip:</p>
<ol>
  <li><strong>Landing page copy</strong> — the new hero section is still in draft. Can your team take the final pass by Wednesday EOD?</li>
  <li><strong>Pricing page</strong> — we agreed to move the annual toggle above the fold, but the change isn't in staging yet.</li>
  <li><strong>Launch email</strong> — I have a rough draft, but it needs your voice before it goes to the 12k list.</li>
</ol>
<p>Two open questions from the wider group:</p>
<ul>
  <li>Do we announce the API beta on day one, or hold it for the follow-up post?</li>
  <li>Marcus flagged that the contract with Acme should be countersigned before we name them in the press note — is that on track?</li>
</ul>
<p>I've attached the current checklist and the draft press note. Let's do a 30-minute sync Thursday morning to close everything out.</p>
<p>Thanks,<br/>Anna</p>
`;

const CONTRACT_BODY = `
<p>Hi Alex,</p>
<p>Legal signed off on everything except <strong>section 4.2</strong> (payment schedule). I left two comments in the doc where I think the net-30 wording is ambiguous.</p>
<p>If you're good with my proposed edit, I can get this countersigned by Friday. The redline is attached as <em>Acme-MSA-v3.pdf</em>.</p>
<p>Best,<br/>Marcus</p>
`;

const DESIGN_BODY = `
<p>Hey everyone,</p>
<p>Quick heads up — I moved the <strong>onboarding design review to Friday at 10:00</strong> so the whole team can join. The Figma link and the latest prototype are in the calendar invite.</p>
<p>Come with feedback on the empty states in particular.</p>
<p>— Priya</p>
`;

const GENERIC_BODY = `<p>Hi Alex,</p><p>Just following up on the thread below — let me know your thoughts when you get a moment.</p><p>Thanks!</p>`;

const BODIES: Record<number, string> = {
  101: HERO_BODY,
  102: CONTRACT_BODY,
  103: DESIGN_BODY,
};

export function bodyFor(threadId: number): string {
  return BODIES[threadId] ?? GENERIC_BODY;
}

// A thread's messages. The reading pane shows the newest one in-folder, so a
// single well-formed message per thread is enough for the demo.
export function threadDetail(threadId: number) {
  const inInbox = INBOX_THREADS.find((x) => x.id === threadId);
  const t = inInbox ?? THREADS_BY_FOLDER[3].find((x) => x.id === threadId);
  if (!t) {
    return { id: threadId, subject: "(demo)", messages: [] };
  }
  const folderId = inInbox ? 1 : 3;
  return {
    id: t.id,
    subject: t.subject,
    messages: [
      {
        id: t.id * 10 + 1,
        folderId,
        threadId: t.id,
        subject: t.subject,
        from: { name: t.fromName, addr: t.fromAddr },
        to: [{ name: "Alex Morgan", addr: "alex@brightwave.io" }],
        cc: [],
        date: t.date,
        snippet: t.snippet,
        isRead: t.isRead,
        isStarred: t.isStarred,
        hasAttachments: t.hasAttachments,
        bodyState: 2,
      },
    ],
  };
}

export function renderedBody(messageId: number) {
  const attachments =
    messageId === 1011
      ? [
          { id: 90001, messageId, filename: "launch-checklist.pdf", mimeType: "application/pdf", size: 84213, isInline: false },
          { id: 90002, messageId, filename: "press-note-draft.docx", mimeType: "application/vnd.openxmlformats-officedocument.wordprocessingml.document", size: 20481, isInline: false },
        ]
      : messageId === 1021
        ? [{ id: 90003, messageId, filename: "Acme-MSA-v3.pdf", mimeType: "application/pdf", size: 156722, isInline: false }]
        : [];
  return {
    messageId,
    html: bodyFor(Math.floor(messageId / 10)),
    blockedImages: 0,
    fromAddr: null,
    attachments,
    invite: null,
  };
}

// --- Search (command palette) ---------------------------------------------
export function searchHits(query: string) {
  const all = [
    {
      messageId: 1011,
      threadId: 101,
      folderId: 1,
      subject: "Q3 launch — final checklist & open questions",
      fromName: "Anna Weber",
      fromAddr: "anna.weber@northwind.example",
      date: NOW - 2 * H,
      snippet: "Landing page copy, pricing page, and the launch email still need an owner…",
    },
    {
      messageId: 1031,
      threadId: 103,
      folderId: 1,
      subject: "Design review moved to Friday 10:00",
      fromName: "Priya Nair",
      fromAddr: "priya@brightwave.io",
      date: NOW - 9 * H,
      snippet: "Moved the onboarding design review to Friday at 10:00 so everyone can join…",
    },
    {
      messageId: 1071,
      threadId: 107,
      folderId: 1,
      subject: "Offsite logistics — hotel + travel",
      fromName: "Sofia Ramos",
      fromAddr: "sofia@brightwave.io",
      date: NOW - 3 * D,
      snippet: "Booking closes Monday — please confirm your travel dates…",
    },
  ];
  const q = query.toLowerCase();
  const hits = all.filter(
    (h) => h.subject.toLowerCase().includes(q) || h.snippet.toLowerCase().includes(q) || h.fromName.toLowerCase().includes(q),
  );
  return hits.length > 0 ? hits : all.slice(0, 2);
}

// --- Drafts ---------------------------------------------------------------
let draftSeq = 5000;
const DRAFTS: Record<number, any> = {};

export function createDraft() {
  const id = ++draftSeq;
  const d = {
    id,
    accountId: "acc-1",
    replyToMessageId: null,
    mode: "new",
    to: "",
    cc: "",
    bcc: "",
    subject: "",
    body: "",
    originMessageId: null,
  };
  DRAFTS[id] = d;
  return d;
}

export function replyTemplate(messageId: number, mode: string) {
  const id = ++draftSeq;
  const threadId = Math.floor(messageId / 10);
  const t = INBOX_THREADS.find((x) => x.id === threadId);
  const quoted = `\n\nOn ${new Date((t?.date ?? NOW) * 1000).toDateString()}, ${t?.fromName ?? "Anna Weber"} <${t?.fromAddr ?? "anna.weber@northwind.example"}> wrote:\n> ${(t?.snippet ?? "").slice(0, 80)}…`;
  const d = {
    id,
    accountId: "acc-1",
    replyToMessageId: messageId,
    mode,
    to: t ? `${t.fromName} <${t.fromAddr}>` : "Anna Weber <anna.weber@northwind.example>",
    cc: "",
    bcc: "",
    subject: (t?.subject ?? "").startsWith("Re:") ? t!.subject : `Re: ${t?.subject ?? "Q3 launch"}`,
    body: quoted,
    originMessageId: null,
  };
  DRAFTS[id] = d;
  return d;
}

export function getDraft(id: number) {
  return DRAFTS[id] ?? createDraftWithId(id);
}
function createDraftWithId(id: number) {
  const d = { id, accountId: "acc-1", replyToMessageId: null, mode: "new", to: "", cc: "", bcc: "", subject: "", body: "", originMessageId: null };
  DRAFTS[id] = d;
  return d;
}
export function updateDraft(d: any) {
  DRAFTS[d.id] = d;
}

// Fixed-id drafts opened directly via #/compose/{id}.
// 7001 = a reply to Anna's Q3 thread (streams AI_REPLY) — the recorder uses this one.
// 7002 = a blank new message (streams AI_COMPOSE_NEW). Not in the scripted tour;
// kept so composing from scratch works when poking at `npm run demo:dev`.
DRAFTS[7001] = {
  id: 7001,
  accountId: "acc-1",
  replyToMessageId: 1011,
  mode: "reply",
  to: "Anna Weber <anna.weber@northwind.example>",
  cc: "",
  bcc: "",
  subject: "Re: Q3 launch — final checklist & open questions",
  body: "\n\nOn " + new Date((NOW - 2 * H) * 1000).toDateString() +
    ", Anna Weber <anna.weber@northwind.example> wrote:\n> Pulling the last threads together before Thursday…",
  originMessageId: null,
};
DRAFTS[7002] = {
  id: 7002,
  accountId: "acc-1",
  replyToMessageId: null,
  mode: "new",
  to: "",
  cc: "",
  bcc: "",
  subject: "",
  body: "",
  originMessageId: null,
};

// --- Scripted AI output ---------------------------------------------------
// Each entry is the full text the "model" streams back for a given feature.

export const AI_SUMMARY = `**Anna is closing out the Q3 launch (Thursday)** and needs three owners:

- **Landing page copy** — final pass due Wednesday EOD (your team).
- **Pricing page** — move the annual toggle above the fold; not in staging yet.
- **Launch email** — draft exists, needs your voice before the 12k send.

**Two decisions to make:**
- Announce the API beta on day one, or hold it for the follow-up post?
- Confirm the Acme contract is countersigned before naming them in the press note.

She's attached the checklist and draft press note, and proposes a 30-min sync Thursday morning.`;

// AI Recap: the inbox catch-up. Unlike the other fixtures this one is a digest
// *across* the unread mail, so it cites all three unread threads — the panel
// marks exactly those as read when it lands, which is the point of the feature.
export const AI_RECAP = {
  text: `**Three things landed while you were away.**

**Anna** is closing out the **Q3 launch** on Thursday. Three items still need an owner, and the **landing page copy** is on your team — final pass by **Wednesday EOD** [1].

**Marcus** has the contract redline back: legal cleared everything except **section 4.2**, and he can have it countersigned by Friday once you approve his edit [2].

**Priya** moved the onboarding **design review to Friday at 10:00** so the whole team can make it [3].`,
  citations: [
    { index: 1, messageId: 1011, threadId: 101, folderId: 1, subject: "Q3 launch — final checklist & open questions", from: "Anna Weber" },
    { index: 2, messageId: 1021, threadId: 102, folderId: 1, subject: "Contract redline — v3 ready for your review", from: "Marcus Lee" },
    { index: 3, messageId: 1031, threadId: 103, folderId: 1, subject: "Design review moved to Friday 10:00", from: "Priya Nair" },
  ],
};

export const AI_ASK = `The launch is **Thursday**. Three items still need an owner: the **landing page copy** (due Wednesday), the **pricing page** toggle change, and the **launch email**. Anna is asking you specifically to take the final pass on the landing copy and to add your voice to the email before it goes out.`;

// A follow-up asked inside an existing email chat. It leans on what was already
// said instead of restating it — that's the point of the dock being one
// continuable session rather than a series of one-shot answers.
export const AI_ASK_FOLLOWUP = `Only the **landing page copy** is yours — Anna asked your team for the final pass by **Wednesday EOD**. The pricing page change sits with the web team, and the launch email just needs your voice on Anna's draft before it goes to the list.`;

// The Translate quick-prompt. The fixture mailbox is written in English, so this
// is a faithful pass-through; it exists so the button does something believable
// when a human pokes at `npm run demo:dev`. The scripted tour doesn't click it.
export const AI_TRANSLATE = `Hi Alex,

Pulling the last threads together before **Thursday's launch**. Three things still need a clear owner: the **landing page copy** (final pass by Wednesday EOD), the **pricing page** (annual toggle above the fold, not yet in staging), and the **launch email** (drafted, but it needs your voice before the 12k send).

Two open questions: whether to announce the API beta on day one or hold it for the follow-up post, and whether the Acme contract is countersigned before we name them in the press note.

The checklist and the draft press note are attached. Anna proposes a 30-minute sync on Thursday morning.`;

export const AI_CHAT = {
  answer: `The **Q3 launch is this Thursday**. The landing page is owned by your team — Anna asked for a final copy pass by **Wednesday EOD** [1]. Priya also moved the onboarding **design review to Friday at 10:00**, so it won't collide with launch day [2].`,
  steps: [
    { kind: "search", arg: "Q3 launch landing page owner", count: 3 },
    { kind: "read", arg: "Q3 launch — final checklist & open questions", count: null },
  ],
  citations: [
    { index: 1, messageId: 1011, threadId: 101, folderId: 1, subject: "Q3 launch — final checklist & open questions", from: "Anna Weber" },
    { index: 2, messageId: 1031, threadId: 103, folderId: 1, subject: "Design review moved to Friday 10:00", from: "Priya Nair" },
  ],
};

// A follow-up in the palette chat: the agent goes back to the mailbox, reads a
// new email and cites it as [3] — [1] keeps its number from the first answer,
// which is what the real agent does with `priorCitations`.
export const AI_CHAT_FOLLOWUP = {
  answer: `Not yet. Marcus has legal sign-off on everything except **section 4.2** (the payment schedule), and he can have it countersigned by **Friday** once you approve his edit [3]. Anna wants that closed before Acme is named in the press note [1].`,
  steps: [
    { kind: "search", arg: "Acme contract countersigned", count: 2 },
    { kind: "read", arg: "Contract redline — v3 ready for your review", count: null },
  ],
  citations: [
    { index: 1, messageId: 1011, threadId: 101, folderId: 1, subject: "Q3 launch — final checklist & open questions", from: "Anna Weber" },
    { index: 3, messageId: 1021, threadId: 102, folderId: 1, subject: "Contract redline — v3 ready for your review", from: "Marcus Lee" },
  ],
};

// --- Picking which scripted answer to serve -------------------------------
// Both AI chats hand the mock their whole history, so the fixtures can react to
// what was actually asked instead of replaying one canned answer forever.

type Turn = { role: string; content: string };

const userTurns = (turns: Turn[] | undefined) => (turns ?? []).filter((t) => t.role === "user");

/** The email chat (`ai_ask`). The quick-prompt buttons send the exact
 *  `ai.prompt_*` strings from en.json — the demo pins `locale: "en"`, so
 *  matching on their English opening word is safe. Anything else is a
 *  free-form question, and a second one in the session is a follow-up. */
export function askAnswer(turns: Turn[] | undefined): string {
  const asked = userTurns(turns);
  const last = (asked[asked.length - 1]?.content ?? "").toLowerCase();
  if (last.startsWith("summarize")) return AI_SUMMARY;
  if (last.startsWith("translate")) return AI_TRANSLATE;
  return asked.length > 1 ? AI_ASK_FOLLOWUP : AI_ASK;
}

/** The mailbox-wide palette chat (`ai_chat`): opener, then follow-up. */
export function chatTurn(turns: Turn[] | undefined): typeof AI_CHAT {
  return userTurns(turns).length > 1 ? AI_CHAT_FOLLOWUP : AI_CHAT;
}

// Reply drafted in the compose window (ai_compose, reply mode).
export const AI_REPLY = `Hi Anna,

Thanks for pulling this together — Thursday works on my side.

My team will take the final pass on the landing page copy and have it to you by Wednesday EOD. I'll also add my voice to the launch email today and send it back for a quick look before it goes to the list.

On the two open questions: let's hold the API beta for the follow-up post so launch day stays focused, and I'll confirm the Acme countersignature with Marcus before we name them in the press note.

See you Thursday morning for the sync.

Best,
Alex`;

// New message drafted from scratch (ai_compose, new mail). Leads with a
// Subject: line so the composer fills the subject field first.
export const AI_COMPOSE_NEW = `Subject: 30-min onboarding sync — Friday?

Hi team,

I'd like to grab 30 minutes on Friday to walk through the new onboarding flow together before the design review. I'm hoping we can align on the empty states and the first-run checklist.

Would 2:00pm work? Happy to move it if that clashes with anything.

Thanks,
Alex`;
