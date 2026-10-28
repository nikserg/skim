---
title: Open-source email clients for Windows — the realistic 2026 shortlist
description: Five actually-maintained open-source email clients that run on Windows in 2026, with licenses, where they store your credentials, and when their "open source" has an asterisk.
date: 2026-10-24
updated: 2026-10-28
image: https://skim-tech.com/social-preview.jpg
related: [best-free-email-client-for-windows-11, thunderbird-alternatives-in-2026, best-email-client-for-windows-2026]
---

"Open source" means a lot of things on an email-client landing page. Sometimes it means "the client is MIT-licensed, no server involved, go read the code." Sometimes it means "the client is open-source but the useful parts (search, send-later, read receipts) run on our servers using credentials we hold." Both are legitimate; they are not the same thing. This post is for Windows users who care about the difference.

Below: five open-source email clients that are actively maintained in 2026, with their licenses, their trust boundaries, and when the "open source" claim comes with asterisks.

## What to check before you commit

Three questions, in order of how much they matter:

1. **Where are your credentials stored?** On your device only, or on the vendor's servers so they can offer cloud features? The latter is a legitimate design, but it's a different threat model.
2. **Is the server component also open source?** If the client is open and the server isn't, you can't self-host.
3. **Is it actively maintained?** Email clients abandoned for two years stop getting updates. Check the GitHub repo's last commit date before you install anything.

## The short list

### Thunderbird (MPL 2.0)

The de facto open-source email client on Windows. Maintained by MZLA Technologies Corporation (a Mozilla subsidiary), funded by donations. Everything is local, no vendor server in the mail path. The 2024 Supernova redesign brought the UI into the current decade; the 2026 releases include a usable unified-inbox view and better quick-filter.

Scope: IMAP, POP, Exchange over IMAP, OpenPGP, calendars, contacts, feeds. If you want one open-source tool to run everything, this is it.

Trade: weight. 300–500 MB RAM on idle for a four-mailbox profile.

Repo: hg.mozilla.org/comm-central (also mirrored on GitHub). License: MPL 2.0.

### Betterbird (MPL 2.0)

A community fork of Thunderbird by @rsjtdrjgfuzkfg and friends. Merges quality-of-life patches faster than upstream Thunderbird. Not a different app, think "Thunderbird with the small annoyances patched." Fully open-source, no server.

### Skim (MIT)

[Skim](/) is a native Rust email client for Windows with a Svelte 5 UI running in system WebView2. Installer is about 5 MB, installs per-user, no admin rights, no bundled browser. Mail syncs over IMAP into a local SQLite cache with FTS5 full-text search; every mutation goes through a local offline-first operation queue so archive/delete/star/send happen instantly on the UI and reconcile with the server afterwards.

The [AI features](/#ai) are BYOK and local-friendly: paste your Anthropic / OpenRouter / OpenAI-compatible endpoint key, and requests go straight from your machine to that provider. Point it at Ollama or LM Studio on `localhost:11434` and the AI email features run on-device with no cloud call ever. Credentials and the API key live in Windows Credential Manager, not in config files or the database.

Scope: email only. No calendar, no contacts manager, no filters/rules, no PGP. If you want those, pick Thunderbird.

Repo: github.com/nikserg/skim. License: MIT.

### Mailspring (GPL-3.0 for the client)

The Mailspring client (originally Nylas Mail) is GPL-3.0 and open. The twist: the paid tier's cloud features, read receipts, send-later, snippets, better search, run on Mailspring's servers, and your IMAP credentials live there to make them work. You can run the free tier without the cloud features, but if what drew you in is the slick UX, you are probably going to want them.

Trade: the Pro back-end is not open source and not self-hostable.

### Claws Mail (GPL-3.0)

The classic Linux email client, also runs on Windows. Lightweight, keyboard-first, GTK UI. If you have been using Claws for 15 years you already know whether you want it. If you haven't, the UI will feel like a 2008 time capsule.

Mentioned here for completeness: there is no actively maintained open-source Windows build of KMail, Evolution or Geary. Those are Linux-only in practice.

## Comparison at a glance

| Client | License | Native UI | Cloud component | Self-hostable features | Email-only |
|---|---|---|---|---|---|
| Thunderbird | MPL 2.0 | Yes (Gecko) | None | N/A (local-only) | No (calendar, contacts) |
| Betterbird | MPL 2.0 | Yes (Gecko) | None | N/A (local-only) | No |
| Skim | MIT | Yes (Rust + WebView2) | None | N/A (local-only) | Yes |
| Mailspring | GPL-3.0 (client only) | Electron | Required for Pro features | No | Yes |
| Claws Mail | GPL-3.0 | Yes (GTK) | None | N/A (local-only) | Mostly |

## Why open-source matters here

Email is one of the few applications where a closed-source client is actually a trust problem. Your mail is the password-reset vector for everything else you own. A proprietary client can, intentionally or by accident, exfiltrate credentials, index your mail server-side, or push an update that changes the deal. An open-source client is auditable; a native, open-source, local-first client (Thunderbird, Betterbird, Skim) narrows the trust surface to the app you can read and the API you call.

That's not a hypothetical. Some mainstream clients already route your Gmail / iCloud credentials through their own servers to enable "smart" features. If that bothers you, this is the list.

## FAQ

**What's the best open-source email client for Windows in 2026?** Thunderbird if you want everything, Skim if you want fast and keyboard-first. Betterbird if Thunderbird-but-less-rough is the ask.

**Can I run Thunderbird portable on Windows?** Yes, the Portable edition at portableapps.com runs from a USB stick without installation.

**Is the new Outlook open source?** No. The old Mail/Calendar apps weren't either, but at least they stored credentials locally.

**Does Skim have Thunderbird's add-on library?** No, and intentionally. Skim stays small on purpose. For calendar/contacts/PGP, Thunderbird is still the better fit.

**Can I self-host Mailspring's back-end?** No. The Pro cloud features are a hosted service; the client code is open-source but the server is not.

**Does Thunderbird have AI?** Not built in. There are community add-ons that wire ChatGPT/Claude into compose. Skim ships AI in-product on your own key, see [the AI section](/#ai).
