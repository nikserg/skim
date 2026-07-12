// Mock data used while the mail backend is under construction (phases 1–2).
import type { Folder, MessageDetail, ThreadRow } from "./types";

const now = Math.floor(Date.now() / 1000);
const H = 3600;
const D = 24 * H;

export const mockFolders: Folder[] = [
  { id: 1, role: "inbox", displayName: "Inbox", unreadCount: 12 },
  { id: 2, role: "starred", displayName: "Starred", unreadCount: 0 },
  { id: 3, role: "sent", displayName: "Sent", unreadCount: 0 },
  { id: 4, role: "drafts", displayName: "Drafts", unreadCount: 2 },
  { id: 5, role: "archive", displayName: "Archive", unreadCount: 0 },
];

export const mockLabels = ["Work", "Personal"];

export const mockThreads: ThreadRow[] = [
  {
    id: 1,
    fromName: "Maya Rönnberg",
    fromAddr: "maya@northstar.io",
    subject: "Q3 roadmap — need your sign-off",
    snippet:
      "Hey, pulled together the milestones we discussed. Could you review the two flagged…",
    date: now - 2 * H,
    isRead: false,
    isStarred: true,
    hasAttachments: false,
    messageCount: 1,
    labels: ["Work"],
  },
  {
    id: 2,
    fromName: "GitHub",
    fromAddr: "notifications@github.com",
    subject: "[skim] PR #482 approved & merged",
    snippet: "tannerlinsley merged 3 commits into main…",
    date: now - 3 * H,
    isRead: false,
    isStarred: false,
    hasAttachments: false,
    messageCount: 1,
    labels: [],
  },
  {
    id: 3,
    fromName: "Luca Moretti",
    fromAddr: "luca@moretti.me",
    subject: "Re: Dinner in Belgrade next week?",
    snippet: "Perfect, Thursday works for me. I'll book the place near the river…",
    date: now - D,
    isRead: true,
    isStarred: false,
    hasAttachments: false,
    messageCount: 4,
    labels: ["Personal"],
  },
  {
    id: 4,
    fromName: "Figma",
    fromAddr: "team@figma.com",
    subject: "Anna invited you to “Skim UI”",
    snippet: "You now have edit access to the file…",
    date: now - D - 5 * H,
    isRead: true,
    isStarred: false,
    hasAttachments: false,
    messageCount: 1,
    labels: [],
  },
  {
    id: 5,
    fromName: "Notion",
    fromAddr: "digest@notion.so",
    subject: "Your weekly digest is ready",
    snippet: "3 pages updated, 1 comment mentions you…",
    date: now - 4 * D,
    isRead: true,
    isStarred: false,
    hasAttachments: false,
    messageCount: 1,
    labels: [],
  },
];

export const mockMessage: MessageDetail = {
  id: 1,
  from: { name: "Maya Rönnberg", addr: "maya@northstar.io" },
  to: [{ name: "Anna", addr: "anna@gmail.com" }],
  cc: [],
  subject: "Q3 roadmap — need your sign-off",
  date: now - 2 * H,
  isRead: false,
  isStarred: true,
  bodyText:
    "Hey,\n\nPulled together the milestones we discussed last week. Could you review the two flagged items and confirm we're still good on timing? I'd love your sign-off before Friday's planning session so the team can start sequencing the work.\n\nThe two at-risk items:\n\n1. Design handoff for the new onboarding — depends on the brand refresh landing this week.\n2. API migration — the vendor moved their deprecation date up by two weeks.\n\nEverything else is tracking green. If you're fine with the buffer I added around the migration, we're set.\n\nMaya",
};

export const mockAiSummary =
  "Maya needs sign-off on the Q3 roadmap. Two milestones are flagged as at-risk (design handoff, API migration). She asks you to reply before Friday's planning.";
