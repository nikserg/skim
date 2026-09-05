# Packaging

Skim is published to Scoop, which ships the NSIS installer
(`Skim_x.y.z_x64-setup.exe`) — the same per-user `.exe` the website links to.
Every release, the `scoop` job in
[`.github/workflows/release.yml`](../.github/workflows/release.yml) updates it;
nothing here needs touching by hand.

There used to be a WinGet job too, publishing `nikserg.Skim` via
[winget-releaser][winget-releaser]. That action can only *update* a package
that already exists in [microsoft/winget-pkgs][winget-pkgs], and the one-time
bootstrap submission never landed there — so the job failed on every single
tag and painted the release run red for no reason. It has been removed. Adding
WinGet back means getting a version accepted upstream by hand first.

## The `RELEASE_TOKEN` secret

The Scoop job runs as the maintainer rather than as `github-actions[bot]`: it
pushes the refreshed manifest straight to `main`, which is protected by a
required pull request. On a user-owned repository GitHub has no way to exempt
`github-actions[bot]` from that rule — both classic protection
(`bypass_pull_request_allowances`) and rulesets reject the Actions app outside
an organization. The maintainer *is* exempt, because `enforce_admins` is off,
so the checkout uses that token instead.

A classic PAT with `public_repo` covers it. Rotate it and the only thing that
breaks is packaging — nothing in the app itself depends on it.

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
