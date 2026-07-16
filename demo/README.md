# Skim product demo — scripted, mocked, reproducible

Generates a short **product demo video** of Skim's core features using the **real
app UI** driven by **mocked data and AI responses**. No real account, mailbox, or
model is ever touched, and the whole thing regenerates from a script — so it stays
current as the app evolves and never leaks personal mail.

The tour covers four features, in one storyline — catch up, dig in, ask around, reply:

1. **AI Recap** — a digest of everything unread, which marks the covered mail read
2. **Ask about the open email** (`✦ Ask` → the `Summarize` quick prompt → a
   follow-up in the same chat)
3. **Search + Ask** across the mailbox (command palette → AI answer with citations
   → a follow-up that sends the agent back to the mailbox)
4. **Draft a reply with AI** (compose window)

Composing a brand-new message isn't in the tour — it's the same AI writing surface
as the reply, and the reply carries the story. The fixtures still cover it
(`AI_COMPOSE_NEW`, draft `7002`) for `npm run demo:dev`.

## How it works

Skim's entire backend surface goes through one choke point: the Tauri IPC layer
(`invoke()` and streaming `Channel`s in `src/lib/api.ts`). The UI never talks to a
server directly — it just calls commands like `list_threads` or `ai_ask`.

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
npm install
npm run demo   # browser check → record → encode → screenshots
```

`npm run demo` starts with `demo:browser` (`playwright install chromium`), which is a
no-op when the browser is already there. That step isn't ceremony: Playwright pins each
browser build to the library version, so any bump — and `^` in `package.json` allows one
on any install — orphans the previously downloaded browser and launching dies with
"Executable doesn't exist". Chromium's headless shell is also a separate download from
Chromium itself, so a half-finished install fails the same way. Re-running the installer
before each session keeps the two in sync instead of relying on someone remembering.

Pin the version (`npm i -D playwright@<version> --save-exact`) if you'd rather not have
npm move it under you and re-download ~150 MB.

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

### Blank-video guard

`record.mjs` samples a frame mid-recording and fails if it compresses to near
nothing — i.e. the video came out blank. This failure is otherwise silent: the tour
passes, the exit code is 0, and a blank MP4 gets published. Failing here means
`demo:encode` never runs and `docs/` keeps the last good video.

If it trips, the usual culprit is screencast capture with a scaled browser context.
The recording context uses `deviceScaleFactor: 1` for that reason; `DEMO_DSF=2` opts
back into 2x if your platform handles it. `DEMO_SKIP_BLANK_CHECK=1` bypasses the guard.

### Themes

Themes are two-axis: `"<cold|warm>-<light|dark>"`. The demo pins **`warm-light`** (the app
default) for the video — override with `DEMO_THEME=cold-dark`. Screenshots render
`warm-light` → `skim-light.jpg` and `warm-dark` → `skim-dark.jpg`; see the `SHOTS` array in
`screenshots.mjs`.

Both scripts set `localStorage.skimdemo.theme`, which the mock returns from `get_settings`.
Note the legacy values `"light"`/`"dark"` still parse but map onto the **cold** palette — so
they render the wrong temperature instead of erroring.

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

~2 MB and far crisper than a GIF at the same size.
