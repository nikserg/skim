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
  };
  DRAFTS[id] = d;
  return d;
}

export function getDraft(id: number) {
  return DRAFTS[id] ?? createDraftWithId(id);
}
function createDraftWithId(id: number) {
  const d = { id, accountId: "acc-1", replyToMessageId: null, mode: "new", to: "", cc: "", bcc: "", subject: "", body: "" };
  DRAFTS[id] = d;
  return d;
}
export function updateDraft(d: any) {
  DRAFTS[d.id] = d;
}

// Fixed-id drafts the recorder opens directly via #/compose/{id}.
// 7001 = a reply to Anna's Q3 thread (streams AI_REPLY);
// 7002 = a blank new message (streams AI_COMPOSE_NEW).
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

export const AI_ASK = `The launch is **Thursday**. Three items still need an owner: the **landing page copy** (due Wednesday), the **pricing page** toggle change, and the **launch email**. Anna is asking you specifically to take the final pass on the landing copy and to add your voice to the email before it goes out.`;

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
