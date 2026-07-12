# Skim

> Email at the speed of thought.

A fast, native, minimalist email client for Windows with bring-your-own-key Claude AI. Under active development — full README coming with the first release.

- **Native & fast** — Rust core + WebView2 UI (Tauri 2), tiny installer, instant cold start.
- **Your mail, your machine** — IMAP/SMTP sync into a local SQLite cache; works offline; no proxy servers.
- **Bring your own AI** — paste an Anthropic API key to draft replies, summarize threads, and ask questions across your mailbox. Requests go straight from your machine to Claude.
- **11 languages**, light & dark themes, keyboard-first.

## Building from source

Prerequisites: [Node.js 20+](https://nodejs.org), [Rust (stable, MSVC)](https://rustup.rs), Visual Studio Build Tools with the C++ workload.

```powershell
npm install
npm run tauri dev    # run in development
npm run tauri build  # produce installers
```

## License

[MIT](LICENSE)
