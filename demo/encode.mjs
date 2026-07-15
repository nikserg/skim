// Encode the recorded WebM into the landing-ready MP4 and publish it to docs/.
//
//   node demo/encode.mjs
//
// Produces demo/output/skim-demo.mp4 (H.264, faststart) and copies it to
// <repo>/docs/skim-demo.mp4 so it's ready to ship with the landing page.

import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";
import { existsSync, statSync, mkdirSync, copyFileSync, readdirSync, rmSync } from "node:fs";

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

/** Find where the UI actually appears, so the video can open on it.
 *
 *  Recording starts before the app has painted, so every take begins with some
 *  blank page. The length varies with machine and cache state, and it can't be
 *  inferred from wall-clock time in the recorder: the video doesn't start
 *  rolling exactly when the page is created, so that overshoots and cuts into
 *  the demo. Measuring the file itself is the only honest answer.
 *
 *  A blank frame is flat colour and compresses to a few hundred bytes; a frame
 *  of real UI never does (tens of KB). That gap is wide enough that PNG size is
 *  a reliable signal, and it needs nothing beyond ffmpeg's most basic features.
 *  Threshold is relative to the video's own peak, so it holds for any theme.
 */
function detectContentStart(file) {
  const FPS = 10; // fine enough that the residual blank is imperceptible
  const scanDir = resolve(OUT, "_scan");
  rmSync(scanDir, { recursive: true, force: true });
  mkdirSync(scanDir, { recursive: true });
  const r = spawnSync("ffmpeg", ["-y", "-hide_banner", "-loglevel", "error", "-i", file,
    "-vf", `fps=${FPS},scale=320:-1`, resolve(scanDir, "%04d.png")]);
  let sizes = [];
  if (r.status === 0) {
    sizes = readdirSync(scanDir)
      .filter((f) => f.endsWith(".png"))
      .sort()
      .map((f) => statSync(resolve(scanDir, f)).size);
  }
  rmSync(scanDir, { recursive: true, force: true });
  if (sizes.length === 0) {
    console.warn("! Couldn't scan the recording for a lead-in; encoding it whole.");
    return 0;
  }
  const peak = Math.max(...sizes);
  const threshold = peak * 0.35;
  const idx = sizes.findIndex((s) => s >= threshold);
  // Always report what the scan saw: when this misfires, the frame sizes are
  // the only thing that explains why, and re-running the scan is cheap.
  console.log(
    `Lead-in scan: content at ${(idx / FPS).toFixed(2)}s ` +
      `(peak ${peak} B, threshold ${Math.round(threshold)} B)\n` +
      `  first frames: ${sizes.slice(0, 12).join(", ")}`,
  );
  if (idx <= 0) return 0;
  // Cut exactly at the first content frame. No pre-roll: everything before this
  // sample is blank by definition, so "a beat of safety" would just hand those
  // blank frames back — and frame 0 is what a poster-less <video> shows.
  return idx / FPS;
}

const mp4 = resolve(OUT, "skim-demo.mp4");

// Trim the blank lead-in: a looping landing video can't open on a white flash,
// and that frame would be the poster too.
const trim = detectContentStart(RAW);
const seek = trim > 0.05 ? ["-ss", String(trim)] : [];
if (seek.length) console.log(`Trimming ${trim.toFixed(1)}s of blank lead-in.`);

// MP4 — the landing asset. -ss before -i seeks the input, which is fine here
// since we re-encode anyway.
ff(
  [...seek, "-i", RAW, "-vf", "scale=1280:800:flags=lanczos,fps=30", "-c:v", "libx264",
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
