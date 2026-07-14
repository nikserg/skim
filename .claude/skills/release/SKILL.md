---
name: release
description: >
  Ship a new Skim release. Commits and pushes any pending changes, bumps the
  version across all manifests, tags it, and lets CI build & publish the GitHub
  Release — then rewrites the release notes in a bright, informal, ironic style
  with emoji. Use when the user says /release, "cut a release", "ship it",
  "выпусти релиз", or asks to publish a new version of Skim.
allowed-tools: Bash(git:*), Bash(gh:*), Bash(cargo:*), Read, Edit, Write, Grep, Glob
---

# Release Skim

Cut and publish a new Skim release end-to-end. Skim ships via a `v*` git tag →
`.github/workflows/release.yml` (tauri-action) builds NSIS+MSI and creates the
GitHub Release. Your job is to get a clean tag pushed and then dress the release
up with great notes.

## Before you start

- Confirm you're in `C:\skim` on the `main` branch: `git rev-parse --abbrev-ref HEAD`.
- Confirm `gh` is authed: `gh auth status` (needed to edit release notes later).
- Read the current version from `src-tauri/tauri.conf.json` (`"version"`). It must
  match `package.json` and `src-tauri/Cargo.toml`.

## Step 1 — Commit & push pending work

1. `git status --short`. If the tree is clean, skip to Step 2.
2. If any **Rust** files (`src-tauri/**/*.rs`) changed, run `cargo fmt` first —
   CI has failed on formatting/clippy before, so never skip this.
   (Add cargo to PATH if needed: `$env:Path += ";$env:USERPROFILE\.cargo\bin"`.)
3. Read the diff (`git diff` / `git diff --staged`) enough to understand it, then
   stage everything (`git add -A`) and commit with a real conventional-commit
   message that summarizes the change — **not** a generic "wip". End the message with:
   ```
   Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>
   ```
4. `git push origin main`.

## Step 2 — Bump the version

Default to a **patch** bump (0.1.5 → 0.1.6). Honor an explicit request instead:
`/release minor`, `/release major`, or an exact version like `/release 0.2.0`.

Edit the version string in **all** of these with the Edit tool — one exact match each:

- `package.json` → `"version": "X.Y.Z"`
- `src-tauri/tauri.conf.json` → `"version": "X.Y.Z"`
- `src-tauri/Cargo.toml` → `version = "X.Y.Z"` (the `[package]` one, first in file)
- `src-tauri/Cargo.lock` → the `name = "skim"` entry's `version = "X.Y.Z"`

⚠️ **Never** write these files with PowerShell `Set-Content -Encoding utf8` — it
adds a BOM that breaks tauri-action. Always use the **Edit** tool.

Commit the bump and push:
```
git add -A
git commit -m "chore: release vX.Y.Z"
git push origin main
```

## Step 3 — Tag & trigger the build

```
git tag vX.Y.Z
git push origin vX.Y.Z
```
Pushing the tag is what triggers the release workflow. A re-run of a failed run
does **not** pick up new permissions or a new commit — if something's wrong you
bump the patch again and push a fresh tag, you don't retag.

## Step 4 — Write the release notes

While CI builds (~14 min), draft the notes. Base them on the real commits since
the previous tag:
```
git log <previous-tag>..vX.Y.Z --oneline
```
Turn that into notes with **personality**: informal, a little ironic/self-deprecating,
generous with emoji, grouped by theme (✨ new, 🐛 fixes, 🧹 under the hood). Lead
with a punchy one-liner. Keep it truthful to what actually changed — humor dresses
the facts, it doesn't invent them. Write the notes to a file, e.g.
`<scratchpad>/release-notes.md`.

Style reference (tone, not a template):
> ## Skim vX.Y.Z — "<cheeky codename>" 🎉
> The one where notifications finally do what you click on them. Wild concept, we know.
>
> ### ✨ New
> - **skim:// links** — click a toast, land on the actual email. Revolutionary. 🪄
>
> ### 🐛 Fixed
> - Inbox no longer naps for 5 minutes when Gmail rudely drops the IDLE connection. ⚡
>
> ### 🧹 Housekeeping
> - Some Rust got `cargo fmt`'d into submission.

## Step 5 — Wait for the build & publish the notes

You must wait for the whole build to finish here — the local reinstall in Step 6
needs the published installer.

1. Find and watch the run to completion (this blocks ~14 min — run it in the
   background so you're notified when it exits):
   ```
   gh run watch $(gh run list --workflow=release.yml --limit 1 --json databaseId --jq '.[0].databaseId') --exit-status
   ```
2. When it succeeds, the release exists (`gh release view vX.Y.Z` succeeds) with
   the NSIS/MSI installers attached. Overwrite the empty body with your notes:
   `gh release edit vX.Y.Z --notes-file <scratchpad>/release-notes.md`

If CI fails, read the logs (`gh run view --log-failed`), fix the cause, bump the
patch, and push a new tag — don't retag the same version. Do **not** proceed to
Step 6 on a failed build.

## Step 6 — Silently update the local install

The user runs Skim installed per-user at `C:\Users\nikit\AppData\Local\Skim\skim.exe`
(NSIS). Pull the freshly published installer from the release and reinstall it
silently:

1. Download the NSIS setup for this version into the scratchpad:
   ```
   gh release download vX.Y.Z --pattern "*_x64-setup.exe" --dir <scratchpad> --clobber
   ```
   (The `msi` also exists; use the `nsis` `*-setup.exe` — it's the installed one.)
2. Close the running instance so the exe isn't locked:
   `Get-Process skim -ErrorAction SilentlyContinue | Stop-Process -Force`
3. Silent reinstall (exit 0 = success):
   `Start-Process "<scratchpad>\Skim_X.Y.Z_x64-setup.exe" -ArgumentList "/S" -Wait`
4. Confirm the on-disk version updated — check that `skim.exe` was just rewritten
   (`Get-Item "$env:LOCALAPPDATA\Skim\skim.exe" | Select-Object LastWriteTime`).

Do not auto-launch the app or run any paid AI actions afterward — leave starting
it to the user unless they ask.

## Done

Report back: the new version, the commit(s) you pushed, the tag, the release URL
(`gh release view vX.Y.Z --json url --jq .url`), CI status, and that the local
install was silently updated to the new version.
