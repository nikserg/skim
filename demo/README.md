# Skim product demo — scripted, mocked, reproducible

Generates a short **product demo video** of Skim's core features using the **real
app UI** driven by **mocked data and AI responses**. No real account, mailbox, or
model is ever touched, and the whole thing regenerates from a script — so it stays
current as the app evolves and never leaks personal mail.

The tour covers four features:

1. **Summarize** a long thread (streaming AI summary)
2. **Search + Ask** across the mailbox (command palette → AI answer with citations)
3. **Draft a reply with AI** (compose window)
4. **Compose a new email with AI** (from a one-line instruction)

## How it works

Skim's entire backend surface goes through one choke point: the Tauri IPC layer
(`invoke()` and streaming `Channel`s in `src/lib/api.ts`). The UI never talks to a
server directly — it just calls commands like `list_threads` or `ai_summarize`.

So the demo swaps **only that layer**:

- `mock/tauri-core.ts` — a fake `invoke()` (canned mailbox data) and a fake
  `Channel` that **streams** scripted AI text token-by-token (the "typing" effect).
- `mock/tauri-window.ts`, `tauri-event.ts`, `tauri-app.ts`, `plugin-*.ts` — inert stubs.
- `mock/data.ts` — the fake mailbox + every scripted AI response. **Edit this file
  to change what the demo shows.**
- `vite.demo.config.ts` — aliases `@tauri-apps/*` to the mocks (via `VITE`/config only;
  the app source is untouched).
- `record.mjs` — boots the app, drives the scripted tour with a visible cursor
  using Playwright, and records WebM.
- `encode.mjs` — turns the WebM into `skim-demo.mp4` (H.264) and publishes a copy
  to `docs/skim-demo.mp4`, ready to ship with the landing page.
- `screenshots.mjs` — renders the app in both themes and writes
  `docs/skim-light.jpg` and `docs/skim-dark.jpg` (800px wide, aspect kept).

Because it depends only on **command names** (a stable contract) and a few CSS
selectors, it survives UI tweaks. If a feature's IPC command or a key selector is
renamed, update `mock/data.ts` / `record.mjs` accordingly.

## Run it

```bash
# one-time
npm install
npx playwright install chromium

# generate everything (record → encode → screenshots)
npm run demo
```

Outputs (all published into `docs/`, ready for the landing page):

- `docs/skim-demo.mp4` — the demo video (auto-copied on every run)
- `docs/skim-light.jpg`, `docs/skim-dark.jpg` — interface screenshots, both themes
- `demo/output/raw.webm` — the raw recording (re-encode source)

Run steps individually if you prefer:

```bash
npm run demo:record   # -> demo/output/raw.webm
npm run demo:encode   # -> demo/output/skim-demo.mp4 + docs/skim-demo.mp4
npm run demo:shots    # -> docs/skim-light.jpg + docs/skim-dark.jpg
```

### Pacing

`record.mjs` uses readable defaults. Override per-run with env vars (ms):

```bash
DEMO_READ=1600 DEMO_AI_TYPING=18 npm run demo:record
```

`DEMO_READ` (dwell per screen), `DEMO_BEAT` (between actions), `DEMO_TYPE`
(keystroke delay), `DEMO_AI_TYPING` / `DEMO_AI_THINK` / `DEMO_AI_STEP` (AI stream
cadence).

### Fast / CI mode

`DEMO_PREBUILT=1` serves a static build (`demo/dist-demo`) instead of the dev
server — instant startup, useful for time-boxed or headless environments:

```bash
npx vite build --config demo/vite.demo.config.ts
DEMO_PREBUILT=1 node demo/record.mjs
```

## Embed on the landing

```html
<video autoplay muted loop playsinline>
  <source src="skim-demo.mp4" type="video/mp4" />
</video>
```

~1 MB and far crisper than a GIF at the same size.
