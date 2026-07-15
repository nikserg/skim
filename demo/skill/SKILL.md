---
name: update-demo-assets
description: Regenerate Skim's landing-page demo video and interface screenshots from the mocked demo harness in demo/. Use this whenever the Skim UI has changed and the landing assets need refreshing — including phrases like "update the demo", "regenerate the demo video", "re-record the demo", "refresh the screenshots", "I changed the interface, update the landing assets", or "the demo looks out of date". Also use it when a demo run fails and the scripted tour needs repair after a UI change.
---

# Update Skim demo assets

Regenerates the product demo video and the light/dark interface screenshots that ship
with the landing page. Everything is driven by the **real Skim UI** running against a
**mocked Tauri IPC layer** (`demo/mock/`), so no real account, mailbox, or model is
ever touched.

Outputs, all published into `docs/`:

- `skim-demo.mp4` — the scripted feature tour (1280×800)
- `skim-light.jpg`, `skim-dark.jpg` — interface screenshots, 800px wide

## Do not commit or push

This skill only updates files on disk. Do not run `git add`, `git commit`, or `git push`.
The user reviews the regenerated assets themselves and decides what to keep — these are
binary files that produce large diffs, and a run that looks fine by exit code can still
be visually wrong. Leaving the working tree dirty is the correct outcome.

Only touch git if the user explicitly asks in that same request (e.g. "regenerate and
commit them"). Otherwise, finish by telling them the files are updated and unstaged.

## Running it

From the repo root:

```bash
npm run demo
```

That chains three steps: `demo:record` (Playwright drives the real UI → `demo/output/raw.webm`)
→ `demo:encode` (ffmpeg → MP4, copied into `docs/`) → `demo:shots` (both themes → `docs/`).

If only part of the UI changed, run just what's needed — it's faster and keeps unrelated
assets byte-identical:

- `npm run demo:shots` — screenshots only (static UI, colors, layout changes)
- `npm run demo:record && npm run demo:encode` — video only (flows, AI behavior)

Prerequisites on a fresh machine: `npm install`, `npx playwright install chromium`, and
ffmpeg on PATH. The whole run takes a couple of minutes; the recording step is the slow part.

## Verify before reporting success

A run can exit 0 and still produce a broken video. An unhandled mock command returns
`undefined` rather than throwing, which renders as an empty pane instead of an error — so
trust your eyes, not the exit code.

1. Confirm the three files in `docs/` have fresh mtimes.
2. Check dimensions: screenshots 800px wide, MP4 1280×800.
3. Pull a few frames from the MP4 and **actually look at them**:
   ```bash
   ffmpeg -y -ss 6 -i docs/skim-demo.mp4 -frames:v 1 /tmp/f6.png    # inbox + AI summary
   ffmpeg -y -ss 13 -i docs/skim-demo.mp4 -frames:v 1 /tmp/f13.png  # command palette answer
   ffmpeg -y -ss 22 -i docs/skim-demo.mp4 -frames:v 1 /tmp/f22.png  # AI-drafted reply
   ```
   Each pane should be populated and the AI text present.
4. View both screenshots — check the dark one especially (email bodies render in a
   sandboxed iframe with their own theming logic and can go dark-on-dark).

Report which files changed and what you saw. If a scene looks wrong, fix it rather than
shipping it.

## When the tour breaks after a UI change

The harness leans on two contracts. The failure message tells you which one moved:

**IPC command names** (`demo/mock/tauri-core.ts`). The mock implements `invoke()` with a
`switch` over command names mirroring `src/lib/api.ts`. If a command was renamed or added,
add the matching `case`. Symptom: an empty pane or missing data rather than a crash.

**Selectors and English labels** (`demo/record.mjs`). The tour clicks things like
`.row`, `.ai-btn:has-text("Summarize")`, `.ai-input .instruction`. Renamed CSS classes or
changed strings in `src/lib/i18n/locales/en.json` break the matching step, and the timeout
error names the selector that failed. The demo forces `locale: "en"`, so English labels are
what matter.

All fixture data and every scripted AI response live in `demo/mock/data.ts` — that's the
file to edit when you want the demo to *show* something different (different emails, a
different summary, different drafted reply).

Non-obvious things that will bite you when editing `record.mjs`:

- The composer body is a `<textarea>`. Its streamed content is its **value**, not its text —
  wait with `waitValue`, not `waitText`.
- `.compose-window textarea` matches the AI instruction box too (it comes first in the DOM).
  Use `.compose-window > textarea` for the body.
- Compose scenes navigate by hash and must call `page.reload()` afterwards — a hash-only
  `goto` doesn't remount the SPA, so the composer never appears.
- Save the video with `video.saveAs()` *before* `browser.close()`, or it fails with
  "target closed".

## Pacing and themes

Defaults are tuned for a readable landing video. Override per run with env vars (all ms):
`DEMO_READ` (dwell per screen), `DEMO_BEAT` (between actions), `DEMO_TYPE` (keystroke delay),
and `DEMO_AI_TYPING` / `DEMO_AI_THINK` / `DEMO_AI_STEP` (AI stream cadence). Useful when a
scene feels rushed after you add content to it.

Themes are two-axis — `"<cold|warm>-<light|dark>"`, e.g. `warm-light` (the app default).
Both scripts force the theme through `localStorage` (`skimdemo.theme`), which the mock
returns from `get_settings`:

- The video is pinned to `warm-light`; override with `DEMO_THEME=cold-dark npm run demo:record`.
- Screenshots render `warm-light` → `docs/skim-light.jpg` and `warm-dark` → `docs/skim-dark.jpg`
  (the `SHOTS` array in `demo/screenshots.mjs` maps theme → filename).

Watch out for legacy single-axis values: `"light"` and `"dark"` still parse, but map onto the
**cold** palette, so passing them silently renders the wrong temperature rather than failing.

`DEMO_PREBUILT=1` serves a static build from `demo/dist-demo` instead of the dev server —
handy in constrained or headless environments.

More detail lives in `demo/README.md`.
