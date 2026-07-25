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

That action can only *update* an existing package, so the first version was
submitted by hand — [PR #407689][bootstrap-pr], schema 1.12.0, three manifests
under `manifests/n/nikserg/Skim/1.0.9/`. Nothing to repeat; from here the
workflow does it. Note that WinGet spells the Tauri installer
`InstallerType: nullsoft`, not `nsis`, and that `ProductCode: Skim` is the ARP
key the app registers under — both are what let WinGet see an upgrade rather
than a fresh install.

The fork of `microsoft/winget-pkgs` under `nikserg` already exists; the action
needs it to push its branch.

Community review of the PR takes anywhere from minutes to a day, so
`winget install nikserg.Skim` trails a fresh release slightly. That's expected.

## The `RELEASE_TOKEN` secret

Both jobs run as the maintainer rather than as `github-actions[bot]`:

- **winget-releaser** pushes to the `winget-pkgs` fork and opens a PR, which
  the stock `GITHUB_TOKEN` cannot reach — it is scoped to this repository. The
  action wants a **classic** PAT with `public_repo`; fine-grained tokens are
  not supported.
- **The Scoop job** pushes the refreshed manifest straight to `main`, which is
  protected by a required pull request. On a user-owned repository GitHub has
  no way to exempt `github-actions[bot]` from that rule — both classic
  protection (`bypass_pull_request_allowances`) and rulesets reject the Actions
  app outside an organization. The maintainer *is* exempt, because
  `enforce_admins` is off, so the checkout uses the same token.

One classic PAT with `public_repo` covers both. Rotate it and the only thing
that breaks is packaging — nothing in the app itself depends on it.

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
[bootstrap-pr]: https://github.com/microsoft/winget-pkgs/pull/407689
