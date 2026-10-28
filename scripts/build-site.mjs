#!/usr/bin/env node
/*
 * Skim site build script.
 *
 * Pre-renders the landing page in all 11 languages (so Google indexes each
 * translation on its own URL), and builds the blog from Markdown sources.
 *
 * Zero npm dependencies — the Markdown and front-matter parsers below are
 * small and good enough for our content.
 *
 * Output layout under docs/ (which GitHub Pages serves):
 *   /                    English landing
 *   /<lang>/             Pre-rendered landing in <lang>, canonical self
 *   /blog/               English blog index
 *   /<lang>/blog/        Localized blog index
 *   /blog/<slug>/        English article
 *   /<lang>/blog/<slug>/ Localized article (falls back to EN if missing)
 */

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import vm from "node:vm";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(__dirname, "..");
const DOCS = path.join(ROOT, "docs");
const SITE = path.join(ROOT, "site");
const BLOG_SRC = path.join(SITE, "blog");

const LANGS = ["en", "ru", "sr", "fr", "de", "es", "it", "pl", "zh", "ja", "ko"];
const BASE_URL = "https://skim-tech.com";

/* ------------------------------------------------------------------
 * Load translations: landing strings from docs/assets/locales.js and
 * blog/UI strings from site/i18n-extra.json. Both are keyed by lang code.
 * ------------------------------------------------------------------ */

function loadI18N() {
  const localesSrc = fs.readFileSync(
    path.join(DOCS, "assets", "locales.js"),
    "utf8",
  );
  const sandbox = { window: {} };
  vm.createContext(sandbox);
  vm.runInContext(localesSrc, sandbox);
  const landing = sandbox.window.I18N;

  const extra = JSON.parse(
    fs.readFileSync(path.join(SITE, "i18n-extra.json"), "utf8"),
  );
  // Merge: for each lang, extra keys win if present
  const merged = {};
  for (const lang of LANGS) {
    merged[lang] = { ...landing.en, ...(landing[lang] || {}),
                     ...(extra.en || {}), ...(extra[lang] || {}) };
  }
  return merged;
}

/* ------------------------------------------------------------------
 * Tiny front-matter + Markdown. Both are enough for our in-house use.
 * ------------------------------------------------------------------ */

function parseFrontMatter(src) {
  const m = src.match(/^---\r?\n([\s\S]+?)\r?\n---\r?\n([\s\S]*)$/);
  if (!m) return { data: {}, body: src };
  const data = {};
  for (const line of m[1].split(/\r?\n/)) {
    const kv = line.match(/^([a-zA-Z0-9_-]+):\s*(.*)$/);
    if (!kv) continue;
    let val = kv[2].trim();
    if (val.startsWith("[") && val.endsWith("]")) {
      val = val.slice(1, -1).split(",").map(s => s.trim().replace(/^["']|["']$/g, "")).filter(Boolean);
    } else {
      val = val.replace(/^["']|["']$/g, "");
    }
    data[kv[1]] = val;
  }
  return { data, body: m[2] };
}

function escapeHTML(s) {
  return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;")
          .replace(/"/g, "&quot;").replace(/'/g, "&#39;");
}
function escapeAttr(s) { return escapeHTML(String(s)); }

/* Markdown: enough for headings, paragraphs, bold/italic, code, lists,
   links, tables, blockquotes, hr. Not a conforming CommonMark parser. */
function renderMarkdown(md) {
  const lines = md.replace(/\r\n?/g, "\n").split("\n");
  let out = [];
  let i = 0;

  function flushParagraph(buf) {
    if (!buf.length) return;
    out.push("<p>" + inline(buf.join(" ")) + "</p>");
  }

  function inline(s) {
    // Escape, then rebuild inline markdown
    let x = s
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;");
    // code `x` first so it's not eaten by other rules
    x = x.replace(/`([^`\n]+)`/g, (_, c) => "<code>" + c + "</code>");
    // links [text](url)
    x = x.replace(/\[([^\]]+)\]\(([^)]+)\)/g,
      (_, t, u) => `<a href="${u.replace(/"/g, "&quot;")}">${t}</a>`);
    // bold **x** and italic *x*
    x = x.replace(/\*\*([^*\n]+)\*\*/g, "<strong>$1</strong>");
    x = x.replace(/(^|[^*])\*([^*\n]+)\*(?!\*)/g, "$1<em>$2</em>");
    return x;
  }

  function renderTable(startIdx) {
    // Markdown pipe table: header | header\n--- | ---\n row | row
    const headerLine = lines[startIdx];
    const sepLine = lines[startIdx + 1];
    if (!/^\s*\|?.*\|.*\|?\s*$/.test(headerLine)) return null;
    if (!/^\s*\|?[\s:-]+\|[\s:|-]+\s*$/.test(sepLine)) return null;
    const splitRow = r => r.replace(/^\s*\|/, "").replace(/\|\s*$/, "")
                           .split("|").map(c => c.trim());
    const headers = splitRow(headerLine);
    const rows = [];
    let j = startIdx + 2;
    while (j < lines.length && /\|/.test(lines[j]) && lines[j].trim() !== "") {
      rows.push(splitRow(lines[j]));
      j++;
    }
    const thead = "<tr>" + headers.map(h => `<th>${inline(h)}</th>`).join("") + "</tr>";
    const tbody = rows.map(r =>
      "<tr>" + r.map(c => `<td>${inline(c)}</td>`).join("") + "</tr>"
    ).join("");
    return { html: `<div class="table-wrap"><table><thead>${thead}</thead><tbody>${tbody}</tbody></table></div>`, consumed: j - startIdx };
  }

  let para = [];
  while (i < lines.length) {
    const line = lines[i];

    // Blank line → flush paragraph
    if (/^\s*$/.test(line)) {
      flushParagraph(para); para = [];
      i++; continue;
    }
    // ATX heading
    const h = line.match(/^(#{1,6})\s+(.*)$/);
    if (h) {
      flushParagraph(para); para = [];
      const level = h[1].length;
      const text = h[2].trim();
      const id = text.toLowerCase().replace(/[^\w\s-]/g, "").trim().replace(/\s+/g, "-");
      out.push(`<h${level} id="${id}">${inline(text)}</h${level}>`);
      i++; continue;
    }
    // Horizontal rule
    if (/^\s*(---|\*\*\*|___)\s*$/.test(line)) {
      flushParagraph(para); para = [];
      out.push("<hr>");
      i++; continue;
    }
    // Blockquote
    if (/^>\s?/.test(line)) {
      flushParagraph(para); para = [];
      const buf = [];
      while (i < lines.length && /^>\s?/.test(lines[i])) {
        buf.push(lines[i].replace(/^>\s?/, ""));
        i++;
      }
      out.push("<blockquote>" + inline(buf.join(" ")) + "</blockquote>");
      continue;
    }
    // Table
    if (/\|/.test(line) && i + 1 < lines.length &&
        /^\s*\|?[\s:-]+\|[\s:|-]+\s*$/.test(lines[i + 1])) {
      flushParagraph(para); para = [];
      const t = renderTable(i);
      if (t) { out.push(t.html); i += t.consumed; continue; }
    }
    // Unordered list
    if (/^\s*[-*+]\s+/.test(line)) {
      flushParagraph(para); para = [];
      const items = [];
      while (i < lines.length && /^\s*[-*+]\s+/.test(lines[i])) {
        items.push(lines[i].replace(/^\s*[-*+]\s+/, ""));
        i++;
      }
      out.push("<ul>" + items.map(it => "<li>" + inline(it) + "</li>").join("") + "</ul>");
      continue;
    }
    // Ordered list
    if (/^\s*\d+\.\s+/.test(line)) {
      flushParagraph(para); para = [];
      const items = [];
      while (i < lines.length && /^\s*\d+\.\s+/.test(lines[i])) {
        items.push(lines[i].replace(/^\s*\d+\.\s+/, ""));
        i++;
      }
      out.push("<ol>" + items.map(it => "<li>" + inline(it) + "</li>").join("") + "</ol>");
      continue;
    }
    // Fenced code
    if (/^```/.test(line)) {
      flushParagraph(para); para = [];
      const buf = [];
      i++;
      while (i < lines.length && !/^```/.test(lines[i])) { buf.push(lines[i]); i++; }
      i++;
      out.push("<pre><code>" + buf.map(l => l.replace(/&/g,"&amp;").replace(/</g,"&lt;")).join("\n") + "</code></pre>");
      continue;
    }
    // Default: paragraph line
    para.push(line);
    i++;
  }
  flushParagraph(para);
  return out.join("\n");
}

function extractHeadings(md) {
  const headings = [];
  for (const line of md.split(/\r?\n/)) {
    const m = line.match(/^(#{2,3})\s+(.*)$/);
    if (!m) continue;
    const level = m[1].length;
    const text = m[2].trim();
    const id = text.toLowerCase().replace(/[^\w\s-]/g, "").trim().replace(/\s+/g, "-");
    headings.push({ level, text, id });
  }
  return headings;
}

function wordCount(md) {
  // Strip front matter already done
  return md.replace(/[`*#>|_[\]()!-]/g, " ").split(/\s+/).filter(Boolean).length;
}

/* ------------------------------------------------------------------
 * URL + path helpers
 * ------------------------------------------------------------------ */

function urlPathFor(lang, pathAfterLang) {
  // "en" → "/<pathAfterLang>", others → "/<lang>/<pathAfterLang>"
  const p = pathAfterLang === "" ? "/" : "/" + pathAfterLang.replace(/^\/+/, "");
  if (lang === "en") return p;
  const q = p === "/" ? `/${lang}/` : `/${lang}${p}`;
  return q;
}
function fsPathFor(lang, pathAfterLang) {
  const dir = lang === "en"
    ? path.join(DOCS, pathAfterLang)
    : path.join(DOCS, lang, pathAfterLang);
  return dir;
}
function ensureDir(d) { fs.mkdirSync(d, { recursive: true }); }
function writeFile(filePath, content) {
  ensureDir(path.dirname(filePath));
  fs.writeFileSync(filePath, content, "utf8");
}

/* ------------------------------------------------------------------
 * HREFLANG block builder
 * ------------------------------------------------------------------ */

function hreflangBlock({ pathAfterLang, availableLangs }) {
  // pathAfterLang: e.g. "", "blog/", "blog/best-free-email-client-for-windows-11/"
  const links = [];
  const enURL = BASE_URL + urlPathFor("en", pathAfterLang);
  links.push(`<link rel="alternate" hreflang="x-default" href="${enURL}">`);
  for (const l of LANGS) {
    if (!availableLangs.includes(l)) continue;
    const u = BASE_URL + urlPathFor(l, pathAfterLang);
    links.push(`<link rel="alternate" hreflang="${l}" href="${u}">`);
  }
  return links.join("\n  ");
}

/* ------------------------------------------------------------------
 * LANDING page builder: take docs/index.html as a template, swap text
 * via data-i18n / data-i18n-html, inject hreflang + <html lang>, write.
 * ------------------------------------------------------------------ */

function renderBlogTeaserCards(lang, articles, i18n) {
  if (!articles.length) return "";
  const dict = i18n[lang];
  const topN = articles.slice(0, 3);
  return topN.map(a => {
    const useLang = a.byLang[lang] ? lang : "en";
    const d = a.byLang[useLang].data;
    return `
        <a class="blog-scrap" href="${articleHref(useLang, a.slug)}">
          <div class="tape tape-pink"></div>
          <h3>${escapeHTML(d.title)}</h3>
          <p>${escapeHTML(d.description || "")}</p>
          <span class="blog-scrap-read">${escapeHTML(dict.blog_read_more || "Read →")}</span>
        </a>`;
  }).join("");
}

function buildLandingPages(i18n, articles) {
  const tplPath = path.join(SITE, "template.html");
  const tpl = fs.readFileSync(tplPath, "utf8");

  for (const lang of LANGS) {
    const dict = i18n[lang];
    const enDict = i18n.en;

    let html = tpl;

    // 1. <html lang="en"> → current lang
    html = html.replace(/<html\s+lang="[^"]*"/, `<html lang="${lang}"`);

    // 2. <title>
    html = html.replace(/<title>[\s\S]*?<\/title>/,
      `<title>${escapeHTML(dict.page_title_landing || enDict.page_title_landing)}</title>`);

    // 3. <meta name="description" content="…">
    html = html.replace(
      /<meta\s+name="description"\s+content="[^"]*"\s*>/,
      `<meta name="description" content="${escapeAttr(dict.page_desc_landing || enDict.page_desc_landing)}">`
    );

    // 4. og:title / og:description / twitter mirrors
    html = html.replace(
      /<meta\s+property="og:title"\s+content="[^"]*"\s*>/,
      `<meta property="og:title" content="${escapeAttr(dict.page_title_landing || enDict.page_title_landing)}">`
    );
    html = html.replace(
      /<meta\s+property="og:description"\s+content="[^"]*"\s*>/,
      `<meta property="og:description" content="${escapeAttr(dict.page_desc_landing || enDict.page_desc_landing)}">`
    );
    html = html.replace(
      /<meta\s+property="og:url"\s+content="[^"]*"\s*>/,
      `<meta property="og:url" content="${BASE_URL + urlPathFor(lang, '')}">`
    );
    html = html.replace(
      /<meta\s+property="og:locale"\s+content="[^"]*"\s*>/,
      `<meta property="og:locale" content="${dict.og_locale || 'en_US'}">`
    );
    html = html.replace(
      /<meta\s+name="twitter:title"\s+content="[^"]*"\s*>/,
      `<meta name="twitter:title" content="${escapeAttr(dict.page_title_landing || enDict.page_title_landing)}">`
    );
    html = html.replace(
      /<meta\s+name="twitter:description"\s+content="[^"]*"\s*>/,
      `<meta name="twitter:description" content="${escapeAttr(dict.page_desc_landing || enDict.page_desc_landing)}">`
    );

    // 5. canonical
    html = html.replace(
      /<link\s+rel="canonical"\s+href="[^"]*"\s*>/,
      `<link rel="canonical" href="${BASE_URL + urlPathFor(lang, '')}">`
    );

    // 6. inject hreflang block right after canonical
    const hreflang = hreflangBlock({ pathAfterLang: "", availableLangs: LANGS });
    html = html.replace(
      /(<link\s+rel="canonical"[^>]*>)/,
      `$1\n  ${hreflang}`
    );

    // 7. swap data-i18n and data-i18n-html inner content
    html = html.replace(
      /<([a-z0-9]+)([^>]*?)\sdata-i18n="([a-z0-9_]+)"([^>]*?)>([\s\S]*?)<\/\1>/gi,
      (full, tag, pre, key, post, inner) => {
        const v = dict[key] != null ? dict[key] : enDict[key];
        if (v == null) return full;
        return `<${tag}${pre} data-i18n="${key}"${post}>${escapeHTML(v)}</${tag}>`;
      }
    );
    html = html.replace(
      /<([a-z0-9]+)([^>]*?)\sdata-i18n-html="([a-z0-9_]+)"([^>]*?)>([\s\S]*?)<\/\1>/gi,
      (full, tag, pre, key, post, inner) => {
        const v = dict[key] != null ? dict[key] : enDict[key];
        if (v == null) return full;
        return `<${tag}${pre} data-i18n-html="${key}"${post}>${v}</${tag}>`;
      }
    );

    // 8. nav Blog link injection — placeholder marker in template
    const blogHref = urlPathFor(lang, "blog/");
    html = html.replace(/\{\{nav_blog_href\}\}/g, blogHref);
    html = html.replace(/\{\{nav_blog_label\}\}/g, escapeHTML(dict.nav_blog || "Blog"));

    // 8a. root-relative asset paths — the landing lives under /<lang>/ for
    // non-English, where the template's relative hrefs (`assets/zine.css`,
    // `skim-light.jpg`, `stickers/*.webp`, `privacy.html`, …) would resolve
    // to `/<lang>/assets/…` and 404. Rewrite anything that isn't absolute,
    // scheme-prefixed, anchor-only, or data: to start at the site root.
    const landingRoot = urlPathFor(lang, "");
    html = html.replace(/\s(href|src|poster)="\.\/"/g,
      (_, attr) => ` ${attr}="${landingRoot}"`);
    html = html.replace(
      /\s(href|src|poster)="(?!https?:|\/\/|\/|#|data:|mailto:|\{\{)([^"]+)"/g,
      (_, attr, p) => ` ${attr}="/${p}"`
    );

    // 8b. blog teaser block on the landing
    html = html.replace(/\{\{landing_blog_kicker\}\}/g,
      escapeHTML(dict.landing_blog_kicker || enDict.landing_blog_kicker || ""));
    html = html.replace(/\{\{landing_blog_h2\}\}/g,
      escapeHTML(dict.landing_blog_h2 || enDict.landing_blog_h2 || ""));
    html = html.replace(/\{\{blog_teaser_cards\}\}/g,
      renderBlogTeaserCards(lang, articles, i18n));

    // 9. Write
    const out = fsPathFor(lang, "index.html");
    writeFile(out, html);
    console.log("wrote", path.relative(ROOT, out));
  }
}

/* ------------------------------------------------------------------
 * BLOG builder
 * ------------------------------------------------------------------ */

function loadBlogArticles() {
  if (!fs.existsSync(BLOG_SRC)) return [];
  const slugs = fs.readdirSync(BLOG_SRC)
    .filter(d => fs.statSync(path.join(BLOG_SRC, d)).isDirectory());
  const articles = [];
  for (const slug of slugs) {
    const dir = path.join(BLOG_SRC, slug);
    const byLang = {};
    for (const lang of LANGS) {
      const file = path.join(dir, `${lang}.md`);
      if (!fs.existsSync(file)) continue;
      const raw = fs.readFileSync(file, "utf8");
      const { data, body } = parseFrontMatter(raw);
      byLang[lang] = { data, body };
    }
    if (!byLang.en) {
      console.warn(`skipping ${slug}: no en.md`);
      continue;
    }
    articles.push({ slug, byLang });
  }
  // Sort by EN front matter 'date' desc
  articles.sort((a, b) => (b.byLang.en.data.date || "").localeCompare(a.byLang.en.data.date || ""));
  return articles;
}

function articleHref(lang, slug) {
  return urlPathFor(lang, `blog/${slug}/`);
}

function renderArticleHTML({ article, lang, i18n, allArticles }) {
  const enDict = i18n.en;
  const dict = i18n[lang];
  const { data, body } = article.byLang[lang] || article.byLang.en;
  const translated = !article.byLang[lang] && lang !== "en";
  const availableLangs = Object.keys(article.byLang);
  const title = data.title || "Untitled";
  const description = data.description || "";
  const datePublished = data.date || "2026-10-28";
  const dateUpdated = data.updated || datePublished;
  const image = data.image || `${BASE_URL}/social-preview.jpg`;
  const bodyHTML = renderMarkdown(body);
  const words = wordCount(body);
  const minRead = Math.max(1, Math.round(words / 230));
  const headings = extractHeadings(body).filter(h => h.level === 2);
  const toc = headings.length >= 3
    ? `<nav class="toc" aria-label="Table of contents"><ol>${
        headings.map(h => `<li><a href="#${h.id}">${escapeHTML(h.text)}</a></li>`).join("")
      }</ol></nav>` : "";

  const canonical = BASE_URL + articleHref(lang, article.slug);
  const hreflang = hreflangBlock({
    pathAfterLang: `blog/${article.slug}/`,
    availableLangs: availableLangs,
  });

  // Related
  let related = Array.isArray(data.related) ? data.related : [];
  const relatedHTML = related.length ? `
    <section class="related">
      <h2>${escapeHTML(dict.blog_related)}</h2>
      <ul>${related.map(slug => {
        const other = allArticles.find(a => a.slug === slug);
        if (!other) return "";
        const otherLang = other.byLang[lang] ? lang : "en";
        const otherData = other.byLang[otherLang].data;
        return `<li><a href="${articleHref(otherLang, slug)}">${escapeHTML(otherData.title)}</a></li>`;
      }).join("")}</ul>
    </section>` : "";

  const translatedBanner = translated
    ? `<div class="translated-banner">${escapeHTML(dict.blog_translated_from_en)}</div>`
    : "";

  const jsonLd = {
    "@context": "https://schema.org",
    "@type": "BlogPosting",
    "headline": title,
    "description": description,
    "datePublished": datePublished,
    "dateModified": dateUpdated,
    "author": { "@type": "Organization", "name": "Skim contributors", "url": "https://github.com/nikserg/skim" },
    "publisher": { "@type": "Organization", "name": "Skim", "logo": { "@type": "ImageObject", "url": `${BASE_URL}/social-preview.jpg` } },
    "image": image,
    "mainEntityOfPage": canonical,
    "inLanguage": lang
  };
  const breadcrumbLd = {
    "@context": "https://schema.org",
    "@type": "BreadcrumbList",
    "itemListElement": [
      { "@type": "ListItem", "position": 1, "name": "Home", "item": BASE_URL + urlPathFor(lang, "") },
      { "@type": "ListItem", "position": 2, "name": dict.nav_blog, "item": BASE_URL + urlPathFor(lang, "blog/") },
      { "@type": "ListItem", "position": 3, "name": title, "item": canonical }
    ]
  };

  const langSwitchOptions = LANGS.map(l => {
    const label = {
      en: "English", ru: "Русский", sr: "Srpski", fr: "Français",
      de: "Deutsch", es: "Español", it: "Italiano", pl: "Polski",
      zh: "中文", ja: "日本語", ko: "한국어"
    }[l];
    const selected = l === lang ? " selected" : "";
    const href = articleHref(l, article.slug);
    // If no translation exists for `l`, fall back to EN URL (never hide — Google can crawl)
    const fallbackHref = article.byLang[l] ? href : articleHref("en", article.slug);
    return `<option value="${fallbackHref}"${selected}>${label}</option>`;
  }).join("");

  return `<!doctype html>
<html lang="${lang}">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>${escapeHTML(title)} — Skim Blog</title>
  <meta name="description" content="${escapeAttr(description)}">
  <link rel="canonical" href="${canonical}">
  ${hreflang}

  <meta property="og:type" content="article">
  <meta property="og:site_name" content="Skim">
  <meta property="og:title" content="${escapeAttr(title)}">
  <meta property="og:description" content="${escapeAttr(description)}">
  <meta property="og:url" content="${canonical}">
  <meta property="og:image" content="${image}">
  <meta property="og:locale" content="${dict.og_locale || 'en_US'}">
  <meta name="twitter:card" content="summary_large_image">
  <meta name="twitter:title" content="${escapeAttr(title)}">
  <meta name="twitter:description" content="${escapeAttr(description)}">
  <meta name="twitter:image" content="${image}">

  <link rel="icon" href="data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 96 96'%3E%3Crect width='96' height='96' rx='22' fill='%23eadfc2'/%3E%3Cg fill='%230d0d10'%3E%3Crect x='25' y='39' width='46' height='8.5' rx='4.25'/%3E%3Crect x='25' y='55' width='29' height='8.5' rx='4.25' fill-opacity='0.55'/%3E%3C/g%3E%3C/svg%3E">

  <link rel="preconnect" href="https://fonts.googleapis.com">
  <link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
  <link href="https://fonts.googleapis.com/css2?family=Anton&family=Caveat:wght@500;600;700&family=Oswald:wght@600;700&family=Permanent+Marker&family=Rock+Salt&family=Special+Elite&family=Hanken+Grotesk:wght@400;500;700&family=IBM+Plex+Mono:wght@400;600&display=swap" rel="stylesheet">
  <link href="/assets/zine.css?v=3" rel="stylesheet">
  <link href="/assets/blog.css?v=3" rel="stylesheet">

  <script type="application/ld+json">${JSON.stringify(jsonLd)}</script>
  <script type="application/ld+json">${JSON.stringify(breadcrumbLd)}</script>
</head>
<body class="article-page">

<div class="frame">
  <div class="sheet">
    <div class="grain"></div>
    <div class="holes"></div>

    <header class="masthead">
      <a class="mast-logo" href="${urlPathFor(lang, '')}">Skim</a>
      <nav class="mast-nav">
        <a href="${urlPathFor(lang, '#product')}">${escapeHTML(dict.nav_features || 'Features')}</a>
        <a href="${urlPathFor(lang, '#ai')}">${escapeHTML(dict.nav_ai || 'AI')}</a>
        <a href="${urlPathFor(lang, 'blog/')}" class="current">${escapeHTML(dict.nav_blog)}</a>
        <a href="https://github.com/nikserg/skim">${escapeHTML(dict.nav_github || 'GitHub')}</a>
        <label class="lang" title="Language">
          <svg width="15" height="15" viewBox="0 0 24 24" fill="none" aria-hidden="true"><circle cx="12" cy="12" r="9" stroke="currentColor" stroke-width="1.6"/><path d="M3 12h18M12 3c2.5 2.5 3.8 5.7 3.8 9S14.5 18.5 12 21C9.5 18.5 8.2 15.3 8.2 12S9.5 5.5 12 3z" stroke="currentColor" stroke-width="1.6"/></svg>
          <select id="langSelect" aria-label="Language">${langSwitchOptions}</select>
        </label>
      </nav>
    </header>

    <main class="section">
      <article>
        <nav class="breadcrumbs">
          <a href="${urlPathFor(lang, '')}">Skim</a> / <a href="${urlPathFor(lang, 'blog/')}">${escapeHTML(dict.nav_blog)}</a> / <span>${escapeHTML(title)}</span>
        </nav>

        <h1>${escapeHTML(title)}</h1>
        <p class="dek">${escapeHTML(description)}</p>
        <div class="meta">
          <time datetime="${datePublished}">${escapeHTML(dict.blog_published)} ${datePublished}</time>
          · ${minRead} ${escapeHTML(dict.blog_min_read)}
        </div>
        ${translatedBanner}
        ${toc}

        ${bodyHTML}

        ${relatedHTML}

        <section class="post-cta">
          <h2>${escapeHTML(dict.blog_cta_h)}</h2>
          <p>${escapeHTML(dict.blog_cta_p)}</p>
          <div class="cta-row">
            <a class="btn-zine" href="https://github.com/nikserg/skim/releases/latest">${escapeHTML(dict.blog_cta_dl)}</a>
            <a class="btn-zine ink" href="https://github.com/nikserg/skim">${escapeHTML(dict.blog_cta_gh)}</a>
          </div>
        </section>

        <p class="back-link"><a href="${urlPathFor(lang, 'blog/')}">${escapeHTML(dict.blog_back)}</a></p>
      </article>
    </main>

    <footer class="zine-footer">
      <span>${escapeHTML(i18n[lang].foot_copy || 'MIT © Skim contributors')}</span>
      <span class="foot-links">
        <a href="https://github.com/nikserg/skim">${escapeHTML(dict.foot_source || 'Source')}</a>
        <a href="https://github.com/nikserg/skim/releases">${escapeHTML(dict.foot_releases || 'Releases')}</a>
        <a href="/privacy.html">${escapeHTML(dict.foot_privacy || 'Privacy')}</a>
        <a href="/terms.html">${escapeHTML(dict.foot_terms || 'Terms')}</a>
      </span>
    </footer>
  </div>
</div>

<script src="/assets/locales.js?v=3"></script>
<script src="/assets/app.js?v=3"></script>
</body>
</html>`;
}

function renderBlogIndex({ lang, articles, i18n }) {
  const dict = i18n[lang];
  const canonical = BASE_URL + urlPathFor(lang, "blog/");
  const hreflang = hreflangBlock({ pathAfterLang: "blog/", availableLangs: LANGS });

  const cards = articles.map(a => {
    const useLang = a.byLang[lang] ? lang : "en";
    const d = a.byLang[useLang].data;
    const words = wordCount(a.byLang[useLang].body);
    const minRead = Math.max(1, Math.round(words / 230));
    return `
      <li class="blog-card">
        <a href="${articleHref(useLang, a.slug)}">
          <h2>${escapeHTML(d.title)}</h2>
          <p>${escapeHTML(d.description || "")}</p>
          <div class="meta"><time datetime="${d.date}">${d.date}</time> · ${minRead} ${escapeHTML(dict.blog_min_read)}</div>
        </a>
      </li>`;
  }).join("");

  const langSwitchOptions = LANGS.map(l => {
    const label = {
      en: "English", ru: "Русский", sr: "Srpski", fr: "Français",
      de: "Deutsch", es: "Español", it: "Italiano", pl: "Polski",
      zh: "中文", ja: "日本語", ko: "한국어"
    }[l];
    const selected = l === lang ? " selected" : "";
    return `<option value="${urlPathFor(l, 'blog/')}"${selected}>${label}</option>`;
  }).join("");

  return `<!doctype html>
<html lang="${lang}">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>${escapeHTML(dict.page_title_blog)}</title>
  <meta name="description" content="${escapeAttr(dict.page_desc_blog)}">
  <link rel="canonical" href="${canonical}">
  ${hreflang}
  <meta property="og:type" content="website">
  <meta property="og:title" content="${escapeAttr(dict.page_title_blog)}">
  <meta property="og:description" content="${escapeAttr(dict.page_desc_blog)}">
  <meta property="og:url" content="${canonical}">
  <meta property="og:locale" content="${dict.og_locale || 'en_US'}">
  <link rel="icon" href="data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 96 96'%3E%3Crect width='96' height='96' rx='22' fill='%23eadfc2'/%3E%3Cg fill='%230d0d10'%3E%3Crect x='25' y='39' width='46' height='8.5' rx='4.25'/%3E%3Crect x='25' y='55' width='29' height='8.5' rx='4.25' fill-opacity='0.55'/%3E%3C/g%3E%3C/svg%3E">
  <link rel="preconnect" href="https://fonts.googleapis.com">
  <link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
  <link href="https://fonts.googleapis.com/css2?family=Anton&family=Caveat:wght@500;600;700&family=Oswald:wght@600;700&family=Permanent+Marker&family=Rock+Salt&family=Special+Elite&family=Hanken+Grotesk:wght@400;500;700&family=IBM+Plex+Mono:wght@400;600&display=swap" rel="stylesheet">
  <link href="/assets/zine.css?v=3" rel="stylesheet">
  <link href="/assets/blog.css?v=3" rel="stylesheet">
</head>
<body class="blog-index-page">

<div class="frame">
  <div class="sheet">
    <div class="grain"></div>
    <div class="holes"></div>

    <header class="masthead">
      <a class="mast-logo" href="${urlPathFor(lang, '')}">Skim</a>
      <nav class="mast-nav">
        <a href="${urlPathFor(lang, '#product')}">${escapeHTML(dict.nav_features || 'Features')}</a>
        <a href="${urlPathFor(lang, '#ai')}">${escapeHTML(dict.nav_ai || 'AI')}</a>
        <a href="${urlPathFor(lang, 'blog/')}" class="current">${escapeHTML(dict.nav_blog)}</a>
        <a href="https://github.com/nikserg/skim">${escapeHTML(dict.nav_github || 'GitHub')}</a>
        <label class="lang" title="Language">
          <svg width="15" height="15" viewBox="0 0 24 24" fill="none" aria-hidden="true"><circle cx="12" cy="12" r="9" stroke="currentColor" stroke-width="1.6"/><path d="M3 12h18M12 3c2.5 2.5 3.8 5.7 3.8 9S14.5 18.5 12 21C9.5 18.5 8.2 15.3 8.2 12S9.5 5.5 12 3z" stroke="currentColor" stroke-width="1.6"/></svg>
          <select id="langSelect" aria-label="Language">${langSwitchOptions}</select>
        </label>
      </nav>
    </header>

    <main class="section">
      <section class="blog-header">
        <h1>${escapeHTML(dict.blog_h1)}</h1>
        <p class="lead">${escapeHTML(dict.blog_lead)}</p>
      </section>
      <ul class="blog-list">
        ${cards}
      </ul>
    </main>

    <footer class="zine-footer">
      <span>${escapeHTML(i18n[lang].foot_copy || 'MIT © Skim contributors')}</span>
      <span class="foot-links">
        <a href="https://github.com/nikserg/skim">${escapeHTML(dict.foot_source || 'Source')}</a>
        <a href="https://github.com/nikserg/skim/releases">${escapeHTML(dict.foot_releases || 'Releases')}</a>
        <a href="/privacy.html">${escapeHTML(dict.foot_privacy || 'Privacy')}</a>
        <a href="/terms.html">${escapeHTML(dict.foot_terms || 'Terms')}</a>
      </span>
    </footer>
  </div>
</div>

<script src="/assets/locales.js?v=3"></script>
<script src="/assets/app.js?v=3"></script>
</body>
</html>`;
}

function buildBlog(i18n, articles) {
  for (const lang of LANGS) {
    // Index page
    const idx = renderBlogIndex({ lang, articles, i18n });
    const idxOut = fsPathFor(lang, "blog/index.html");
    writeFile(idxOut, idx);
    console.log("wrote", path.relative(ROOT, idxOut));

    // Articles
    for (const article of articles) {
      const html = renderArticleHTML({ article, lang, i18n, allArticles: articles });
      const out = fsPathFor(lang, `blog/${article.slug}/index.html`);
      writeFile(out, html);
    }
  }
}

/* ------------------------------------------------------------------
 * SITEMAP
 * ------------------------------------------------------------------ */

function buildSitemap(articles) {
  const urls = [];
  // Landing
  urls.push({ path: "", available: LANGS });
  urls.push({ path: "blog/", available: LANGS });
  // Articles — hreflang only for langs that have the translation
  for (const a of articles) {
    urls.push({ path: `blog/${a.slug}/`, available: Object.keys(a.byLang) });
  }

  const xml = [`<?xml version="1.0" encoding="UTF-8"?>`,
`<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9" xmlns:xhtml="http://www.w3.org/1999/xhtml">`];

  for (const u of urls) {
    for (const l of u.available) {
      const loc = BASE_URL + urlPathFor(l, u.path);
      xml.push(`  <url>`);
      xml.push(`    <loc>${loc}</loc>`);
      // Alternates
      xml.push(`    <xhtml:link rel="alternate" hreflang="x-default" href="${BASE_URL + urlPathFor('en', u.path)}"/>`);
      for (const alt of u.available) {
        xml.push(`    <xhtml:link rel="alternate" hreflang="${alt}" href="${BASE_URL + urlPathFor(alt, u.path)}"/>`);
      }
      xml.push(`    <changefreq>${u.path.startsWith('blog/') && u.path !== 'blog/' ? 'monthly' : 'weekly'}</changefreq>`);
      xml.push(`    <priority>${l === 'en' ? '1.0' : '0.8'}</priority>`);
      xml.push(`  </url>`);
    }
  }
  // Keep legal pages
  xml.push(`  <url><loc>${BASE_URL}/privacy.html</loc><changefreq>yearly</changefreq><priority>0.3</priority></url>`);
  xml.push(`  <url><loc>${BASE_URL}/terms.html</loc><changefreq>yearly</changefreq><priority>0.3</priority></url>`);

  xml.push(`</urlset>`);
  writeFile(path.join(DOCS, "sitemap.xml"), xml.join("\n"));
  console.log("wrote", path.relative(ROOT, path.join(DOCS, "sitemap.xml")));
}

/* ------------------------------------------------------------------
 * MAIN
 * ------------------------------------------------------------------ */

function main() {
  const i18n = loadI18N();
  const articles = loadBlogArticles();
  console.log(`i18n languages: ${LANGS.join(', ')}`);
  console.log(`blog articles: ${articles.length}`);
  buildLandingPages(i18n, articles);
  buildBlog(i18n, articles);
  buildSitemap(articles);
  console.log("done.");
}

main();
