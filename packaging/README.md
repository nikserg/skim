# Packaging

Skim is published to two Windows package managers. Both ship the NSIS
installer (`Skim_x.y.z_x64-setup.exe`) — the same per-user `.exe` the website
links to. Every release, the `winget` and `scoop` jobs in
[`.github/workflows/release.yml`](../.github/workflows/release.yml) update
them; nothing here needs touching by hand.

## WinGet — `nikserg.Skim`

Manifests live in [microsoft/winget-pkgs][winget-pkgs]; on each tag,
[winget-releaser][winget-releaser] opens a PR that bumps the version, URL, and
hashes there.

That action updates an existing package, so it needs a first version submitted
by hand — one time, already done:

```powershell
winget install wingetcreate
wingetcreate new https://github.com/nikserg/skim/releases/download/v1.0.9/Skim_1.0.9_x64-setup.exe
```

Two prerequisites for the job:

- a fork of `microsoft/winget-pkgs` under `nikserg`;
- a repository secret `WINGET_TOKEN` — a **classic** PAT with the `public_repo`
  scope (fine-grained tokens are not supported).

Community review of the PR takes anywhere from minutes to a day, so
`winget install Skim` trails a fresh release slightly. That's expected.

## Scoop — the `bucket/` folder in this repo

Rather than a second repository, this repo *is* the bucket:

```powershell
scoop bucket add skim https://github.com/nikserg/skim
scoop install skim/skim
```

[`bucket/skim.json`](../bucket/skim.json) pins an exact URL and SHA-256, so
the release job downloads the asset, hashes it, and commits the manifest back
to `main`. Two details worth knowing before editing it:

- The `#/dl.7z` URL fragment is what tells Scoop to unpack the NSIS installer
  with 7-Zip instead of dropping the installer itself into the app folder. The
  archive holds a single self-contained `skim.exe` plus 7-Zip's `$PLUGINSDIR`
  scratch folder, which `post_install` removes.
- `post_install` also drops a `.managed-updates` marker next to the exe. The
  app checks for it at startup and keeps its own updater quiet — otherwise the
  in-app banner would run the NSIS installer and leave a second copy under
  `%LOCALAPPDATA%\Skim`. `scoop update skim` is the update path instead.

Uninstalling runs the same registry cleanup as `NSIS_HOOK_POSTUNINSTALL` in
[`src-tauri/windows/hooks.nsh`](../src-tauri/windows/hooks.nsh) — keep the two
in sync. Mail, settings, and credentials live outside the app folder
(`%APPDATA%`, Credential Manager) and are untouched by either.

[winget-pkgs]: https://github.com/microsoft/winget-pkgs
[winget-releaser]: https://github.com/vedantmgoyal9/winget-releaser
