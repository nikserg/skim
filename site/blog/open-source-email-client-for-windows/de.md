---
title: Open-Source-E-Mail-Clients für Windows — die realistische Kurzliste 2026
description: Fünf aktiv gepflegte Open-Source-E-Mail-Clients für Windows 2026, mit Lizenzen, wo deine Credentials gespeichert werden, und wann „Open Source" ein Sternchen trägt.
date: 2026-10-24
updated: 2026-10-28
image: https://skim-tech.com/social-preview.jpg
related: [best-free-email-client-for-windows-11, thunderbird-alternatives-in-2026, best-email-client-for-windows-2026]
---

„Open Source" auf einer E-Mail-Client-Landing kann vieles heißen. Manchmal: Client ist MIT-lizenziert, kein Server, lies den Code. Manchmal: Client ist offen, aber die nützlichen Features (Suche, Später senden, Lesebestätigungen) laufen auf Servern des Anbieters mit deinen Credentials. Beides legitim, aber nicht dasselbe.

## Was prüfen

Drei Fragen: Wo liegen deine Credentials? Ist die Serverkomponente auch Open Source? Wird es noch gepflegt (GitHub-Commit-Datum)?

## Die Kurzliste

### Thunderbird (MPL 2.0)

De-facto-Standard. Alles lokal, kein Vendor-Server im Mail-Pfad. Supernova-Redesign + Unified Inbox 2026. Scope: IMAP, POP, Exchange über IMAP, OpenPGP, Kalender, Kontakte, Feeds. Preis: 300–500 MB RAM im Leerlauf.

### Betterbird (MPL 2.0)

Community-Fork von Thunderbird mit schnelleren QoL-Patches. Vollständig Open Source, kein Server.

### Skim (MIT)

[Skim](/) ist ein nativer Rust-E-Mail-Client für Windows mit Svelte-5-UI in WebView2. ~5 MB Installer, keine Admin-Rechte. Mail per IMAP in lokalen SQLite mit FTS5-Suche; jede Mutation durch lokale Offline-First-Queue. Archivieren/Löschen/Senden passieren sofort und synchronisieren später.

[KI-Features](/#ai) BYOK und lokal-freundlich: Anthropic-/OpenRouter-/OpenAI-kompatibler Endpoint, oder lokal via Ollama/LM Studio auf `localhost:11434`. Credentials und API-Schlüssel im Windows Credential Manager, nicht in Dateien.

Scope: nur Mail. Kein Kalender, keine Kontakte, keine Filter, kein PGP.

Repo: github.com/nikserg/skim. Lizenz: MIT.

### Mailspring (Client GPL-3.0)

Client offen, Pro-Backend proprietär und auf Mailspring-Servern, nicht self-hostbar.

### Claws Mail (GPL-3.0)

Linux-Klassiker, läuft auf Windows. Leicht, GTK-UI, 2008-Ästhetik.

## Vergleich

| Client | Lizenz | Native UI | Cloud | Self-hostbar | Nur Mail |
|---|---|---|---|---|---|
| Thunderbird | MPL 2.0 | Ja (Gecko) | Nein | N/A | Nein |
| Betterbird | MPL 2.0 | Ja (Gecko) | Nein | N/A | Nein |
| Skim | MIT | Ja (Rust+WebView2) | Nein | N/A | Ja |
| Mailspring | GPL-3.0 (Client) | Electron | Nötig für Pro | Nein | Ja |
| Claws Mail | GPL-3.0 | Ja (GTK) | Nein | N/A | Meistens |

## Warum Open Source hier zählt

E-Mail ist der Passwort-Reset-Vektor für dein ganzes digitales Leben. Ein geschlossener Client kann, absichtlich oder nicht. Credentials exfiltrieren oder serverseitig indexieren. Ein nativer, offener, lokal-first Client (Thunderbird, Betterbird, Skim) begrenzt die Vertrauensoberfläche auf App und API, die du lesen kannst.

## FAQ

**Was ist der beste Open-Source-E-Mail-Client für Windows 2026?** Thunderbird für alles, Skim für schnell und Tastatur-first, Betterbird für Thunderbird-ohne-Rauheit.

**Thunderbird portable auf Windows?** Portableapps.com, USB-Stick, keine Installation.

**Ist das neue Outlook Open Source?** Nein.

**Hat Skim ein Erweiterungs-Ökosystem wie Thunderbird?** Nein, absichtlich.

**Mailspring-Backend self-hostbar?** Nein, nur Client ist offen.

**Hat Thunderbird KI?** Nicht eingebaut. Add-ons gibt es. Skim liefert KI im Produkt auf eigenem Schlüssel, [AI-Sektion](/#ai).
