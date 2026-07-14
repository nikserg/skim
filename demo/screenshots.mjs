// Interface screenshots for the landing page.
//
// Renders the real app (mocked data) in both themes and writes:
//   docs/skim-light.jpg
//   docs/skim-dark.jpg
//
//   node demo/screenshots.mjs                 # dev server (default)
//   DEMO_PREBUILT=1 node demo/screenshots.mjs # serve demo/dist-demo statically
//
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { spawnSync } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";
import { mkdirSync, existsSync, readFileSync, writeFileSync, rmSync } from "node:fs";
import { createServer } from "node:http";

const DIR = dirname(fileURLToPath(import.meta.url));
const ROOT = resolve(DIR, "..");
const DOCS = resolve(ROOT, "docs");
const TMP = resolve(DIR, "output");
const PORT = 1421;
const BASE = `http://localhost:${PORT}`;
// Capture the three-pane layout at 3:2, then downscale to OUT_W wide (aspect kept).
const SIZE = { width: 1600, height: 1068 };
const OUT_W = 800;

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

async function shoot(browser, theme) {
  const context = await browser.newContext({ viewport: SIZE, deviceScaleFactor: 2 });
  await context.addInitScript((t) => { try { localStorage.setItem("skimdemo.theme", t); } catch {} }, theme);
  const page = await context.newPage();
  await page.goto(BASE + "/");
  await page.locator(".row", { hasText: "Q3 launch" }).first().waitFor({ timeout: 15000 });
  // Open the hero thread so the reading pane is populated (three-pane look).
  await page.locator('.row:has-text("Q3 launch")').first().click();
  await page.locator(".subject", { hasText: "Q3 launch" }).first().waitFor();
  await sleep(700); // let fonts + layout settle

  const rawPng = resolve(TMP, `_shot-${theme}.png`);
  await page.screenshot({ path: rawPng }); // 2x → 3200×2136
  await context.close();

  // Downscale to the marketing size and write JPEG.
  const jpg = resolve(DOCS, `skim-${theme}.jpg`);
  const r = spawnSync("ffmpeg", ["-y", "-hide_banner", "-loglevel", "error",
    "-i", rawPng, "-vf", `scale=${OUT_W}:-2:flags=lanczos`, "-q:v", "3", jpg], { stdio: "inherit" });
  if (r.status !== 0) throw new Error("ffmpeg failed for " + theme);
  rmSync(rawPng, { force: true });
  return jpg;
}

async function main() {
  mkdirSync(DOCS, { recursive: true });
  mkdirSync(TMP, { recursive: true });
  const server = await startServer();
  const browser = await chromium.launch();
  const written = [];
  try {
    for (const theme of ["light", "dark"]) written.push(await shoot(browser, theme));
  } finally {
    await browser.close();
    server.kill("SIGTERM");
    await sleep(200);
  }
  console.log("\n✓ Screenshots updated:");
  for (const f of written) console.log("  " + f);
}

main().catch((e) => { console.error(e); process.exit(1); });
