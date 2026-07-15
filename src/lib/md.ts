/** Minimal, safe renderer for the markdown subset AI answers actually use:
 *  **bold**, *italic*, `code`, "- " bullets, and "#" headings. The input is
 *  HTML-escaped before any tags are added, so the output is safe for
 *  `{@html}`. Style the container with the global `md-body` class. */
export function mdLite(text: string): string {
  const escaped = text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
  // Each bold run gets a cycling highlighter class (hl1..hl3). Warm ("quiet
  // zine") themes render these as a highlighter mark via CSS; cold themes leave
  // them as plain bold. The counter is call-local so identical text always
  // numbers the same, and it continues across lines within one render.
  let hl = 0;
  const inline = (s: string) =>
    s
      .replace(/\*\*([^*\n]+)\*\*/g, (_m, b) => `<strong class="hl${(hl++ % 3) + 1}">${b}</strong>`)
      .replace(/(^|[^*])\*([^*\n]+)\*(?!\*)/g, "$1<em>$2</em>")
      .replace(/`([^`\n]+)`/g, "<code>$1</code>");
  return escaped
    .split("\n")
    .map((line) => {
      const bullet = line.match(/^\s*[-–•*] +(.*)$/);
      if (bullet) return `<div class="md-li">${inline(bullet[1])}</div>`;
      const heading = line.match(/^\s*#{1,4} +(.*)$/);
      if (heading) return `<div class="md-h">${inline(heading[1])}</div>`;
      if (line.trim() === "") return `<div class="md-gap"></div>`;
      return `<div class="md-p">${inline(line)}</div>`;
    })
    .join("");
}
