# Contributing to Skim

Thanks for considering a contribution! Skim is a minimalist project on purpose — the best patches are small, focused, and keep the app fast.

## Development setup

Prerequisites (Windows):

- [Node.js 20+](https://nodejs.org)
- [Rust (stable, MSVC)](https://rustup.rs)
- Visual Studio Build Tools with the **Desktop development with C++** workload

```powershell
npm install
npm run tauri dev    # run the app with hot reload
```

Before opening a PR, make sure the checks CI runs are green locally:

```powershell
npm run check                                   # svelte-check + tsc
cargo fmt --check   --manifest-path src-tauri/Cargo.toml
cargo clippy --all-targets --manifest-path src-tauri/Cargo.toml -- -D warnings
cargo test          --manifest-path src-tauri/Cargo.toml
```

## Project shape

- `src/` — Svelte 5 frontend. No UI kits; design tokens live in `src/styles/tokens.css`. The violet accent (`--accent`) is reserved **exclusively** for AI features.
- `src-tauri/src/` — Rust core: `mail/` (IMAP/SMTP/sync/sanitize), `db/` (SQLite + FTS5), `ai/` (Anthropic client), `commands/` (the IPC surface).
- Every mutation goes through the offline op queue (`pending_ops`); the UI updates optimistically and the server catches up.

## Translations

Locale files live in `src/lib/i18n/locales/*.json`. English (`en.json`) is the source of truth; other languages were written by the maintainers and welcome native-speaker review. Plural keys use the `key_one` / `key_few` / `key_many` / `key_other` convention selected via `Intl.PluralRules`.

To fix or improve a translation, edit the JSON and open a PR — no code changes needed.

## Scope

Skim intentionally leaves things out: multi-account UI, PGP, calendars, contacts, filters/rules, snooze. If a feature idea grows the surface area, open an issue to discuss before writing code. Bug fixes, performance work, and polish are always welcome.

## License

By contributing you agree that your contributions are licensed under the [MIT License](LICENSE).
