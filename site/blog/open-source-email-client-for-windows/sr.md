---
title: Open-source email klijenti za Windows — realna kratka lista 2026
description: Pet aktivno održavanih open-source email klijenata za Windows u 2026, s licencama, gde čuvaju vaše podatke za prijavu, i kada „open source" ima zvezdicu.
date: 2026-10-24
updated: 2026-10-28
image: https://skim-tech.com/social-preview.jpg
related: [best-free-email-client-for-windows-11, thunderbird-alternatives-in-2026, best-email-client-for-windows-2026]
---

„Open source" na landing stranici email klijenta znači razne stvari. Nekad „klijent pod MIT, bez servera, čitajte kod". Nekad „klijent otvoren, ali korisne funkcije rade na našim serverima s vašim podacima". Oboje legitimno, ali nije isto.

## Šta proveriti

Tri pitanja: gde su vaši podaci za prijavu?, da li je serverska komponenta takođe open-source?, da li se još održava (datum poslednjeg commit-a)?

## Kratka lista

### Thunderbird (MPL 2.0)

De facto standard. Sve lokalno, nijedan vendorski server u putanji. Supernova + unificirano sanduče 2026. 300–500 MB RAM u mirovanju.

### Betterbird (MPL 2.0)

Community fork sa bržim ispravkama. Potpuno open-source, bez servera.

### Skim (MIT)

[Skim](/) je nativni Rust email klijent za Windows sa Svelte 5 UI u WebView2. Instaler ~5 MB, bez admin prava. Pošta preko IMAP u lokalni SQLite sa FTS5; mutacije kroz lokalni offline-first red čekanja, arhiva/brisanje/slanje trenutni u UI i kasnije sinhroni sa serverom.

[AI funkcije](/#ai) BYOK i local-friendly: Anthropic / OpenRouter / OpenAI-kompatibilan endpoint, ili lokalno preko Ollama/LM Studio na `localhost:11434`. Podaci za prijavu i API ključ u Windows Credential Manager-u.

Opseg: samo pošta. Bez kalendara, kontakata, filtera, PGP.

Repo: github.com/nikserg/skim. Licenca: MIT.

### Mailspring (klijent GPL-3.0)

Klijent otvoren, Pro back-end zatvoren i nije self-hostable.

### Claws Mail (GPL-3.0)

Linux klasik, radi i na Windows-u. GTK, estetika 2008.

## Poređenje

| Klijent | Licenca | Nativan UI | Cloud | Self-hostable | Samo pošta |
|---|---|---|---|---|---|
| Thunderbird | MPL 2.0 | Da (Gecko) | Ne | N/A | Ne |
| Betterbird | MPL 2.0 | Da (Gecko) | Ne | N/A | Ne |
| Skim | MIT | Da (Rust+WebView2) | Ne | N/A | Da |
| Mailspring | GPL-3.0 (klijent) | Electron | Za Pro | Ne | Da |
| Claws Mail | GPL-3.0 | Da (GTK) | Ne | N/A | Većinom |

## Zašto open-source ovde važi

Email je vektor reseta lozinke za sve ostalo. Zatvoren klijent može, namerno ili ne, izvući podatke ili indeksirati na serveru. Nativni, otvoren, lokalni klijent (Thunderbird, Betterbird, Skim) sužava površinu poverenja na aplikaciju i API koje možete pročitati.

## FAQ

**Najbolji open-source email klijent za Windows 2026?** Thunderbird za sve, Skim za brzo, Betterbird za Thunderbird-bez-hrapavosti.

**Thunderbird portable na Windows-u?** Portableapps.com.

**Da li je novi Outlook open-source?** Ne.

**Da li Skim ima ekosistem proširenja kao Thunderbird?** Ne, namerno.

**Self-host Mailspring back-end-a?** Ne, samo klijent je otvoren.

**Da li Thunderbird ima AI?** Nema ugrađen. Dodaci. Skim donosi AI u proizvodu na vašem ključu, [AI](/#ai).
