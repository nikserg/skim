---
title: Clients email open-source pour Windows — la courte liste réaliste 2026
description: Cinq clients email open-source réellement maintenus sur Windows en 2026, avec leurs licences, où ils stockent vos identifiants, et quand « open source » vient avec un astérisque.
date: 2026-10-24
updated: 2026-10-28
image: https://skim-tech.com/social-preview.jpg
related: [best-free-email-client-for-windows-11, thunderbird-alternatives-in-2026, best-email-client-for-windows-2026]
---

« Open source » sur une page de client email peut vouloir dire beaucoup de choses. Parfois « client sous MIT, pas de serveur, lisez le code ». Parfois « client ouvert, mais les fonctions utiles (recherche, envoi différé, accusés) tournent sur nos serveurs avec vos identifiants ». Les deux sont légitimes, mais pas identiques.

## À vérifier

Trois questions : où sont vos identifiants ? Le serveur est-il aussi open-source ? Le projet est-il encore maintenu (date du dernier commit) ?

## La liste courte

### Thunderbird (MPL 2.0)

Le standard de facto. Tout local, aucun serveur dans le trajet du courrier. Supernova + boîte unifiée 2026. IMAP, POP, Exchange via IMAP, OpenPGP, agendas, contacts, flux. 300–500 Mo RAM au repos.

### Betterbird (MPL 2.0)

Fork communautaire de Thunderbird avec correctifs plus rapides. Open-source complet, pas de serveur.

### Skim (MIT)

[Skim](/) est un client email natif Rust pour Windows avec UI Svelte 5 dans WebView2. ~5 Mo d'installeur, sans droits admin. Courrier en IMAP vers SQLite local avec recherche FTS5 ; mutations via file d'attente offline-first, archive/suppression/envoi instantanés dans l'UI puis réconciliés plus tard.

[Fonctions IA](/#ai) BYOK et local-friendly : Anthropic / OpenRouter / endpoint compatible OpenAI, ou local via Ollama/LM Studio sur `localhost:11434`. Identifiants et clé API dans Windows Credential Manager.

Portée : email seul. Pas d'agenda, de contacts, de filtres, de PGP.

Dépôt : github.com/nikserg/skim. Licence : MIT.

### Mailspring (client GPL-3.0)

Client ouvert, back-end Pro propriétaire et non auto-hébergeable.

### Claws Mail (GPL-3.0)

Classique Linux, tourne sur Windows. GTK, esthétique 2008.

## Comparaison

| Client | Licence | UI native | Cloud | Auto-hébergeable | Email seul |
|---|---|---|---|---|---|
| Thunderbird | MPL 2.0 | Oui (Gecko) | Non | N/A | Non |
| Betterbird | MPL 2.0 | Oui (Gecko) | Non | N/A | Non |
| Skim | MIT | Oui (Rust+WebView2) | Non | N/A | Oui |
| Mailspring | GPL-3.0 (client) | Electron | Pour Pro | Non | Oui |
| Claws Mail | GPL-3.0 | Oui (GTK) | Non | N/A | Surtout |

## Pourquoi l'open-source compte ici

L'email est le vecteur de réinitialisation de mot de passe pour tout le reste. Un client fermé peut, volontairement ou non, exfiltrer des identifiants ou indexer côté serveur. Un client natif, ouvert, local-first (Thunderbird, Betterbird, Skim) limite la surface de confiance à l'app lisible et à l'API appelée.

## FAQ

**Meilleur client email open-source pour Windows 2026 ?** Thunderbird pour tout, Skim pour rapide, Betterbird pour Thunderbird-sans-aspérités.

**Thunderbird portable sur Windows ?** Portableapps.com.

**Le nouveau Outlook est-il open-source ?** Non.

**Skim a-t-il un écosystème d'extensions comme Thunderbird ?** Non, volontairement.

**Peut-on auto-héberger Mailspring ?** Non, seul le client est ouvert.

**Thunderbird a-t-il de l'IA ?** Pas intégrée. Add-ons communautaires. Skim livre l'IA sur votre clé, [AI section](/#ai).
