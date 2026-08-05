//! Swapping an email's text for its translation without touching its markup.
//!
//! We never move a node, only replace the text inside one, so no HTML tree is
//! needed: a linear scan that remembers the byte range of every text run is
//! enough, and splicing translations back into those ranges leaves tags,
//! attributes, `cid:` images and link hrefs exactly as they were. The rewritten
//! HTML is therefore no more dangerous than the original and still goes through
//! `sanitize` on its way to the pane.
//!
//! Text is grouped into *segments* — one per block-level element — with inline
//! formatting reduced to numbered markers (`Read our <1>latest post</1>`), so
//! the model translates a whole sentence at once and may move the markup as the
//! target language's word order requires. Translating each text node separately
//! would hand it fragments and get two halves of a sentence that don't agree.
//!
//! Nothing here trusts the model's answer: a segment whose markers came back
//! wrong collapses into its first text node, and a segment that never came back
//! keeps the original. The document is always intact.

use crate::mail::parse::decode_entities;
use std::collections::HashMap;
use std::ops::Range;

/// Inline elements: they become markers *inside* a segment. Everything else
/// ends the current segment, which keeps the invariant `apply` relies on —
/// within a segment, two text runs are always separated by a marker.
const INLINE: &[&str] = &[
    "a", "abbr", "b", "bdi", "bdo", "big", "cite", "code", "del", "em", "font", "i", "ins", "mark",
    "q", "s", "small", "span", "strike", "strong", "sub", "sup", "time", "u", "var",
];

/// Elements whose text is not part of the message.
const SKIP: &[&str] = &["script", "style", "title", "head", "noscript"];

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    /// The n-th text run of this block.
    Text(usize),
    Open(u32),
    Close(u32),
}

/// One block-level occurrence: its structure, and where its text lives in the
/// original HTML.
#[derive(Debug)]
struct Block {
    /// Segment number whose translation fills this block.
    id: u32,
    tokens: Vec<Token>,
    spans: Vec<Range<usize>>,
}

/// The translatable content of one email body.
#[derive(Debug, Default)]
pub struct Extracted {
    /// Unique segment texts in document order; the segment number is the index
    /// plus one. Blocks that read identically (newsletter boilerplate repeats a
    /// lot) share one entry, so the model is asked once.
    texts: Vec<String>,
    blocks: Vec<Block>,
    /// Segment number of the subject line, once [`Extracted::add_subject`] has
    /// put it in. No block references it — its translation is stored on its own
    /// rather than spliced into the body.
    subject: Option<u32>,
}

impl Extracted {
    /// How many segments the model is being asked for.
    pub fn len(&self) -> usize {
        self.texts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.texts.is_empty()
    }

    /// The numbered list the model receives.
    pub fn prompt_list(&self) -> String {
        let mut out = String::new();
        for (i, text) in self.texts.iter().enumerate() {
            out.push_str(&format!("[[{}]] {}\n", i + 1, text));
        }
        out
    }

    /// Drop the segments that don't fit the request budget. Returns whether
    /// anything was dropped — their blocks simply keep the original text, so a
    /// long message ends up translated down to a point and intact after it.
    pub fn truncate_to(&mut self, max_chars: usize, max_segments: usize) -> bool {
        let mut kept = 0;
        let mut chars = 0;
        for text in self.texts.iter().take(max_segments) {
            let len = text.chars().count();
            if kept > 0 && chars + len > max_chars {
                break;
            }
            chars += len;
            kept += 1;
        }
        if kept >= self.texts.len() {
            return false;
        }
        self.texts.truncate(kept);
        true
    }

    /// Add the subject line as one more segment. The reading pane shows it above
    /// the body, so leaving it in the original language would make a translated
    /// message look half-done.
    ///
    /// Call this *after* [`Extracted::truncate_to`]: the subject is one short
    /// line and must never be the thing that gets dropped. A subject that
    /// already appears in the body (newsletters like to repeat it as a heading)
    /// shares that segment instead of paying for it twice.
    pub fn add_subject(&mut self, subject: &str) {
        let text = collapse_ws(subject).trim().to_string();
        if text.is_empty() || !text.chars().any(char::is_alphabetic) {
            return;
        }
        let id = match self.texts.iter().position(|t| *t == text) {
            Some(i) => i as u32 + 1,
            None => {
                self.texts.push(text);
                self.texts.len() as u32
            }
        };
        self.subject = Some(id);
    }

    /// Segment number carrying the subject, if one was added.
    pub fn subject_id(&self) -> Option<u32> {
        self.subject
    }
}

/// Find every translatable segment in `html`.
pub fn extract(html: &str) -> Extracted {
    let mut out = Extracted::default();
    let mut ids: HashMap<String, u32> = HashMap::new();
    let mut cur = Builder::default();
    let bytes = html.as_bytes();
    let mut i = 0;
    let mut run_start: Option<usize> = None;
    let mut skip_depth: Option<&str> = None;

    while i < bytes.len() {
        if bytes[i] != b'<' {
            if skip_depth.is_none() && run_start.is_none() {
                run_start = Some(i);
            }
            i += 1;
            continue;
        }

        // A tag (or comment) starts here: the text run before it ends.
        let tag_start = i;
        let tag = read_tag(html, i);
        if let Some(start) = run_start.take() {
            if skip_depth.is_none() {
                cur.push_text(html, start..tag_start);
            }
        }
        i = tag.end;

        match tag.kind {
            // A comment interrupts the text, so it also ends the segment —
            // otherwise the runs on either side would sit side by side with no
            // marker between them and `apply` could not tell them apart.
            TagKind::Comment | TagKind::Bang => cur.flush(&mut out, &mut ids),
            TagKind::Open(name) => {
                // Tags nested inside a skipped element change nothing.
                if skip_depth.is_some() {
                    continue;
                }
                if SKIP.contains(&name.as_str()) {
                    cur.flush(&mut out, &mut ids);
                    skip_depth = Some(SKIP.iter().find(|s| **s == name).copied().unwrap());
                } else if INLINE.contains(&name.as_str()) {
                    // A trailing slash is ignored, exactly as HTML5 ignores it on
                    // a non-void element — and it keeps every pair of text runs in
                    // a segment separated by a marker, which `apply` relies on.
                    cur.open_inline();
                } else {
                    cur.flush(&mut out, &mut ids);
                }
            }
            TagKind::Close(name) => {
                if let Some(skipped) = skip_depth {
                    if skipped == name {
                        skip_depth = None;
                    }
                    continue;
                }
                if INLINE.contains(&name.as_str()) {
                    cur.close_inline();
                } else {
                    cur.flush(&mut out, &mut ids);
                }
            }
        }
    }
    if let Some(start) = run_start {
        if skip_depth.is_none() {
            cur.push_text(html, start..bytes.len());
        }
    }
    cur.flush(&mut out, &mut ids);
    out
}

/// Find every translatable segment in a plain-text body: one per paragraph.
///
/// Hard-wrapped lines inside a paragraph are joined for the model — a paragraph
/// is one unit of meaning, and the pane wraps text itself — but the blank lines
/// between paragraphs are never touched, so the shape of the message survives.
pub fn extract_text(text: &str) -> Extracted {
    let mut out = Extracted::default();
    let mut ids: HashMap<String, u32> = HashMap::new();
    let mut cur = Builder {
        decode: false,
        ..Builder::default()
    };
    for span in paragraphs(text) {
        cur.push_text(text, span);
        cur.flush(&mut out, &mut ids);
    }
    out
}

/// Byte ranges of the paragraphs in `text`, excluding the blank lines between.
fn paragraphs(text: &str) -> Vec<Range<usize>> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\n' {
            let mut j = i + 1;
            while j < bytes.len() && matches!(bytes[j], b' ' | b'\t' | b'\r') {
                j += 1;
            }
            if bytes.get(j) == Some(&b'\n') {
                out.push(start..i);
                start = j + 1;
                i = j + 1;
                continue;
            }
        }
        i += 1;
    }
    if start < bytes.len() {
        out.push(start..bytes.len());
    }
    out
}

/// Splice the translations back in. Untranslated segments keep their original
/// text, so a partial answer still yields a whole document.
pub fn apply(html: &str, extracted: &Extracted, translations: &HashMap<u32, String>) -> String {
    splice(html, extracted, translations, true)
}

/// [`apply`] for a plain-text body, where a `<` is just a `<`.
pub fn apply_text(
    text: &str,
    extracted: &Extracted,
    translations: &HashMap<u32, String>,
) -> String {
    splice(text, extracted, translations, false)
}

/// Read the model's numbered answer. Anything before the first `[[n]]` is
/// ignored, and a reply cut off mid-flight still yields the segments that did
/// arrive — those get translated, the rest keep the original.
pub fn parse_reply(reply: &str) -> HashMap<u32, String> {
    let mut out = HashMap::new();
    let bytes = reply.as_bytes();
    let mut open: Option<(u32, usize)> = None;
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b'[' && bytes[i + 1] == b'[' {
            let digits = i + 2;
            let mut j = digits;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                j += 1;
            }
            if j > digits && reply[j..].starts_with("]]") {
                if let Ok(id) = reply[digits..j].parse::<u32>() {
                    if let Some((prev, from)) = open {
                        out.insert(prev, reply[from..i].trim().to_string());
                    }
                    open = Some((id, j + 2));
                    i = j + 2;
                    continue;
                }
            }
        }
        i += 1;
    }
    if let Some((prev, from)) = open {
        out.insert(prev, reply[from..].trim().to_string());
    }
    out
}

fn splice(
    source: &str,
    extracted: &Extracted,
    translations: &HashMap<u32, String>,
    escape: bool,
) -> String {
    let html = source;
    let mut edits: Vec<(Range<usize>, String)> = Vec::new();
    for block in &extracted.blocks {
        let Some(translated) = translations.get(&block.id) else {
            continue;
        };
        let originals: Vec<&str> = block.spans.iter().map(|s| &html[s.clone()]).collect();
        let pieces = distribute(block, translated, &originals);
        for (span, piece) in block.spans.iter().zip(pieces) {
            let piece = if escape { escape_text(&piece) } else { piece };
            edits.push((span.clone(), piece));
        }
    }
    edits.sort_by_key(|(span, _)| span.start);

    let mut out = String::with_capacity(html.len() + html.len() / 4);
    let mut at = 0;
    for (span, piece) in edits {
        if span.start < at {
            continue; // Overlapping spans can't happen, but never corrupt.
        }
        out.push_str(&html[at..span.start]);
        out.push_str(&piece);
        at = span.end;
    }
    out.push_str(&html[at..]);
    out
}

/// Split one translated segment across the block's text runs.
fn distribute(block: &Block, translated: &str, originals: &[&str]) -> Vec<String> {
    let mut out = vec![String::new(); block.spans.len()];
    let pieces = parse_markers(translated);

    let want: Vec<Token> = block
        .tokens
        .iter()
        .filter(|t| !matches!(t, Token::Text(_)))
        .cloned()
        .collect();
    let got: Vec<Token> = pieces
        .iter()
        .filter_map(|p| match p {
            Piece::Open(k) => Some(Token::Open(*k)),
            Piece::Close(k) => Some(Token::Close(*k)),
            Piece::Literal(_) => None,
        })
        .collect();

    if want != got {
        // The model dropped, renumbered or invented a marker, so we can't tell
        // which run each fragment belongs to. Keep every word by collapsing the
        // whole segment into the first run: the inline structure is lost, the
        // text and the tags are not.
        let flat: String = pieces
            .iter()
            .filter_map(|p| match p {
                Piece::Literal(text) => Some(text.as_str()),
                _ => None,
            })
            .collect();
        if let Some(first) = out.first_mut() {
            *first = flat.trim().to_string();
        }
        return out;
    }

    // Marker sequences agree, so the literal between marker i and i+1 belongs to
    // whichever run sits in that same gap.
    let mut slots: Vec<Option<usize>> = vec![None; want.len() + 1];
    let mut gap = 0;
    for token in &block.tokens {
        match token {
            Token::Text(idx) => slots[gap] = Some(*idx),
            _ => gap += 1,
        }
    }
    let mut literals: Vec<String> = vec![String::new(); want.len() + 1];
    let mut gap = 0;
    for piece in &pieces {
        match piece {
            Piece::Literal(text) => literals[gap].push_str(text),
            _ => gap += 1,
        }
    }

    let mut pending = String::new();
    let mut last: Option<usize> = None;
    for (gap, literal) in literals.iter().enumerate() {
        match slots[gap] {
            Some(idx) => {
                out[idx] = format!("{pending}{literal}");
                pending.clear();
                last = Some(idx);
            }
            // Text the model put where the original had none (a space it added
            // between two links, say) still belongs to the message.
            None if !literal.is_empty() => match last {
                Some(idx) => out[idx].push_str(literal),
                None => pending.push_str(literal),
            },
            None => {}
        }
    }
    if !pending.is_empty() {
        if let Some(first) = slots.iter().flatten().next() {
            out[*first] = format!("{pending}{}", out[*first]);
        }
    }

    for (piece, original) in out.iter_mut().zip(originals) {
        preserve_edges(piece, original);
    }
    out
}

/// Keep the spacing that separated this run from the markup around it: a model
/// that trims its answer would otherwise glue a word to the next link.
fn preserve_edges(piece: &mut String, original: &str) {
    if original.trim().is_empty() {
        // A run that was only whitespace is a separator, not content: keep it
        // byte for byte unless the model actually put words there.
        if piece.trim().is_empty() {
            *piece = original.to_string();
        }
        return;
    }
    if piece.trim().is_empty() {
        // Emptied on purpose — the model moved these words into another gap of
        // the same segment. Restoring the original would duplicate them.
        return;
    }
    if original.starts_with(char::is_whitespace) && !piece.starts_with(char::is_whitespace) {
        piece.insert(0, ' ');
    }
    if original.ends_with(char::is_whitespace) && !piece.ends_with(char::is_whitespace) {
        piece.push(' ');
    }
}

// ---- segment building ------------------------------------------------------

struct Builder {
    text: String,
    tokens: Vec<Token>,
    spans: Vec<Range<usize>>,
    open: Vec<u32>,
    next_marker: u32,
    letters: usize,
    /// HTML runs carry entities; a plain-text body's `&amp;` is literal.
    decode: bool,
}

impl Default for Builder {
    fn default() -> Self {
        Self {
            text: String::new(),
            tokens: Vec::new(),
            spans: Vec::new(),
            open: Vec::new(),
            next_marker: 0,
            letters: 0,
            decode: true,
        }
    }
}

impl Builder {
    fn push_text(&mut self, html: &str, span: Range<usize>) {
        let raw = &html[span.clone()];
        let decoded = if self.decode {
            collapse_ws(&decode_entities(raw))
        } else {
            collapse_ws(raw)
        };
        if decoded.is_empty() {
            return;
        }
        self.letters += decoded.chars().filter(|c| c.is_alphabetic()).count();
        self.tokens.push(Token::Text(self.spans.len()));
        self.spans.push(span);
        self.text.push_str(&decoded);
    }

    fn open_inline(&mut self) {
        self.next_marker += 1;
        let k = self.next_marker;
        self.open.push(k);
        self.tokens.push(Token::Open(k));
        self.text.push_str(&format!("<{k}>"));
    }

    fn close_inline(&mut self) {
        // A stray close (markup that was never opened in this segment) is
        // nothing to represent.
        let Some(k) = self.open.pop() else { return };
        // Nothing came in since the matching open, so the pair wraps nothing.
        // That is what an inline element holding only padding leaves behind now
        // that the padding is stripped: the run collapses away and never becomes
        // a token. A model handed `<2></2>` routinely drops it, `distribute`
        // then falls back to flattening the whole segment into its first run,
        // and every other run is spliced back empty. `k` was the last number
        // minted, so taking it back keeps the numbering dense.
        if self.tokens.last() == Some(&Token::Open(k)) {
            self.tokens.pop();
            self.text.truncate(self.text.len() - format!("<{k}>").len());
            self.next_marker -= 1;
            return;
        }
        self.tokens.push(Token::Close(k));
        self.text.push_str(&format!("</{k}>"));
    }

    fn flush(&mut self, out: &mut Extracted, ids: &mut HashMap<String, u32>) {
        // Balance markup left open by the block boundary, so what the model
        // sees — and what it must send back — is well formed.
        while let Some(k) = self.open.pop() {
            self.tokens.push(Token::Close(k));
            self.text.push_str(&format!("</{k}>"));
        }
        let text = std::mem::take(&mut self.text);
        let tokens = std::mem::take(&mut self.tokens);
        let spans = std::mem::take(&mut self.spans);
        let letters = std::mem::replace(&mut self.letters, 0);
        self.next_marker = 0;

        // Nothing to translate in numbers, prices or bare punctuation.
        if letters == 0 || spans.is_empty() {
            return;
        }
        let text = text.trim().to_string();
        if text.is_empty() {
            return;
        }
        let next = out.texts.len() as u32 + 1;
        let id = *ids.entry(text.clone()).or_insert_with(|| {
            out.texts.push(text);
            next
        });
        out.blocks.push(Block { id, tokens, spans });
    }
}

// ---- tag and marker scanning -----------------------------------------------

enum TagKind {
    Open(String),
    Close(String),
    Comment,
    /// `<!doctype …>` and friends.
    Bang,
}

struct Tag {
    kind: TagKind,
    /// Byte offset just past the tag.
    end: usize,
}

/// Read the tag starting at `at` (which must be a `<`).
fn read_tag(html: &str, at: usize) -> Tag {
    let bytes = html.as_bytes();
    let rest = &html[at..];
    if rest.starts_with("<!--") {
        let end = html[at + 4..]
            .find("-->")
            .map(|p| at + 4 + p + 3)
            .unwrap_or(html.len());
        return Tag {
            kind: TagKind::Comment,
            end,
        };
    }
    let closing = rest.starts_with("</");
    let name_start = at + if closing { 2 } else { 1 };
    if !closing
        && bytes
            .get(name_start)
            .is_some_and(|b| !b.is_ascii_alphabetic())
    {
        return Tag {
            kind: TagKind::Bang,
            end: tag_end(html, at),
        };
    }

    let mut j = name_start;
    while j < bytes.len()
        && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b':' || bytes[j] == b'-')
    {
        j += 1;
    }
    let name = html[name_start..j].to_ascii_lowercase();
    let end = tag_end(html, at);
    let kind = if closing {
        TagKind::Close(name)
    } else {
        TagKind::Open(name)
    };
    Tag { kind, end }
}

/// Byte offset just past the `>` that closes the tag at `at`, ignoring `>`
/// inside quoted attribute values (`alt="a > b"` must not desync the scan).
fn tag_end(html: &str, at: usize) -> usize {
    let bytes = html.as_bytes();
    let mut quote: Option<u8> = None;
    let mut i = at + 1;
    while i < bytes.len() {
        let b = bytes[i];
        match quote {
            Some(q) if b == q => quote = None,
            Some(_) => {}
            None if b == b'"' || b == b'\'' => quote = Some(b),
            None if b == b'>' => return i + 1,
            None => {}
        }
        i += 1;
    }
    html.len()
}

enum Piece {
    Literal(String),
    Open(u32),
    Close(u32),
}

/// Split a translated segment into its literal text and `<k>` markers.
fn parse_markers(text: &str) -> Vec<Piece> {
    let mut out = Vec::new();
    let mut literal = String::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'<' {
            let closing = text[i..].starts_with("</");
            let digits_at = i + if closing { 2 } else { 1 };
            let mut j = digits_at;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                j += 1;
            }
            if j > digits_at && bytes.get(j) == Some(&b'>') {
                if let Ok(k) = text[digits_at..j].parse::<u32>() {
                    if !literal.is_empty() {
                        out.push(Piece::Literal(std::mem::take(&mut literal)));
                    }
                    out.push(if closing {
                        Piece::Close(k)
                    } else {
                        Piece::Open(k)
                    });
                    i = j + 1;
                    continue;
                }
            }
        }
        let ch = text[i..].chars().next().unwrap_or(' ');
        literal.push(ch);
        i += ch.len_utf8();
    }
    if !literal.is_empty() {
        out.push(Piece::Literal(literal));
    }
    out
}

/// Zero-width characters used as *padding*. `split_whitespace` doesn't see them,
/// so each one survives collapsing as its own "word", and a newsletter preheader
/// carries hundreds of them to push the client's preview line past the teaser.
/// They hold nothing to translate, and a model told to answer faithfully copies
/// them back until it runs out of output budget, leaving every segment after the
/// first untranslated.
///
/// Deliberately narrow. The neighbouring code points are *not* padding and must
/// survive: U+200C and U+200D join emoji sequences and shape Persian and
/// Devanagari words, U+200E and U+200F carry bidirectional direction. Dropping
/// those would corrupt the text on the way to the model, and `apply` writes the
/// answer back over the original.
fn invisible(c: char) -> bool {
    matches!(
        c,
        '\u{00ad}' | '\u{034f}' | '\u{200b}' | '\u{2060}' | '\u{feff}'
    )
}

/// Collapse runs of whitespace to single spaces, keeping the edges: whether a
/// run started or ended with space is what separates it from the markup around.
fn collapse_ws(text: &str) -> String {
    let stripped: String = text.chars().filter(|c| !invisible(*c)).collect();
    let text = stripped.as_str();
    let mut out = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if out.is_empty() {
        // A whitespace-only run still separates the markup around it.
        return if text.is_empty() {
            String::new()
        } else {
            " ".to_string()
        };
    }
    if text.starts_with(char::is_whitespace) {
        out.insert(0, ' ');
    }
    if text.ends_with(char::is_whitespace) {
        out.push(' ');
    }
    out
}

fn escape_text(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn translate_all(extracted: &Extracted, f: impl Fn(&str) -> String) -> HashMap<u32, String> {
        extracted
            .texts
            .iter()
            .enumerate()
            .map(|(i, t)| (i as u32 + 1, f(t)))
            .collect()
    }

    #[test]
    fn splits_at_block_boundaries_and_keeps_inline_markup_inside() {
        let html = "<p>Read our <a href=\"https://x.test\">latest post</a> now</p><p>Bye</p>";
        let ex = extract(html);
        assert_eq!(
            ex.texts,
            vec![
                "Read our <1>latest post</1> now".to_string(),
                "Bye".to_string()
            ]
        );
    }

    #[test]
    fn a_moved_marker_still_lands_on_the_right_run() {
        let html = "<p>Click <b>here</b> to continue</p>";
        let ex = extract(html);
        assert_eq!(ex.texts, vec!["Click <1>here</1> to continue".to_string()]);
        let out = apply(
            html,
            &ex,
            &translate_all(&ex, |_| "Чтобы продолжить, нажмите <1>здесь</1>".into()),
        );
        assert_eq!(out, "<p>Чтобы продолжить, нажмите <b>здесь</b></p>");
    }

    #[test]
    fn a_slash_on_an_inline_tag_is_ignored_like_html5_does() {
        // If it ended the segment instead, the two runs around it would sit in
        // one segment with no marker between them and could not be told apart.
        let html = "<p>before<span/>after the marker</p>";
        let ex = extract(html);
        assert_eq!(ex.texts, vec!["before<1>after the marker</1>".to_string()]);
        let out = apply(
            html,
            &ex,
            &translate_all(&ex, |_| "до<1>после маркера</1>".into()),
        );
        assert_eq!(out, "<p>до<span/>после маркера</p>");
    }

    #[test]
    fn nested_inline_elements_nest_their_markers() {
        let html = "<p>See <a href=\"#\"><b>the docs</b></a> first</p>";
        let ex = extract(html);
        assert_eq!(
            ex.texts,
            vec!["See <1><2>the docs</2></1> first".to_string()]
        );
    }

    #[test]
    fn skips_script_style_and_comments() {
        let html = "<style>.a{color:red}</style><p>Hello there friend</p>\
                    <script>var x = 'text';</script><!-- Wir haben das gesehen -->";
        let ex = extract(html);
        assert_eq!(ex.texts, vec!["Hello there friend".to_string()]);
    }

    #[test]
    fn drops_segments_without_letters() {
        let html = "<td>1 234,00 €</td><td>—</td><td>Total due</td>";
        let ex = extract(html);
        assert_eq!(ex.texts, vec!["Total due".to_string()]);
    }

    #[test]
    fn identical_blocks_are_asked_for_once() {
        let html = "<p>Unsubscribe</p><p>Middle bit</p><td>Unsubscribe</td>";
        let ex = extract(html);
        assert_eq!(ex.len(), 2);
        let out = apply(
            html,
            &ex,
            &translate_all(&ex, |t| {
                if t == "Unsubscribe" {
                    "Отписаться".into()
                } else {
                    "Середина".into()
                }
            }),
        );
        assert_eq!(out, "<p>Отписаться</p><p>Середина</p><td>Отписаться</td>");
    }

    #[test]
    fn a_mismatched_marker_set_collapses_into_the_first_run() {
        let html = "<p>Click <b>here</b> to continue</p>";
        let ex = extract(html);
        // The model dropped the marker entirely.
        let out = apply(
            html,
            &ex,
            &translate_all(&ex, |_| "Нажмите здесь, чтобы продолжить".into()),
        );
        // Text intact, tags intact, inline structure flattened.
        assert_eq!(out, "<p>Нажмите здесь, чтобы продолжить<b></b></p>");
    }

    #[test]
    fn a_missing_segment_keeps_the_original() {
        let html = "<p>First paragraph here</p><p>Second paragraph here</p>";
        let ex = extract(html);
        let mut only_first = HashMap::new();
        only_first.insert(1, "Первый абзац".to_string());
        let out = apply(html, &ex, &only_first);
        assert_eq!(out, "<p>Первый абзац</p><p>Second paragraph here</p>");
    }

    #[test]
    fn escapes_the_translation_it_splices_in() {
        let html = "<p>Terms and conditions</p>";
        let ex = extract(html);
        let out = apply(html, &ex, &translate_all(&ex, |_| "A & B <C>".into()));
        assert_eq!(out, "<p>A &amp; B &lt;C&gt;</p>");
    }

    #[test]
    fn attributes_and_hrefs_survive_untouched() {
        let html = "<a href=\"https://x.test/a?b=1&amp;c=2\" style=\"color:red\">Buy now</a>";
        let ex = extract(html);
        let out = apply(html, &ex, &translate_all(&ex, |_| "Купить".into()));
        assert_eq!(
            out,
            "<a href=\"https://x.test/a?b=1&amp;c=2\" style=\"color:red\">Купить</a>"
        );
    }

    #[test]
    fn a_quoted_angle_bracket_in_an_attribute_does_not_desync_the_scan() {
        let html = "<p><img alt=\"a > b\" src=\"x.png\">Caption text here</p>";
        let ex = extract(html);
        assert_eq!(ex.texts, vec!["Caption text here".to_string()]);
    }

    #[test]
    fn keeps_the_spacing_around_inline_markup() {
        let html = "<p>before <b>bold</b> after</p>";
        let ex = extract(html);
        // A model that trims each fragment would glue the words together.
        let out = apply(
            html,
            &ex,
            &translate_all(&ex, |_| "до<1>жирный</1>после".into()),
        );
        assert_eq!(out, "<p>до <b>жирный</b> после</p>");
    }

    #[test]
    fn truncation_leaves_the_tail_in_the_original_language() {
        let html = "<p>The first paragraph of the message</p><p>The second paragraph</p>";
        let mut ex = extract(html);
        assert!(ex.truncate_to(20, 100));
        assert_eq!(ex.len(), 1);
        let out = apply(html, &ex, &translate_all(&ex, |_| "Первый абзац".into()));
        assert_eq!(out, "<p>Первый абзац</p><p>The second paragraph</p>");
    }

    #[test]
    fn truncation_always_keeps_at_least_one_segment() {
        let html = "<p>A single very long paragraph that blows the whole budget on its own</p>";
        let mut ex = extract(html);
        assert!(!ex.truncate_to(5, 100));
        assert_eq!(ex.len(), 1);
    }

    #[test]
    fn the_subject_rides_along_as_one_more_segment() {
        let html = "<p>Body paragraph here</p>";
        let mut ex = extract(html);
        ex.add_subject("Your invoice is ready");
        assert_eq!(
            ex.prompt_list(),
            "[[1]] Body paragraph here\n[[2]] Your invoice is ready\n"
        );
        assert_eq!(ex.subject_id(), Some(2));
        // It is not spliced into the body — only its own field carries it.
        let out = apply(
            html,
            &ex,
            &translate_all(&ex, |t| {
                if t.starts_with("Body") {
                    "Текст письма".into()
                } else {
                    "Ваш счёт готов".into()
                }
            }),
        );
        assert_eq!(out, "<p>Текст письма</p>");
    }

    #[test]
    fn a_subject_repeated_in_the_body_is_not_paid_for_twice() {
        let html = "<h1>Your invoice is ready</h1><p>Details below</p>";
        let mut ex = extract(html);
        ex.add_subject("Your invoice is ready");
        assert_eq!(ex.len(), 2);
        assert_eq!(ex.subject_id(), Some(1));
    }

    #[test]
    fn the_subject_survives_truncation() {
        let html = "<p>The first paragraph of the message</p><p>The second paragraph</p>";
        let mut ex = extract(html);
        assert!(ex.truncate_to(20, 100));
        ex.add_subject("Still translated");
        assert_eq!(ex.subject_id(), Some(2));
        assert_eq!(ex.len(), 2);
    }

    #[test]
    fn an_empty_subject_adds_nothing() {
        let mut ex = extract("<p>Body paragraph here</p>");
        ex.add_subject("   ");
        assert_eq!(ex.subject_id(), None);
        ex.add_subject("#12345 — (!) 99,90");
        assert_eq!(ex.subject_id(), None);
        assert_eq!(ex.len(), 1);
    }

    #[test]
    fn prompt_list_is_numbered_from_one() {
        let html = "<p>Alpha bravo</p><p>Charlie delta</p>";
        let ex = extract(html);
        assert_eq!(ex.prompt_list(), "[[1]] Alpha bravo\n[[2]] Charlie delta\n");
    }

    #[test]
    fn br_ends_a_segment_so_runs_stay_distinguishable() {
        let html = "<p>Erste Zeile<br>Zweite Zeile</p>";
        let ex = extract(html);
        assert_eq!(
            ex.texts,
            vec!["Erste Zeile".to_string(), "Zweite Zeile".to_string()]
        );
        let out = apply(
            html,
            &ex,
            &translate_all(&ex, |t| {
                if t.starts_with("Erste") {
                    "Первая строка".into()
                } else {
                    "Вторая строка".into()
                }
            }),
        );
        assert_eq!(out, "<p>Первая строка<br>Вторая строка</p>");
    }

    #[test]
    fn plain_text_without_any_tags_is_one_segment() {
        let html = "Just a bare line of text";
        let ex = extract(html);
        assert_eq!(ex.texts, vec!["Just a bare line of text".to_string()]);
        let out = apply(html, &ex, &translate_all(&ex, |_| "Просто строка".into()));
        assert_eq!(out, "Просто строка");
    }

    #[test]
    fn entities_are_decoded_for_the_model_and_reescaped_on_the_way_back() {
        let html = "<p>Terms&nbsp;&amp;&nbsp;conditions apply</p>";
        let ex = extract(html);
        assert_eq!(ex.texts, vec!["Terms & conditions apply".to_string()]);
        let out = apply(
            html,
            &ex,
            &translate_all(&ex, |_| "Правила & условия".into()),
        );
        assert_eq!(out, "<p>Правила &amp; условия</p>");
    }

    #[test]
    fn plain_text_is_segmented_by_paragraph_and_keeps_the_blank_lines() {
        let text = "Guten Tag,\n\nvielen Dank für Ihre Nachricht.\nWir melden uns.\n\nViele Grüße";
        let ex = extract_text(text);
        assert_eq!(
            ex.texts,
            vec![
                "Guten Tag,".to_string(),
                // The hard wrap inside the paragraph is joined for the model.
                "vielen Dank für Ihre Nachricht. Wir melden uns.".to_string(),
                "Viele Grüße".to_string(),
            ]
        );
        let out = apply_text(
            text,
            &ex,
            &translate_all(&ex, |t| match t.chars().next() {
                Some('G') => "Добрый день,".into(),
                Some('v') => "спасибо за ваше сообщение. Мы свяжемся с вами.".into(),
                _ => "С уважением".into(),
            }),
        );
        assert_eq!(
            out,
            "Добрый день,\n\nспасибо за ваше сообщение. Мы свяжемся с вами.\n\nС уважением"
        );
    }

    #[test]
    fn plain_text_is_not_html_escaped_on_the_way_back() {
        let text = "The condition a < b holds for every pair of values in the set";
        let ex = extract_text(text);
        let out = apply_text(text, &ex, &translate_all(&ex, |_| "Условие a < b".into()));
        assert_eq!(out, "Условие a < b");
    }

    #[test]
    fn parses_the_numbered_reply() {
        let reply = "[[1]] Первый абзац\n[[2]] Второй абзац с <1>ссылкой</1>\n";
        let parsed = parse_reply(reply);
        assert_eq!(parsed.get(&1).unwrap(), "Первый абзац");
        assert_eq!(parsed.get(&2).unwrap(), "Второй абзац с <1>ссылкой</1>");
    }

    #[test]
    fn parse_reply_ignores_a_preamble_and_survives_a_cut_off_answer() {
        let reply = "Sure, here are the translations:\n[[1]] Готово\n[[2]] Обрезано на середин";
        let parsed = parse_reply(reply);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed.get(&1).unwrap(), "Готово");
        assert_eq!(parsed.get(&2).unwrap(), "Обрезано на середин");
    }

    #[test]
    fn parse_reply_keeps_a_multi_line_segment_whole() {
        let reply = "[[1]] Erste Zeile\nzweite Zeile derselben Antwort\n[[2]] Zweiter";
        let parsed = parse_reply(reply);
        assert_eq!(
            parsed.get(&1).unwrap(),
            "Erste Zeile\nzweite Zeile derselben Antwort"
        );
    }

    #[test]
    fn invisible_preheader_padding_never_reaches_the_prompt() {
        // Newsletters pad the preheader with hundreds of invisible characters so
        // the client's preview line stops at the teaser. They carry nothing to
        // translate, and a model told to answer faithfully copies them back
        // until it hits the output ceiling, truncating every segment after the
        // first, which then keeps its original language.
        let padding = "\u{034f} ".repeat(200) + &"\u{00ad}".repeat(50);
        let html = format!("<div>Run it anywhere.{padding}</div><p>Hey there,</p>");
        let ex = extract(&html);
        let list = ex.prompt_list();
        assert!(
            !list.contains(['\u{034f}', '\u{00ad}']),
            "invisible padding reached the prompt"
        );
        assert_eq!(ex.texts.len(), 2);
        assert_eq!(ex.texts[0], "Run it anywhere.");
        assert_eq!(ex.texts[1], "Hey there,");
    }

    #[test]
    fn an_inline_wrapping_only_padding_leaves_no_empty_marker() {
        // The preheader trick is often a span holding nothing but padding. With
        // the padding stripped the run is empty, but the marker pair around it
        // would survive: handed `<2></2>`, a model drops it, `distribute` falls
        // back to flattening the segment, and every run but the first comes back
        // empty, which `splice` then writes over the original text.
        let html = "<p>Read our <a href=\"https://x.test\">latest post</a> today\
                    <span>\u{200b}\u{200b}\u{200b}</span></p>";
        let ex = extract(html);
        let list = ex.prompt_list();
        assert!(list.contains("<1>latest post</1>"), "{list}");
        assert!(
            !list.contains("<2>"),
            "an empty marker pair reached the prompt: {list}"
        );
    }

    #[test]
    fn joiners_and_direction_marks_are_not_padding() {
        // The characters next to the padding in the code chart carry meaning:
        // ZWJ composes an emoji, ZWNJ shapes Persian, and the bidi marks set
        // direction. `apply` writes the model's answer back over the original,
        // so dropping them on the way out would corrupt the mail itself.
        let html = "<p>Meet the team \u{1f469}\u{200d}\u{1f4bb}</p>\
                    <p>\u{645}\u{6cc}\u{200c}\u{631}\u{648}\u{645}</p>\
                    <p>\u{200e}left to right</p>";
        let ex = extract(html);
        let list = ex.prompt_list();
        assert!(list.contains('\u{200d}'), "emoji joiner was dropped");
        assert!(list.contains('\u{200c}'), "Persian joiner was dropped");
        assert!(list.contains('\u{200e}'), "direction mark was dropped");
    }

    #[test]
    fn the_result_is_still_valid_input_for_the_sanitizer() {
        let html = "<div><p>Hello <a href=\"https://x.test\">friend</a></p>\
                    <img src=\"cid:logo\"></div>";
        let ex = extract(html);
        let out = apply(
            html,
            &ex,
            &translate_all(&ex, |_| "Привет <1>друг</1>".into()),
        );
        let clean = crate::mail::sanitize::sanitize_email_html(&out, 1, false);
        assert!(clean.html.contains("Привет"));
        assert!(clean.html.contains("друг"));
        assert!(clean.html.contains("https://x.test"));
    }
}
