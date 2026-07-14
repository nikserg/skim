// Product demo recorder.
//
// Boots the app in demo mode (mocked Tauri IPC), drives the real UI through a
// scripted tour with a visible cursor, and records the whole thing to WebM.
// Then run `node demo/encode.mjs` to produce the MP4/WebM/GIF for the landing.
//
//   node demo/record.mjs                 # dev server (default)
//   DEMO_PREBUILT=1 node demo/record.mjs # serve demo/dist-demo statically
//
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";
import { mkdir, rm, readdir } from "node:fs/promises";
import { appendFileSync, existsSync, readFileSync } from "node:fs";
import { createServer } from "node:http";

const DIR = dirname(fileURLToPath(import.meta.url));
const ROOT = resolve(DIR, "..");
const OUT = resolve(DIR, "output");
const PORT = 1421;
const BASE = `http://localhost:${PORT}`;
const SIZE = { width: 1280, height: 800 };

// ---- pacing (ms) — readable defaults; override via env for a quick pass ----
const envNum = (k, d) => (Number(process.env[k]) > 0 ? Number(process.env[k]) : d);
const READ = envNum("DEMO_READ", 1400);
const BEAT = envNum("DEMO_BEAT", 650);
const TYPE_DELAY = envNum("DEMO_TYPE", 42);
const AI_TUNABLES = {
  "skimdemo.typingMs": envNum("DEMO_AI_TYPING", 24),
  "skimdemo.thinkMs": envNum("DEMO_AI_THINK", 420),
  "skimdemo.stepMs": envNum("DEMO_AI_STEP", 650),
};

const MIME = {
  ".html": "text/html", ".js": "text/javascript", ".mjs": "text/javascript",
  ".css": "text/css", ".json": "application/json", ".svg": "image/svg+xml",
  ".woff2": "font/woff2", ".woff": "font/woff", ".ttf": "font/ttf",
  ".png": "image/png", ".webp": "image/webp", ".ico": "image/x-icon",
};

async function startServer() {
  if (process.env.DEMO_PREBUILT) {
    const outDir = resolve(DIR, "dist-demo");
    if (!existsSync(resolve(outDir, "index.html")))
      throw new Error("No build at " + outDir + " — run vite build --config demo/vite.demo.config.ts");
    const server = createServer((req, res) => {
      let p = decodeURIComponent((req.url || "/").split("?")[0]);
      if (p === "/" || !p.includes(".")) p = "/index.html";
      const file = resolve(outDir, "." + p);
      if (!file.startsWith(outDir) || !existsSync(file)) { res.writeHead(404).end("not found"); return; }
      const ext = file.slice(file.lastIndexOf("."));
      res.writeHead(200, { "content-type": MIME[ext] || "application/octet-stream" });
      res.end(readFileSync(file));
    });
    await new Promise((r) => server.listen(PORT, r));
    return { kill: () => server.close() };
  }
  const bin = process.platform === "win32" ? "npx.cmd" : "npx";
  const proc = spawn(bin, ["vite", "--config", "demo/vite.demo.config.ts"], {
    cwd: ROOT, stdio: "inherit", shell: process.platform === "win32",
  });
  for (let i = 0; i < 120; i++) {
    try { const r = await fetch(BASE); if (r.ok) return proc; } catch {}
    await sleep(500);
  }
  throw new Error("Demo server did not start on " + BASE);
}

// ---- cursor + click overlay (injected into every page) ------------------
const CURSOR_INIT = () => {
  function ensure() {
    if (document.getElementById("demo-cursor")) return;
    const c = document.createElement("div");
    c.id = "demo-cursor";
    c.style.cssText =
      "position:fixed;left:640px;top:400px;z-index:2147483647;pointer-events:none;" +
      "transform:translate(-3px,-2px);filter:drop-shadow(0 1px 2px rgba(0,0,0,.35));";
    c.innerHTML =
      '<svg width="24" height="24" viewBox="0 0 24 24" fill="none">' +
      '<path d="M5 3l14 7-6 1.5L10 18 5 3z" fill="#111" stroke="#fff" stroke-width="1.3" stroke-linejoin="round"/></svg>';
    document.documentElement.appendChild(c);
    window.__cursor = c;
  }
  if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", ensure);
  else ensure();
  window.__moveCursor = (x, y, dur) => new Promise((res) => {
    ensure();
    const c = window.__cursor;
    const sx = parseFloat(c.style.left) || 640, sy = parseFloat(c.style.top) || 400;
    const t0 = performance.now();
    const ease = (t) => (t < 0.5 ? 2 * t * t : 1 - Math.pow(-2 * t + 2, 2) / 2);
    const frame = (t) => {
      const k = Math.min(1, (t - t0) / dur), e = ease(k);
      c.style.left = sx + (x - sx) * e + "px";
      c.style.top = sy + (y - sy) * e + "px";
      if (k < 1) requestAnimationFrame(frame); else res();
    };
    requestAnimationFrame(frame);
  });
  window.__clickPulse = (x, y) => {
    ensure();
    const p = document.createElement("div");
    p.style.cssText =
      "position:fixed;left:" + x + "px;top:" + y + "px;width:12px;height:12px;" +
      "border:2px solid #6b46f2;border-radius:50%;z-index:2147483646;pointer-events:none;" +
      "transform:translate(-50%,-50%);opacity:.9;transition:all .45s ease-out;";
    document.documentElement.appendChild(p);
    requestAnimationFrame(() => { p.style.width = "40px"; p.style.height = "40px"; p.style.opacity = "0"; });
    setTimeout(() => p.remove(), 480);
  };
};

function makeDriver(page) {
  async function moveTo(x, y, dur = 520) {
    await page.evaluate(([x, y, d]) => window.__moveCursor(x, y, d), [x, y, dur]);
    await page.mouse.move(x, y);
  }
  async function center(sel) {
    const el = page.locator(sel).first();
    await el.waitFor({ state: "visible", timeout: 15000 });
    await el.scrollIntoViewIfNeeded();
    const b = await el.boundingBox();
    return { el, x: b.x + b.width / 2, y: b.y + b.height / 2 };
  }
  async function click(sel, { dur = 520 } = {}) {
    const { el, x, y } = await center(sel);
    await moveTo(x, y, dur);
    await sleep(120);
    await page.evaluate(([x, y]) => window.__clickPulse(x, y), [x, y]);
    await el.click();
    await sleep(BEAT);
  }
  async function type(sel, text, { delay = TYPE_DELAY } = {}) {
    const { el, x, y } = await center(sel);
    await moveTo(x, y, 420);
    await page.evaluate(([x, y]) => window.__clickPulse(x, y), [x, y]);
    await el.click();
    await sleep(220);
    await el.pressSequentially(text, { delay });
  }
  async function waitText(sel, substring, timeout = 20000) {
    await page.locator(sel, { hasText: substring }).first().waitFor({ state: "visible", timeout });
  }
  async function waitValue(sel, substring, timeout = 20000) {
    await page.waitForFunction(
      ([s, sub]) => { const el = document.querySelector(s); return el && String(el.value || "").includes(sub); },
      [sel, substring], { timeout },
    );
  }
  return { moveTo, click, type, waitText, waitValue, page };
}

const T0 = Date.now();
function mark(s) { try { appendFileSync(resolve(OUT, "progress.log"), `+${Date.now() - T0}ms ${s}\n`); } catch {} }

async function tour(d) {
  const { page } = d;

  // Scene 0 — land on the inbox.
  mark("tour-start");
  await page.goto(BASE + "/");
  await page.locator(".row", { hasText: "Q3 launch" }).first().waitFor({ timeout: 15000 });
  mark("inbox-loaded");
  await sleep(READ);

  // Scene 1 — Summarize a long thread.
  await d.click('.row:has-text("Q3 launch")');
  await page.locator(".subject", { hasText: "Q3 launch" }).first().waitFor();
  await sleep(READ);
  await d.click('.ai-btn:has-text("Summarize")');
  await d.waitText(".ai-card .ai-text", "sync");
  await sleep(READ + 900);
  await d.click('.ai-dock .dock-btn[aria-label="Close"]');
  await sleep(BEAT);
  mark("scene1-summarize-done");

  // Scene 2 — Search + ask across the mailbox (command palette).
  await d.click(".sidebar .search");
  await page.locator(".panel .input-row input").waitFor();
  await sleep(BEAT);
  await d.type(".panel .input-row input", "who owns the launch landing page?");
  await sleep(700);
  await page.keyboard.press("Enter");
  await d.waitText(".chat-answer .chat-text", "Thursday");
  await page.locator(".source-chip").first().waitFor();
  await sleep(READ + 900);
  await d.click(".source-chip");
  await page.locator(".subject", { hasText: "Q3 launch" }).first().waitFor();
  await sleep(READ);
  mark("scene2-search-done");

  // Scene 3 — Draft a reply with AI (compose window).
  await page.goto(BASE + "/#/compose/7001");
  await page.reload();
  await page.locator(".ai-input .instruction").waitFor({ timeout: 15000 });
  await sleep(BEAT);
  await d.type(".ai-input .instruction", "Confirm Thursday works, promise the landing copy by Wednesday, keep it warm");
  await sleep(500);
  await d.click('.ai-input button:has-text("Draft")');
  await d.waitValue(".compose-window > textarea", "Best,");
  await sleep(READ + 1200);
  mark("scene3-reply-done");

  // Scene 4 — Compose a brand-new email with AI.
  await page.goto(BASE + "/#/compose/7002");
  await page.reload();
  await page.locator(".ai-input .instruction").waitFor({ timeout: 15000 });
  await sleep(BEAT);
  await d.type(".ai-input .instruction", "Invite the team to a 30-minute onboarding sync on Friday at 2pm");
  await sleep(500);
  await d.click('.ai-input button:has-text("Draft")');
  await page.locator(".subject-row input").waitFor();
  await d.waitValue(".compose-window > textarea", "Thanks,");
  await sleep(READ + 1600);
  mark("scene4-compose-done");
}

async function main() {
  await rm(OUT, { recursive: true, force: true });
  await mkdir(resolve(OUT, "video"), { recursive: true });
  const server = await startServer();
  const browser = await chromium.launch();
  const context = await browser.newContext({
    viewport: SIZE, deviceScaleFactor: 2,
    recordVideo: { dir: resolve(OUT, "video"), size: SIZE },
  });
  await context.addInitScript(CURSOR_INIT);
  await context.addInitScript((t) => {
    try { for (const [k, v] of Object.entries(t)) localStorage.setItem(k, String(v)); } catch {}
  }, AI_TUNABLES);
  const page = await context.newPage();
  const d = makeDriver(page);
  let failed = null;
  try { await tour(d); } catch (e) { failed = e; console.error("Tour failed:", e.message); }
  const video = page.video();
  await context.close();
  if (video) await video.saveAs(resolve(OUT, "raw.webm"));
  await browser.close();
  try { for (const f of await readdir(resolve(OUT, "video"))) await rm(resolve(OUT, "video", f), { force: true }); } catch {}
  server.kill("SIGTERM");
  await sleep(300);
  if (failed) process.exit(1);
  console.log("\n✓ Recorded:", resolve(OUT, "raw.webm"));
  console.log("  Next: node demo/encode.mjs");
}

main().catch((e) => { console.error(e); process.exit(1); });
