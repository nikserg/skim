// Encode the recorded WebM into the landing-ready MP4 and publish it to docs/.
//
//   node demo/encode.mjs
//
// Produces demo/output/skim-demo.mp4 (H.264, faststart) and copies it to
// <repo>/docs/skim-demo.mp4 so it's ready to ship with the landing page.

import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";
import { existsSync, statSync, mkdirSync, copyFileSync } from "node:fs";

const DIR = dirname(fileURLToPath(import.meta.url));
const ROOT = resolve(DIR, "..");
const OUT = resolve(DIR, "output");
const RAW = resolve(OUT, "raw.webm");
const DOCS = resolve(ROOT, "docs");

if (!existsSync(RAW)) {
  console.error("No recording found at", RAW, "\nRun `node demo/record.mjs` first.");
  process.exit(1);
}

function ff(args, label) {
  console.log("\n▶", label);
  const r = spawnSync("ffmpeg", ["-y", "-hide_banner", "-loglevel", "error", ...args], {
    stdio: "inherit",
  });
  if (r.status !== 0) {
    console.error("ffmpeg failed:", label);
    process.exit(1);
  }
}

const mp4 = resolve(OUT, "skim-demo.mp4");

// MP4 — the landing asset.
ff(
  ["-i", RAW, "-vf", "scale=1280:800:flags=lanczos,fps=30", "-c:v", "libx264",
   "-crf", "20", "-preset", "veryfast", "-pix_fmt", "yuv420p", "-movflags", "+faststart", "-an", mp4],
  "MP4 (H.264)",
);

// Publish to docs/ so it ships with the site.
mkdirSync(DOCS, { recursive: true });
const published = resolve(DOCS, "skim-demo.mp4");
copyFileSync(mp4, published);

const mb = (p) => (statSync(p).size / 1e6).toFixed(2) + " MB";
console.log("\n✓ Done:");
console.log("  " + mp4 + "  (" + mb(mp4) + ")");
console.log("  → published to " + published);
console.log(
  "\nEmbed on the landing with:\n" +
    '  <video autoplay muted loop playsinline>\n' +
    '    <source src="skim-demo.mp4" type="video/mp4">\n' +
    "  </video>",
);
