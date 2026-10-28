#!/usr/bin/env node
/*
 * Follow-up pass after deslop-pass.mjs: the previous pass turned "###
 * Betterbird — Thunderbird minus the bugs" into "### Betterbird. Thunderbird
 * minus the bugs" (period + space) which reads like two sentences inside a
 * heading. This pass restores a comma inside H2/H3 lines only.
 */
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const BLOG = path.resolve(__dirname, "..", "site", "blog");

function fixLine(line) {
  // Repair earlier bad run that produced "### 2, Skim, …" (list marker lost)
  const repaired = line.replace(/^(#{2,3}) (\d+), /, "$1 $2. ");

  // ### 1. Betterbird. Thunderbird minus the bugs
  //   → ### 1. Betterbird, Thunderbird minus the bugs
  // Keep the "1." list marker, replace the second "." with ",".
  const numbered = repaired.match(/^(#{2,3} \d+\. [^.]+)\. (.+)$/);
  if (numbered) return `${numbered[1]}, ${numbered[2]}`;

  // Guard: continue with repaired variant below
  line = repaired;

  // ### eM Client. Outlook's shape, free up to two accounts
  //   → ### eM Client, Outlook's shape, free up to two accounts
  // Only when there is no "N." list-marker at the start.
  if (!/^#{2,3} \d+\./.test(line)) {
    const unnumbered = line.match(/^(#{2,3} [^.]+)\. (.+)$/);
    if (unnumbered) return `${unnumbered[1]}, ${unnumbered[2]}`;
  }

  return line;
}

function run() {
  const slugs = fs.readdirSync(BLOG).filter(d =>
    fs.statSync(path.join(BLOG, d)).isDirectory());
  let changed = 0;
  for (const slug of slugs) {
    const dir = path.join(BLOG, slug);
    for (const f of fs.readdirSync(dir)) {
      if (!/\.md$/.test(f)) continue;
      const full = path.join(dir, f);
      const src = fs.readFileSync(full, "utf8");
      const next = src.split(/\r?\n/).map(fixLine).join("\n");
      if (next !== src) {
        fs.writeFileSync(full, next, "utf8");
        changed++;
      }
    }
  }
  console.log(`${changed} file(s) with heading fixes.`);
}

run();
