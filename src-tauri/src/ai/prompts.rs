//! Prompt construction for Skim's AI features. Email content is always the
//! plain-text rendition, truncated to keep requests bounded.

const MAX_BODY_CHARS: usize = 24_000;
const MAX_CONTEXT_CHARS: usize = 1_500;
const MAX_CHAIN_CHARS: usize = 4_000;

pub fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let cut: String = text.chars().take(max).collect();
    format!("{cut}\n…(truncated)")
}

fn locale_line(locale: &str) -> String {
    format!("Respond in the user's language (locale: {locale}).")
}

/// "Right now for the user it is …" — date, weekday, time, UTC offset.
fn now_block(now: &str) -> String {
    format!("Current date and time for the user: {now}.")
}

pub struct EmailBlock {
    pub from: String,
    pub date: String,
    pub subject: String,
    pub body: String,
    /// Rendered attachment context — extracted text and/or notes about files
    /// provided natively. Empty when the email has no readable attachments or
    /// the feature didn't request them. Has its own budget, so it is appended
    /// verbatim (not subject to the body `limit`).
    pub attachments: String,
}

fn render_emails(emails: &[EmailBlock], limit: usize) -> String {
    emails
        .iter()
        .map(|e| {
            let mut block = format!(
                "--- From: {} | Date: {} | Subject: {} ---\n{}",
                e.from,
                e.date,
                e.subject,
                truncate(&e.body, limit)
            );
            if !e.attachments.is_empty() {
                block.push_str(&format!("\n\nAttachments:\n{}", e.attachments));
            }
            block
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

pub fn summarize(emails: &[EmailBlock], now: &str, locale: &str) -> (String, String) {
    let system = format!(
        "You are Skim's email assistant. Be terse and concrete. {} {}",
        now_block(now),
        locale_line(locale)
    );
    let user = format!(
        "Summarize this email conversation in 2–4 short bullet points. \
         Account for any attached files (their extracted text, or documents/images \
         provided to you directly). Call out action items, deadlines, and any asks \
         directed at the user. No preamble.\n\n{}",
        render_emails(emails, MAX_BODY_CHARS / emails.len().max(1))
    );
    (system, user)
}

/// The user's standing writer preferences from Settings.
#[derive(Default, Clone)]
pub struct WriterProfile {
    /// How the AI refers to / signs as the user.
    pub name: String,
    /// One of the style ids from Settings ('formal', 'friendly', 'mine', …).
    pub style: Option<String>,
    /// Free-form standing instructions (facts, signature rules, …).
    pub instructions: Option<String>,
    /// AI-distilled description of the user's own writing style ('mine').
    pub style_profile: Option<String>,
}

fn style_line(style: Option<&str>) -> &'static str {
    match style {
        Some("formal") => "Default register: professional and formal.",
        Some("friendly") => "Default register: friendly and personable.",
        Some("concise") => "Default register: brief and to the point — no filler.",
        Some("sarcastic") => {
            "Default register: witty, with light irony where appropriate (never rude)."
        }
        Some("enthusiastic") => "Default register: upbeat and energetic.",
        _ => "",
    }
}

/// The standing style directive: a named register, or the analyzed personal
/// style when 'mine' is selected.
fn style_directive(profile: &WriterProfile) -> String {
    if profile.style.as_deref() == Some("mine") {
        if let Some(desc) = profile
            .style_profile
            .as_deref()
            .filter(|s| !s.trim().is_empty())
        {
            return format!(
                "Write exactly the way the user writes. Their personal style, \
                 distilled from their sent mail:\n{}",
                truncate(desc, 2_500)
            );
        }
        return String::new();
    }
    style_line(profile.style.as_deref()).to_string()
}

fn profile_block(profile: &WriterProfile) -> String {
    let mut out = String::new();
    if let Some(instructions) = profile
        .instructions
        .as_deref()
        .filter(|s| !s.trim().is_empty())
    {
        out.push_str(&format!(
            "\nStanding instructions from the user (always follow them):\n{}",
            truncate(instructions, 2_000)
        ));
    }
    out
}

/// `reply_chain` is the conversation in chronological order; the LAST entry
/// is the message being replied to. Empty for a fresh email.
pub fn draft(
    instruction: &str,
    reply_chain: &[EmailBlock],
    tone: Option<&str>,
    profile: &WriterProfile,
    now: &str,
    locale: &str,
) -> (String, String) {
    let tone_line = match tone {
        Some("shorter") => "Keep it under 80 words.".to_string(),
        Some("warmer") => "Use a friendly, warm register.".to_string(),
        Some("formal") => "Use a professional, formal register.".to_string(),
        // The per-request tone chip overrides the standing style.
        _ => style_directive(profile),
    };
    // Replies follow the conversation's language, not the UI locale.
    let language_rule = if reply_chain.is_empty() {
        format!(
            "Write in the language the instruction implies; otherwise {}",
            locale_line(locale)
        )
    } else {
        "Write the reply in the language of the message being replied to — \
         NOT the user's interface language, unless they are the same."
            .to_string()
    };
    let system = format!(
        "You draft emails for {}, writing in their voice (first person). Write only the \
         email body — no subject line, no commentary, no placeholder signature blocks. \
         {} {language_rule} {tone_line}{}",
        profile.name,
        now_block(now),
        profile_block(profile)
    );
    let user = if reply_chain.is_empty() {
        format!("Write an email that does the following: {instruction}")
    } else {
        let per_email = (MAX_BODY_CHARS / reply_chain.len().max(1)).min(MAX_CHAIN_CHARS);
        format!(
            "The conversation so far, oldest first:\n\n{}\n\nWrite a reply to the LAST \
             message that does the following: {instruction}",
            render_emails(reply_chain, per_email)
        )
    };
    (system, user)
}

/// System prompt + reply-context preamble for an interactive drafting session.
/// Each user turn is an instruction; the assistant answers with the full,
/// current email body. `reply_chain` is empty for a fresh email; otherwise it
/// is the conversation being replied to (chronological, replied-to last) and
/// the preamble carries it. The caller folds the preamble into the first turn.
pub fn compose_session(
    reply_chain: &[EmailBlock],
    profile: &WriterProfile,
    now: &str,
    locale: &str,
) -> (String, String) {
    let language_rule = if reply_chain.is_empty() {
        format!(
            "Write in the language the instructions imply; otherwise {}",
            locale_line(locale)
        )
    } else {
        "Write in the language of the message being replied to — NOT the user's \
         interface language, unless they are the same."
            .to_string()
    };
    // A fresh email also gets its subject from the co-author: the reply keeps
    // the client's automatic "Re:" subject, so only new mail needs a header.
    let output_rule = if reply_chain.is_empty() {
        "Begin your reply with a single line in the form `Subject: <a concise, \
         specific subject line>`, then a blank line, then the complete, current email \
         body. Write the subject in the same language as the body. Keep the literal \
         `Subject:` prefix in English. Below that first line, reply with ONLY the email \
         body — no commentary, no code fences, no placeholder signature blocks."
    } else {
        "Always reply with ONLY the complete, current email body — no subject line, \
         no commentary, no code fences, no placeholder signature blocks."
    };
    let system = format!(
        "You draft emails for {}, writing in their voice (first person). This is an \
         interactive drafting session: each user message is an instruction or a revision \
         request. {output_rule} Apply each new instruction to the draft so far, keeping \
         everything the user did not ask to change. {} {language_rule} {}{}",
        profile.name,
        now_block(now),
        style_directive(profile),
        profile_block(profile),
    );
    let preamble = if reply_chain.is_empty() {
        String::new()
    } else {
        let per_email = (MAX_BODY_CHARS / reply_chain.len().max(1)).min(MAX_CHAIN_CHARS);
        format!(
            "You are drafting a reply to the LAST message in this conversation \
             (oldest first):\n\n{}",
            render_emails(reply_chain, per_email)
        )
    };
    (system, preamble)
}

pub fn adjust(
    current_text: &str,
    adjustment: &str,
    profile: &WriterProfile,
    now: &str,
    locale: &str,
) -> (String, String) {
    let directive = match adjustment {
        "shorter" => "Rewrite it to be significantly shorter (aim for half the length) while keeping every essential point.",
        "warmer" => "Rewrite it in a friendlier, warmer register without losing substance.",
        "formal" => "Rewrite it in a professional, formal register.",
        other => other,
    };
    let system = format!(
        "You edit email drafts written in the voice of {}. Output only the rewritten \
         email body, nothing else. {} Keep the original language of the draft; otherwise {}{}",
        profile.name,
        now_block(now),
        locale_line(locale),
        profile_block(profile)
    );
    let user = format!(
        "Current draft:\n\n{}\n\n{directive}",
        truncate(current_text, MAX_BODY_CHARS)
    );
    (system, user)
}

/// Q&A session over an email conversation. `chain` is chronological; the last
/// entry is the message open in the reading pane. Returns (system, preamble) —
/// the preamble is folded into the first user turn, questions arrive as turns.
pub fn ask_session(chain: &[EmailBlock], now: &str, locale: &str) -> (String, String) {
    if chain.len() <= 1 {
        let system = format!(
            "You answer the user's questions about a specific email. Answer from the \
             email's content and any attached files — their extracted text, or the \
             documents/images provided to you directly. If the answer isn't there, say \
             so plainly. Be brief. {} {}",
            now_block(now),
            locale_line(locale)
        );
        let preamble = render_emails(chain, MAX_BODY_CHARS);
        return (system, preamble);
    }
    let system = format!(
        "You answer the user's questions about an email conversation. Answer from the \
         emails' content and any attached files — their extracted text, or the \
         documents/images provided to you directly. If the answer isn't there, say so \
         plainly. Questions are usually about the LAST (most recent) message — use the \
         earlier messages as context. Be brief. {} {}",
        now_block(now),
        locale_line(locale)
    );
    let (earlier, last) = chain.split_at(chain.len() - 1);
    let per_email = (MAX_BODY_CHARS / chain.len()).min(MAX_CHAIN_CHARS);
    let preamble = format!(
        "The email conversation, oldest first:\n\n{}\n\n{}",
        render_emails(earlier, per_email),
        render_emails(last, MAX_BODY_CHARS)
    );
    (system, preamble)
}

/// System prompt for the agentic mailbox assistant (`ai_chat`). The model
/// drives retrieval with the `search_emails` / `read_email` tools; this only
/// sets the behavior — no email content is injected up front.
pub fn chat_agent(now: &str, locale: &str) -> String {
    format!(
        "You are Skim's mailbox assistant, helping the user with questions about their entire \
         mailbox. You have two tools: `search_emails` (find emails by keyword, sender, subject, \
         folder, and/or date range) and `read_email` (read a search result's full body by its \
         [N] number). Always search before answering — never guess from memory. When a question \
         needs figures or detail (e.g. \"how much did I spend on X\"), search for the sender, \
         read the relevant emails, and add up the numbers yourself. For time-based questions, \
         compute the date range from the current date and pass `after`/`before`. Each search \
         result is tagged [N]; cite the emails you used with those bracketed numbers right after \
         the claim they support. If nothing relevant turns up, say so plainly. Be concise. {} {}",
        now_block(now),
        locale_line(locale)
    )
}

/// Catch-up digest of unread mail. `context` is numbered newest-first;
/// `more` is how many older unread messages didn't fit.
pub fn recap(
    context: &[(usize, EmailBlock)],
    more: usize,
    now: &str,
    locale: &str,
) -> (String, String) {
    let system = format!(
        "You are Skim's mailbox assistant. The user just opened their inbox and \
         wants to catch up without reading everything. {} {}",
        now_block(now),
        locale_line(locale)
    );
    let blocks = context
        .iter()
        .map(|(i, e)| {
            format!(
                "<email index=\"{i}\" from=\"{}\" date=\"{}\" subject=\"{}\">\n{}\n</email>",
                e.from,
                e.date,
                e.subject,
                truncate(&e.body, MAX_CONTEXT_CHARS)
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    let tail = if more > 0 {
        format!("\n\n(There are {more} more, older unread emails not included here.)")
    } else {
        String::new()
    };
    let user = format!(
        "These are the user's unread emails, newest first. Write a recap:\n\
         - One short bullet per email (merge closely related ones), naming \
         the sender and the gist.\n\
         - Put things that need an action or a reply first and say what's needed; \
         call out deadlines explicitly.\n\
         - Newsletters and promos get at most a few words each, grouped together.\n\
         - Cite each email with its index like [2] after the bullet.\n\
         Formatting: '-' bullets and **bold** only — no headings, no tables, \
         no preamble. Start directly with the first bullet.\n\n{blocks}{tail}"
    );
    (system, user)
}

/// Prompt for distilling the user's writing style from their sent mail.
/// `samples` are the user's own words (quoted tails stripped), newest first.
pub fn style_analysis(samples: &[String], locale: &str) -> (String, String) {
    let system = format!(
        "You are an expert writing-style analyst. {} The description you produce \
         will be pasted into another AI's system prompt so it can write emails \
         indistinguishable from this person's.",
        locale_line(locale)
    );
    let body = samples
        .iter()
        .enumerate()
        .map(|(i, s)| format!("<email n=\"{}\">\n{s}\n</email>", i + 1))
        .collect::<Vec<_>>()
        .join("\n\n");
    let user = format!(
        "Below are emails written by one person (their own words; quoted replies \
         removed). Distill HOW this person writes into a compact style guide of \
         8–14 short imperative directives, one per line. Cover: tone and formality; \
         typical greetings and sign-offs (quote them verbatim); sentence length and \
         rhythm; favorite words and pet phrases; punctuation and emoji habits; \
         formatting (paragraphs, lists); which languages they use and when. \
         No preamble, no commentary — just the directives.\n\n{body}"
    );
    (system, user)
}
