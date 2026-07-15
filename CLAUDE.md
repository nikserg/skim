# CLAUDE.md

Guidance for AI agents (and humans) working on Skim. Read this before writing code.
Skim is a **minimalist native email client** for Windows (Rust + Tauri 2 core, Svelte 5 UI).

## Product principles — the lens for every change

These are not style preferences; they are the reason the project exists. When a change
conflicts with one of them, the change is wrong, not the principle. If a request seems to
require breaking one, say so and propose an alternative that doesn't.

### 1. Minimalism — it's an email client, not everything

Skim does one thing: work with mail. We deliberately cut extra features and integrations to
stay focused. Before adding anything, ask "does this serve reading/writing/organizing mail?"
If it grows the surface area (calendar, contacts, rules/filters, snooze, PGP, multi-account
UI, plugins…), it does **not** belong here — open an issue to discuss before writing code.
Fewer buttons, less code, less to maintain. The smallest change that solves the problem wins.

### 2. Contextuality — think *for* the user, not *at* them

We relentlessly reduce cognitive load. That means:
- **Don't offer choice for its own sake.** No long menus, no menus-inside-menus, no settings
  for things we can decide well ourselves. There is no menu bar — surface the *right* action
  at the *right* moment instead.
- **Hide UI when it isn't needed, reveal it when it is.** Buttons appear only when useful.
- **Predict intent and act on the user's behalf** where the answer is obvious (autoconfig,
  optimistic actions, smart defaults). Spend our effort so the user spends less of theirs.
- **Be obvious and predictable.** The app should never surprise. If behavior isn't the most
  expected one, it's a bug.

### 3. Speed, lightness, resource-efficiency

Skim is meant to run all day, every day. Guard that:
- Keep the binary and installer small (native WebView2, **no** bundled browser / Electron).
- Keep the UI instantly responsive — optimistic updates, no jank, no spinners where a local
  cache can answer.
- Respect the machine's CPU, RAM, and battery. Prefer doing less work to doing work faster.
  Watch for polling loops, needless re-renders, and holding data in memory that SQLite already has.

## Practical notes

- **Dev setup, build, and the exact CI checks** live in [CONTRIBUTING.md](CONTRIBUTING.md).
  Run them green locally before finishing: `npm run check`, `cargo fmt --check`,
  `cargo clippy … -D warnings`, `cargo test` (all against `src-tauri/Cargo.toml`).
- **Where things live:** `src/` = Svelte 5 UI (no UI kits; design tokens in
  `src/styles/tokens.css`); `src-tauri/src/` = Rust core — `mail/` (IMAP/SMTP/sync/sanitize),
  `db/` (SQLite + FTS5), `ai/` (Anthropic/OpenRouter client), `commands/` (the IPC surface).
- **The violet accent (`--accent`) is reserved *exclusively* for AI features.** Never use it
  for ordinary UI.
- **Every mutation is offline-first.** Archive/delete/star/send apply locally at once and go
  through the op queue (`pending_ops`); the UI updates optimistically and the server catches
  up with retries. New mutations must follow this pattern, not call the server directly.
- **HTML mail is untrusted.** It is sanitized in Rust (ammonia) and rendered in a no-script
  sandboxed iframe under a strict CSP. Don't loosen this.
- **Secrets never touch the DB or config files.** Passwords, OAuth tokens, and the API key
  live in Windows Credential Manager.
- **i18n:** user-facing strings go through the i18n layer; `en.json` is the source of truth.
  See [Translations](CONTRIBUTING.md#translations).

When in doubt, re-read the three principles above.
