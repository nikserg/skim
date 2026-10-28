---
title: Klienty poczty open-source na Windows — realistyczna krótka lista 2026
description: Pięć aktywnie rozwijanych klientów poczty open-source na Windows w 2026, z licencjami, gdzie trzymają dane logowania, i kiedy „open source" ma gwiazdkę.
date: 2026-10-24
updated: 2026-10-28
image: https://skim-tech.com/social-preview.jpg
related: [best-free-email-client-for-windows-11, thunderbird-alternatives-in-2026, best-email-client-for-windows-2026]
---

„Open source" na landingu klienta poczty znaczy różne rzeczy. Czasem „klient MIT, bez serwera, przeczytaj kod". Czasem „klient otwarty, ale użyteczne funkcje działają na naszych serwerach z twoimi danymi". Obie opcje legalne, ale to nie to samo.

## Co sprawdzić

Trzy pytania: gdzie są twoje dane logowania?, czy komponent serwerowy też jest open-source?, czy nadal rozwijany (data ostatniego commita)?

## Krótka lista

### Thunderbird (MPL 2.0)

Standard de facto. Wszystko lokalnie. Supernova + zunifikowana skrzynka 2026. 300–500 MB RAM na jałowym.

### Betterbird (MPL 2.0)

Fork społeczności z szybszymi poprawkami. Pełny open-source, bez serwera.

### Skim (MIT)

[Skim](/) to natywny klient poczty w Ruście na Windows z UI Svelte 5 w WebView2. ~5 MB instalatora, bez uprawnień admina. Poczta IMAP do lokalnego SQLite z FTS5; mutacje przez lokalną kolejkę offline-first, archiwizacja/usuwanie/wysyłanie natychmiast w UI i synchronizacja z serwerem później.

[Funkcje AI](/#ai) BYOK i local-friendly: Anthropic / OpenRouter / endpoint zgodny z OpenAI, lub lokalnie przez Ollama/LM Studio na `localhost:11434`. Dane logowania i klucz API w Windows Credential Manager.

Zakres: tylko poczta. Bez kalendarza, kontaktów, filtrów, PGP.

Repo: github.com/nikserg/skim. Licencja: MIT.

### Mailspring (klient GPL-3.0)

Klient otwarty, back-end Pro zamknięty i nie do samo-hostowania.

### Claws Mail (GPL-3.0)

Klasyka Linuksa, także Windows. GTK, estetyka 2008.

## Porównanie

| Klient | Licencja | Natywne UI | Chmura | Samo-hostowalne | Tylko poczta |
|---|---|---|---|---|---|
| Thunderbird | MPL 2.0 | Tak (Gecko) | Nie | N/A | Nie |
| Betterbird | MPL 2.0 | Tak (Gecko) | Nie | N/A | Nie |
| Skim | MIT | Tak (Rust+WebView2) | Nie | N/A | Tak |
| Mailspring | GPL-3.0 (klient) | Electron | Dla Pro | Nie | Tak |
| Claws Mail | GPL-3.0 | Tak (GTK) | Nie | N/A | Głównie |

## Dlaczego open-source tu się liczy

Poczta to wektor resetowania hasła dla wszystkiego innego. Zamknięty klient może, celowo lub nie, wyekstrahować dane albo indeksować po stronie serwera. Natywny, otwarty, lokalny klient (Thunderbird, Betterbird, Skim) zawęża powierzchnię zaufania do aplikacji i API, które można przeczytać.

## FAQ

**Najlepszy open-source klient poczty na Windows 2026?** Thunderbird dla wszystkiego, Skim dla szybkiego, Betterbird dla Thunderbird-bez-chropowatości.

**Thunderbird portable na Windows?** Portableapps.com.

**Czy nowy Outlook jest open-source?** Nie.

**Czy Skim ma ekosystem rozszerzeń jak Thunderbird?** Nie, celowo.

**Samo-hosting back-endu Mailspring?** Nie, tylko klient jest otwarty.

**Czy Thunderbird ma AI?** Niewbudowane. Dodatki. Skim niesie AI w produkcie na twoim kluczu, [AI](/#ai).
