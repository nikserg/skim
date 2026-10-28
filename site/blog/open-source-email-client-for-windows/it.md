---
title: Client email open-source per Windows — la lista realistica del 2026
description: Cinque client email open-source attivamente mantenuti per Windows nel 2026, con licenze, dove conservano le credenziali e quando "open source" ha un asterisco.
date: 2026-10-24
updated: 2026-10-28
image: https://skim-tech.com/social-preview.jpg
related: [best-free-email-client-for-windows-11, thunderbird-alternatives-in-2026, best-email-client-for-windows-2026]
---

"Open source" su una landing di un client email significa cose diverse. A volte "client MIT, nessun server, leggi il codice". A volte "client aperto, ma le funzioni utili girano sui nostri server con le tue credenziali". Entrambi sono legittimi, ma non sono la stessa cosa.

## Cosa controllare

Tre domande: dove sono le credenziali?, anche il server è open-source?, è ancora mantenuto (data ultimo commit)?

## Lista breve

### Thunderbird (MPL 2.0)

Standard di fatto. Tutto locale, nessun server del vendor nel percorso. Supernova + casella unificata 2026. IMAP, POP, Exchange via IMAP, OpenPGP, calendari, contatti, feed. 300–500 MB RAM a riposo.

### Betterbird (MPL 2.0)

Fork community con patch più rapide. Open-source completo, niente server.

### Skim (MIT)

[Skim](/) è un client email nativo Rust per Windows con UI Svelte 5 in WebView2. Installer ~5 MB, senza diritti admin. Posta IMAP in SQLite locale con ricerca FTS5; mutazioni tramite coda offline-first, archivio/eliminazione/invio istantanei e riconciliati dopo.

[Funzioni IA](/#ai) BYOK e locali: Anthropic / OpenRouter / endpoint compatibile OpenAI, o locale via Ollama/LM Studio su `localhost:11434`. Credenziali e chiave API in Windows Credential Manager.

Scopo: solo email. Niente calendario, contatti, filtri, PGP.

Repo: github.com/nikserg/skim. Licenza: MIT.

### Mailspring (client GPL-3.0)

Client aperto, back-end Pro proprietario e non auto-ospitabile.

### Claws Mail (GPL-3.0)

Classico Linux, anche Windows. GTK, estetica 2008.

## Confronto

| Client | Licenza | UI nativa | Cloud | Auto-ospitabile | Solo email |
|---|---|---|---|---|---|
| Thunderbird | MPL 2.0 | Sì (Gecko) | No | N/A | No |
| Betterbird | MPL 2.0 | Sì (Gecko) | No | N/A | No |
| Skim | MIT | Sì (Rust+WebView2) | No | N/A | Sì |
| Mailspring | GPL-3.0 (client) | Electron | Per Pro | No | Sì |
| Claws Mail | GPL-3.0 | Sì (GTK) | No | N/A | Perlopiù |

## Perché l'open-source conta qui

L'email è il vettore di reset password per tutto il resto. Un client chiuso può, intenzionalmente o no, esfiltrare credenziali o indicizzare lato server. Un client nativo, aperto, locale (Thunderbird, Betterbird, Skim) riduce la superficie di fiducia all'app leggibile e all'API chiamata.

## FAQ

**Miglior client email open-source per Windows 2026?** Thunderbird per tutto, Skim per veloce, Betterbird per Thunderbird-senza-asperità.

**Thunderbird portable su Windows?** Portableapps.com.

**Il nuovo Outlook è open-source?** No.

**Skim ha ecosistema estensioni come Thunderbird?** No, intenzionalmente.

**Auto-ospitare il back-end Mailspring?** No, solo il client è aperto.

**Thunderbird ha IA?** Non integrata. Add-on. Skim porta IA nel prodotto sulla tua chiave, [AI](/#ai).
