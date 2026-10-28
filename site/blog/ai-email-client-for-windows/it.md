---
title: Client email IA per Windows — la lista onesta del 2026
description: Cosa fa davvero un client email IA, chi fa pagare un abbonamento, chi accetta la tua chiave, e quando una LLM locale via Ollama ha più senso.
date: 2026-10-26
updated: 2026-10-28
image: https://skim-tech.com/social-preview.jpg
related: [best-free-email-client-for-windows-11, outlook-alternatives-for-windows, open-source-email-client-for-windows]
---

"Client email IA" per fornitori diversi significa cose diverse. Per Gmail: il riepilogo Gemini in cima a un thread lungo. Per Spark+ e Canary: un pulsante "Componi con IA" e una bolletta mensile. Per Skim: la tua chiave API, il tuo modello, la posta che resta sulla macchina.

## A cosa serve davvero l'IA nella posta

Quattro cose, per tempo risparmiato: scrivere la bozza di una risposta (i migliori client imparano il tuo stile dai Inviati), riassumere un thread lungo, rispondere a domande su tutta la casella, e un digest dei non letti. Il resto è confezione di marketing.

## Client email IA per Windows nel 2026

### Skim, BYOK, locale, gratis

[Skim](/) è un client email gratuito e open-source per Windows con core Rust nativo. Le [funzioni IA](/#ai), bozza, raffinamento, domande sul thread, chat sull'intera casella, digest dei non letti, usano la *tua* chiave API. Incolla una chiave Anthropic (Claude), OpenRouter (Claude, GPT, Gemini, Grok e centinaia di modelli) o qualsiasi endpoint compatibile OpenAI, e i pulsanti IA viola si accendono. La chiave vive in Windows Credential Manager; le richieste vanno dalla tua macchina direttamente al provider.

Punta Skim a un modello locale su Ollama, LM Studio o vLLM, e il "client email IA per Windows" non chiama mai a casa.

Prezzo: gratis. Costo API: quello che chiede il provider, a prezzo di costo.

### Spark+ (Readdle)

"Componi con IA", suggerimenti, categorizzazione. Posta via server Readdle. ~5–8 $/mese.

### Canary Mail

"Sidekick" scrive, riassume, risponde. ~20–50 $/anno.

### Outlook + Copilot

Microsoft 365 Copilot porta l'IA in Outlook. ~30 $/utente/mese.

### Thunderbird + add-on

Niente IA integrata, ma add-on community collegano ChatGPT/Claude.

## Confronto

| Client | IA nel prodotto | Paghi a | LLM locale | Posta resta locale | Gratis |
|---|---|---|---|---|---|
| Skim | Bozza, QA thread, chat casella, digest | Tu → Anthropic/OpenRouter | Sì (Ollama) | Sì | Sì |
| Spark+ | Compose, replies | Readdle | No | No | Limitato |
| Canary | Compose, riassunti | Canary | No | Parziale | Limitato |
| Outlook + Copilot | Bozza, riassunti | Microsoft | No | No | No |
| Thunderbird + add-on | Dipende | Tu | Sì via add-on | Sì | Sì |

## BYOK vs abbonamento: i numeri reali

20 bozze IA e 3 riassunti di thread al giorno costano $1–3/mese su Claude Haiku, $5–8/mese su GPT-4 via OpenRouter, e $0 su Llama-3 locale in Ollama.

## IA email locale, la strada Ollama

Installa Ollama, `ollama pull llama3.1:8b`, punta il client a `http://localhost:11434/v1`. Skim lo tratta come qualsiasi endpoint compatibile OpenAI.

## FAQ

**Miglior client email IA per Windows?** Per la maggior parte: quello che non ti blocca in un abbonamento. [Skim](/). BYOK, gratis.

**L'IA legge la mia posta?** Nei client BYOK come Skim solo quando lo chiedi, diretto al provider scelto.

**ChatGPT?** Via OpenRouter o API OpenAI, sì in Skim.

**LLM locale?** In Skim sì, qualsiasi endpoint compatibile OpenAI.

**Diverso da Gemini in Gmail?** Sì. Gemini gira lato server nel tuo account Google; un client desktop non serve. BYOK desktop è per chi vuole la posta locale con la propria chiave.
