/** Minimal, safe renderer for the markdown subset AI answers actually use:
 *  **bold**, *italic*, `code`, "- " bullets, "#" headings, "|" pipe tables,
 *  and bare/linked URLs. The input is HTML-escaped before any tags are added,
 *  so the output is safe for `{@html}`. Links open in the system browser — the
 *  container must intercept clicks on `a.md-link` and route the href through
 *  the opener (use the `aiLinks` action from lib/ai-links), never letting the
 *  webview navigate. Style the container with the global `md-body` class. */
export function mdLite(text: string): string {
  const escaped = text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
  // Autolink http(s)/www URLs and schemeless domains that carry a path (the
  // path requirement keeps "config.json" or "e.g." from becoming links).
  // Runs before the emphasis passes: URLs don't contain ** * or `, so the
  // anchors they produce are left untouched by them.
  const linkify = (s: string) =>
    s.replace(
      /(?:https?:\/\/|www\.)[^\s<]+|(?:[a-z0-9](?:[a-z0-9-]*[a-z0-9])?\.)+[a-z]{2,}\/[^\s<]+/gi,
      (m) => {
        // Trailing punctuation usually belongs to the sentence, not the URL.
        const trail = m.match(/[.,;:!?)\]}'"]+$/)?.[0] ?? "";
        const url = trail ? m.slice(0, -trail.length) : m;
        const href = /^https?:\/\//i.test(url) ? url : `https://${url}`;
        return `<a class="md-link" href="${href}" target="_blank" rel="noreferrer noopener">${url}</a>${trail}`;
      },
    );
  // Each bold run gets a cycling highlighter class (hl1..hl3). Warm ("quiet
  // zine") themes render these as a highlighter mark via CSS; cold themes leave
  // them as plain bold. The counter is call-local so identical text always
  // numbers the same, and it continues across lines within one render.
  let hl = 0;
  const inline = (s: string) =>
    linkify(s)
      .replace(/\*\*([^*\n]+)\*\*/g, (_m, b) => `<strong class="hl${(hl++ % 3) + 1}">${b}</strong>`)
      .replace(/(^|[^*])\*([^*\n]+)\*(?!\*)/g, "$1<em>$2</em>")
      .replace(/`([^`\n]+)`/g, "<code>$1</code>");
  // Pipe tables: a "| a | b |" header, a "|---|---|" separator, then body rows.
  // Split the cells on unescaped "|", drop the outer edges, and inline-render
  // each cell so bold/links/etc. work inside them.
  const cells = (row: string) =>
    row
      .trim()
      .replace(/^\||\|$/g, "")
      .split("|")
      .map((c) => c.trim());
  const isRow = (l: string) => /\S/.test(l) && l.includes("|") && /^\s*\|.*\|\s*$/.test(l.trim());
  const isSep = (l: string) => /^\s*\|?[-:\s|]*-[-:\s|]*\|?\s*$/.test(l) && l.includes("|");

  const lines = escaped.split("\n");
  const out: string[] = [];
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    // A table needs a header row immediately followed by a separator row.
    if (isRow(line) && i + 1 < lines.length && isSep(lines[i + 1])) {
      const head = cells(line).map((c) => `<th>${inline(c)}</th>`).join("");
      const body: string[] = [];
      i += 2;
      while (i < lines.length && isRow(lines[i]) && !isSep(lines[i])) {
        body.push(`<tr>${cells(lines[i]).map((c) => `<td>${inline(c)}</td>`).join("")}</tr>`);
        i++;
      }
      i--; // step back: the for-loop's i++ will re-advance past the last row
      out.push(
        `<table class="md-table"><thead><tr>${head}</tr></thead><tbody>${body.join("")}</tbody></table>`,
      );
      continue;
    }
    const bullet = line.match(/^\s*[-–•*] +(.*)$/);
    if (bullet) out.push(`<div class="md-li">${inline(bullet[1])}</div>`);
    else {
      const heading = line.match(/^\s*#{1,4} +(.*)$/);
      if (heading) out.push(`<div class="md-h">${inline(heading[1])}</div>`);
      else if (line.trim() === "") out.push(`<div class="md-gap"></div>`);
      else out.push(`<div class="md-p">${inline(line)}</div>`);
    }
  }
  return out.join("");
}
