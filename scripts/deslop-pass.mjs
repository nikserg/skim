#!/usr/bin/env node
/*
 * One-shot deslop pass over site/blog/<slug>/<lang>.md.
 *
 * - Replaces em-dashes in BODY text with commas or periods, keeping dashes
 *   inside front-matter title/description where they read as subtitle
 *   separators and are standard SEO formatting.
 * - Patches a handful of specific AI tells ("quietly rot", "ecosystem" used
 *   loosely, "it's worth noting", etc.).
 */
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const BLOG = path.resolve(__dirname, "..", "site", "blog");

const EM = "—";          // em-dash
const EM_SP = " — ";     // " — "

function splitFM(src) {
  const m = src.match(/^(---\r?\n[\s\S]+?\r?\n---\r?\n)([\s\S]*)$/);
  if (!m) return { fm: "", body: src };
  return { fm: m[1], body: m[2] };
}

// Replace " — " in body. Case: upper-case next word → ". " (new sentence),
// lower-case → ", " (continuation).
function stripEmDashes(body) {
  return body.replace(/ — (\S)/g, (_, next) => {
    if (/[A-ZА-ЯЁ0-9]/u.test(next)) return ". " + next;
    return ", " + next;
  }).replace(/—/g, "-"); // any remaining stray em-dash → hyphen
}

// Specific small slop fixes applied to body text.
function patchSlop(body) {
  return body
    .replace(/\bquietly rot\b/g, "stop getting updates")
    .replace(/\bquietly changes the deal\b/g, "changes the deal")
    .replace(/\bThunderbird's extension ecosystem\b/g, "Thunderbird's add-on library")
    .replace(/\bit's worth noting that\b/gi, "")
    .replace(/\bin conclusion,?\s*/gi, "")
    .replace(/\bdespite these challenges,?\s*/gi, "");
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
      const { fm, body } = splitFM(src);
      let newBody = patchSlop(stripEmDashes(body));
      // Collapse accidental double spaces from the patches
      newBody = newBody.replace(/[ \t]{2,}/g, " ");
      const next = fm + newBody;
      if (next !== src) {
        fs.writeFileSync(full, next, "utf8");
        changed++;
        console.log("deslopped", path.relative(path.resolve(__dirname, ".."), full));
      }
    }
  }
  console.log(`\n${changed} file(s) updated.`);
}

run();
