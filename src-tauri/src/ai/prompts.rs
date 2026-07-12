//! Prompt construction for Skim's AI features. Email content is always the
//! plain-text rendition, truncated to keep requests bounded.

const MAX_BODY_CHARS: usize = 24_000;
const MAX_CONTEXT_CHARS: usize = 1_500;

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

pub struct EmailBlock {
    pub from: String,
    pub date: String,
    pub subject: String,
    pub body: String,
}

fn render_emails(emails: &[EmailBlock], limit: usize) -> String {
    emails
        .iter()
        .map(|e| {
            format!(
                "--- From: {} | Date: {} | Subject: {} ---\n{}",
                e.from,
                e.date,
                e.subject,
                truncate(&e.body, limit)
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

pub fn summarize(emails: &[EmailBlock], locale: &str) -> (String, String) {
    let system = format!(
        "You are Skim's email assistant. Be terse and concrete. {}",
        locale_line(locale)
    );
    let user = format!(
        "Summarize this email conversation in 2–4 short bullet points. \
         Call out action items, deadlines, and any asks directed at the user. \
         No preamble.\n\n{}",
        render_emails(emails, MAX_BODY_CHARS / emails.len().max(1))
    );
    (system, user)
}

/// The user's standing writer preferences from Settings.
#[derive(Default, Clone)]
pub struct WriterProfile {
    /// How the AI refers to / signs as the user.
    pub name: String,
    /// One of the style ids from Settings ('formal', 'friendly', …).
    pub style: Option<String>,
    /// Free-form standing instructions (facts, signature rules, …).
    pub instructions: Option<String>,
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

pub fn draft(
    instruction: &str,
    reply_context: Option<&EmailBlock>,
    tone: Option<&str>,
    profile: &WriterProfile,
    locale: &str,
) -> (String, String) {
    let tone_line = match tone {
        Some("shorter") => "Keep it under 80 words.",
        Some("warmer") => "Use a friendly, warm register.",
        Some("formal") => "Use a professional, formal register.",
        // The per-request tone chip overrides the standing style.
        _ => style_line(profile.style.as_deref()),
    };
    let system = format!(
        "You draft emails for {}, writing in their voice (first person). Write only the \
         email body — no subject line, no commentary, no placeholder signature blocks. \
         Match the language the conversation is in; otherwise {} {tone_line}{}",
        profile.name,
        locale_line(locale),
        profile_block(profile)
    );
    let user = match reply_context {
        Some(email) => format!(
            "The email being replied to:\n\n{}\n\nWrite a reply that does the following: {instruction}",
            truncate(&email.body, MAX_BODY_CHARS)
        ),
        None => format!("Write an email that does the following: {instruction}"),
    };
    (system, user)
}

pub fn adjust(
    current_text: &str,
    adjustment: &str,
    profile: &WriterProfile,
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
         email body, nothing else. Keep the original language of the draft; otherwise {}{}",
        profile.name,
        locale_line(locale),
        profile_block(profile)
    );
    let user = format!(
        "Current draft:\n\n{}\n\n{directive}",
        truncate(current_text, MAX_BODY_CHARS)
    );
    (system, user)
}

pub fn ask(email: &EmailBlock, question: &str, locale: &str) -> (String, String) {
    let system = format!(
        "You answer questions about a specific email. Answer only from the email's \
         content; if it doesn't contain the answer, say so plainly. Be brief. {}",
        locale_line(locale)
    );
    let user = format!(
        "{}\n\nQuestion: {question}",
        render_emails(std::slice::from_ref(email), MAX_BODY_CHARS)
    );
    (system, user)
}

pub fn chat(
    question: &str,
    context: &[(usize, EmailBlock)],
    today: &str,
    locale: &str,
) -> (String, String) {
    let system = format!(
        "You are Skim's mailbox assistant. Answer the user's question using ONLY the \
         numbered emails provided. Cite sources with bracketed indices like [2] after \
         each claim they support. If the emails don't contain the answer, say so. \
         Today is {today}. Be concise. {}",
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
    let user = format!("{blocks}\n\nQuestion: {question}");
    (system, user)
}
