//! iCalendar (RFC 5545) invitation parsing and iMIP (RFC 6047) REPLY
//! construction.
//!
//! Scope is deliberately small: extract enough from a METHOD:REQUEST /
//! CANCEL / REPLY payload to render an invite card, and emit the tiny
//! METHOD:REPLY calendar that organizers (Google, Outlook) auto-process.

use chrono::{DateTime, NaiveDate, NaiveDateTime, TimeZone, Utc};
use chrono_tz::Tz;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InviteMethod {
    Request,
    Cancel,
    Reply,
}

impl InviteMethod {
    pub fn as_str(&self) -> &'static str {
        match self {
            InviteMethod::Request => "request",
            InviteMethod::Cancel => "cancel",
            InviteMethod::Reply => "reply",
        }
    }
}

#[derive(Debug, Clone)]
pub struct InviteAttendee {
    pub email: String,
    pub name: Option<String>,
    pub partstat: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ParsedInvite {
    pub method: InviteMethod,
    pub uid: String,
    pub sequence: i64,
    pub summary: Option<String>,
    pub location: Option<String>,
    pub organizer_email: Option<String>,
    pub organizer_name: Option<String>,
    pub attendees: Vec<InviteAttendee>,
    /// Unix seconds (UTC) for timed events; None for all-day.
    pub starts_at: Option<i64>,
    pub ends_at: Option<i64>,
    pub is_all_day: bool,
    /// "YYYY-MM-DD" for all-day events (end is inclusive, DTEND fixed up).
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    pub rrule: Option<String>,
    /// Full unfolded RECURRENCE-ID content line, echoed verbatim in replies
    /// so a single-instance answer targets the right occurrence.
    pub recurrence_id_line: Option<String>,
}

// ---- content-line model -------------------------------------------------

struct Prop {
    name: String,
    params: Vec<(String, String)>,
    value: String,
    raw: String,
}

impl Prop {
    fn param(&self, name: &str) -> Option<&str> {
        self.params
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
}

/// RFC 5545 §3.1: a CRLF followed by a space or tab continues the line.
fn unfold(text: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for line in text.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if let Some(rest) = line.strip_prefix([' ', '\t']) {
            if let Some(last) = lines.last_mut() {
                last.push_str(rest);
                continue;
            }
        }
        lines.push(line.to_string());
    }
    lines.retain(|l| !l.is_empty());
    lines
}

/// Split `NAME;PARAM=v;PARAM="q:v":value` respecting double-quoted params.
fn parse_content_line(line: &str) -> Option<Prop> {
    let mut in_quotes = false;
    let mut colon = None;
    for (i, c) in line.char_indices() {
        match c {
            '"' => in_quotes = !in_quotes,
            ':' if !in_quotes => {
                colon = Some(i);
                break;
            }
            _ => {}
        }
    }
    let colon = colon?;
    let head = &line[..colon];
    let value = line[colon + 1..].to_string();

    let mut parts: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    for c in head.chars() {
        match c {
            '"' => {
                in_quotes = !in_quotes;
                cur.push(c);
            }
            ';' if !in_quotes => {
                parts.push(std::mem::take(&mut cur));
            }
            _ => cur.push(c),
        }
    }
    parts.push(cur);

    let name = parts.first()?.trim().to_ascii_uppercase();
    if name.is_empty() {
        return None;
    }
    let params = parts[1..]
        .iter()
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p.as_str(), ""));
            (
                k.trim().to_ascii_uppercase(),
                v.trim().trim_matches('"').to_string(),
            )
        })
        .collect();
    Some(Prop {
        name,
        params,
        value,
        raw: line.to_string(),
    })
}

/// Unescape a TEXT value (RFC 5545 §3.3.11).
fn unescape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') | Some('N') => out.push('\n'),
                Some(other) => out.push(other),
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn non_empty(s: String) -> Option<String> {
    let s = s.trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

// ---- date/time ----------------------------------------------------------

enum IcsTime {
    AllDay(NaiveDate),
    Timed(i64),
}

/// DTSTART/DTEND value forms: `...Z` (UTC), `TZID=<iana>` (wall time in that
/// zone), bare DATE (all-day), floating (treated as user-local). Non-IANA
/// TZIDs (Outlook Windows names) fall back to user-local wall time so the
/// card still renders.
fn parse_dt(prop: &Prop) -> Option<IcsTime> {
    let v = prop.value.trim();
    if prop.param("VALUE") == Some("DATE")
        || (v.len() == 8 && v.chars().all(|c| c.is_ascii_digit()))
    {
        return NaiveDate::parse_from_str(v, "%Y%m%d")
            .ok()
            .map(IcsTime::AllDay);
    }
    if let Some(stripped) = v.strip_suffix('Z') {
        let ndt = NaiveDateTime::parse_from_str(stripped, "%Y%m%dT%H%M%S").ok()?;
        return Some(IcsTime::Timed(Utc.from_utc_datetime(&ndt).timestamp()));
    }
    let ndt = NaiveDateTime::parse_from_str(v, "%Y%m%dT%H%M%S").ok()?;
    if let Some(tzid) = prop.param("TZID") {
        if let Ok(tz) = Tz::from_str(tzid) {
            let ts = tz
                .from_local_datetime(&ndt)
                .earliest()
                .map(|dt| dt.timestamp())?;
            return Some(IcsTime::Timed(ts));
        }
    }
    // Floating or unknown TZID: interpret as user-local wall time.
    chrono::Local
        .from_local_datetime(&ndt)
        .earliest()
        .map(|dt| IcsTime::Timed(dt.timestamp()))
}

/// ISO 8601 duration subset used by ICS: [+-]P[nW][nD][T[nH][nM][nS]].
fn parse_duration_secs(v: &str) -> Option<i64> {
    let v = v.trim();
    let (sign, v) = match v.strip_prefix('-') {
        Some(rest) => (-1i64, rest),
        None => (1i64, v.strip_prefix('+').unwrap_or(v)),
    };
    let v = v.strip_prefix('P')?;
    let mut total = 0i64;
    let mut num = String::new();
    let mut in_time = false;
    for c in v.chars() {
        match c {
            'T' | 't' => in_time = true,
            '0'..='9' => num.push(c),
            _ => {
                let n: i64 = num.parse().ok()?;
                num.clear();
                total += n * match c.to_ascii_uppercase() {
                    'W' => 604_800,
                    'D' => 86_400,
                    'H' if in_time => 3_600,
                    'M' if in_time => 60,
                    'S' if in_time => 1,
                    _ => return None,
                };
            }
        }
    }
    if !num.is_empty() {
        return None;
    }
    Some(sign * total)
}

// ---- parsing ------------------------------------------------------------

fn mailto(value: &str) -> Option<String> {
    let v = value.trim();
    let addr = if v.len() >= 7 && v[..7].eq_ignore_ascii_case("mailto:") {
        &v[7..]
    } else {
        v
    };
    let addr = addr.trim();
    if addr.contains('@') {
        Some(addr.to_string())
    } else {
        None
    }
}

pub fn parse_invite(bytes: &[u8]) -> Option<ParsedInvite> {
    let text = String::from_utf8_lossy(bytes);
    let mut stack: Vec<String> = Vec::new();
    let mut method: Option<String> = None;
    let mut event: Vec<Prop> = Vec::new();
    let mut seen_event = false;
    let mut in_first_event = false;

    for line in unfold(&text) {
        let Some(prop) = parse_content_line(&line) else {
            continue;
        };
        match prop.name.as_str() {
            "BEGIN" => {
                let comp = prop.value.trim().to_ascii_uppercase();
                if comp == "VEVENT" && !seen_event {
                    seen_event = true;
                    in_first_event = true;
                }
                stack.push(comp);
            }
            "END" => {
                let comp = prop.value.trim().to_ascii_uppercase();
                if comp == "VEVENT" {
                    in_first_event = false;
                }
                stack.pop();
            }
            _ => match stack.last().map(|s| s.as_str()) {
                Some("VCALENDAR") if prop.name == "METHOD" => {
                    method = Some(prop.value.trim().to_ascii_uppercase());
                }
                Some("VEVENT") if in_first_event => event.push(prop),
                _ => {}
            },
        }
    }

    let method = match method.as_deref() {
        Some("REQUEST") => InviteMethod::Request,
        Some("CANCEL") => InviteMethod::Cancel,
        Some("REPLY") => InviteMethod::Reply,
        _ => return None,
    };

    let find = |name: &str| event.iter().find(|p| p.name == name);
    let uid = non_empty(find("UID")?.value.clone())?;
    let sequence = find("SEQUENCE")
        .and_then(|p| p.value.trim().parse::<i64>().ok())
        .unwrap_or(0);
    let summary = find("SUMMARY").and_then(|p| non_empty(unescape(&p.value)));
    let location = find("LOCATION").and_then(|p| non_empty(unescape(&p.value)));

    let organizer = find("ORGANIZER");
    let organizer_email = organizer.and_then(|p| mailto(&p.value));
    let organizer_name =
        organizer.and_then(|p| p.param("CN").and_then(|cn| non_empty(cn.to_string())));

    let attendees = event
        .iter()
        .filter(|p| p.name == "ATTENDEE")
        .filter_map(|p| {
            Some(InviteAttendee {
                email: mailto(&p.value)?,
                name: p.param("CN").and_then(|cn| non_empty(cn.to_string())),
                partstat: p.param("PARTSTAT").map(|s| s.to_ascii_uppercase()),
            })
        })
        .collect();

    let start = find("DTSTART").and_then(parse_dt);
    let end = find("DTEND").and_then(parse_dt);

    let mut starts_at = None;
    let mut ends_at = None;
    let mut is_all_day = false;
    let mut start_date = None;
    let mut end_date = None;
    match start {
        Some(IcsTime::AllDay(d)) => {
            is_all_day = true;
            start_date = Some(d.format("%Y-%m-%d").to_string());
            // DTEND is exclusive for all-day events; show the inclusive end.
            let inclusive_end = match end {
                Some(IcsTime::AllDay(e)) if e > d => e.pred_opt().unwrap_or(d),
                _ => d,
            };
            end_date = Some(inclusive_end.format("%Y-%m-%d").to_string());
        }
        Some(IcsTime::Timed(ts)) => {
            starts_at = Some(ts);
            ends_at = match end {
                Some(IcsTime::Timed(te)) => Some(te),
                _ => find("DURATION")
                    .and_then(|p| parse_duration_secs(&p.value))
                    .map(|secs| ts + secs)
                    .or(Some(ts)),
            };
        }
        None => {}
    }

    Some(ParsedInvite {
        method,
        uid,
        sequence,
        summary,
        location,
        organizer_email,
        organizer_name,
        attendees,
        starts_at,
        ends_at,
        is_all_day,
        start_date,
        end_date,
        rrule: find("RRULE").and_then(|p| non_empty(p.value.clone())),
        recurrence_id_line: find("RECURRENCE-ID").map(|p| p.raw.clone()),
    })
}

// ---- REPLY construction ---------------------------------------------------

/// Escape a TEXT value (RFC 5545 §3.3.11).
fn escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            ';' => out.push_str("\\;"),
            ',' => out.push_str("\\,"),
            '\n' => out.push_str("\\n"),
            '\r' => {}
            _ => out.push(c),
        }
    }
    out
}

/// Fold a content line at 75 octets (RFC 5545 §3.1), on char boundaries.
fn push_folded(out: &mut String, line: &str) {
    let mut count = 0usize;
    for c in line.chars() {
        let len = c.len_utf8();
        if count + len > 75 {
            out.push_str("\r\n ");
            count = 1;
        }
        out.push(c);
        count += len;
    }
    out.push_str("\r\n");
}

/// Build the METHOD:REPLY calendar answering `invite` with the given
/// PARTSTAT ("ACCEPTED" | "DECLINED" | "TENTATIVE"). UID / SEQUENCE /
/// ORGANIZER / RECURRENCE-ID are echoed so the organizer's server matches
/// the event.
pub fn build_reply_ics(
    invite: &ParsedInvite,
    attendee_email: &str,
    attendee_name: Option<&str>,
    partstat: &str,
) -> String {
    build_reply_ics_at(invite, attendee_email, attendee_name, partstat, Utc::now())
}

fn build_reply_ics_at(
    invite: &ParsedInvite,
    attendee_email: &str,
    attendee_name: Option<&str>,
    partstat: &str,
    now: DateTime<Utc>,
) -> String {
    let mut out = String::new();
    push_folded(&mut out, "BEGIN:VCALENDAR");
    push_folded(&mut out, "PRODID:-//Skim//Skim Mail//EN");
    push_folded(&mut out, "VERSION:2.0");
    push_folded(&mut out, "CALSCALE:GREGORIAN");
    push_folded(&mut out, "METHOD:REPLY");
    push_folded(&mut out, "BEGIN:VEVENT");
    push_folded(&mut out, &format!("UID:{}", invite.uid));
    push_folded(&mut out, &format!("SEQUENCE:{}", invite.sequence));
    push_folded(
        &mut out,
        &format!("DTSTAMP:{}", now.format("%Y%m%dT%H%M%SZ")),
    );
    if let Some(org) = &invite.organizer_email {
        push_folded(&mut out, &format!("ORGANIZER:mailto:{org}"));
    }
    let cn = attendee_name
        .map(|n| format!(";CN=\"{}\"", n.replace('"', "")))
        .unwrap_or_default();
    push_folded(
        &mut out,
        &format!("ATTENDEE;PARTSTAT={partstat}{cn}:mailto:{attendee_email}"),
    );
    if let Some(rid) = &invite.recurrence_id_line {
        push_folded(&mut out, rid);
    }
    if let Some(summary) = &invite.summary {
        push_folded(&mut out, &format!("SUMMARY:{}", escape(summary)));
    }
    push_folded(&mut out, "END:VEVENT");
    push_folded(&mut out, "END:VCALENDAR");
    out
}

// ---- tests ----------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const GOOGLE_REQUEST: &str = concat!(
        "BEGIN:VCALENDAR\r\n",
        "PRODID:-//Google Inc//Google Calendar 70.9054//EN\r\n",
        "VERSION:2.0\r\n",
        "CALSCALE:GREGORIAN\r\n",
        "METHOD:REQUEST\r\n",
        "BEGIN:VTIMEZONE\r\n",
        "TZID:Europe/Madrid\r\n",
        "BEGIN:DAYLIGHT\r\n",
        "TZOFFSETFROM:+0100\r\n",
        "TZOFFSETTO:+0200\r\n",
        "TZNAME:CEST\r\n",
        "DTSTART:19700329T020000\r\n",
        "RRULE:FREQ=YEARLY;BYMONTH=3;BYDAY=-1SU\r\n",
        "END:DAYLIGHT\r\n",
        "END:VTIMEZONE\r\n",
        "BEGIN:VEVENT\r\n",
        "DTSTART;TZID=Europe/Madrid:20260714T201500\r\n",
        "DTEND;TZID=Europe/Madrid:20260714T204500\r\n",
        "RRULE:FREQ=WEEKLY;BYDAY=TU\r\n",
        "DTSTAMP:20260713T164000Z\r\n",
        "ORGANIZER;CN=Anna Bell:mailto:organizer@example.com\r\n",
        "UID:6ok0f9ss6q0f8dhmkbgeh1sv1c@google.com\r\n",
        "ATTENDEE;CUTYPE=INDIVIDUAL;ROLE=REQ-PARTICIPANT;PARTSTAT=NEEDS-ACTION;RSVP=\r\n",
        " TRUE;CN=attendee@example.com;X-NUM-GUESTS=0:mailto:attendee@exampl\r\n",
        " e.com\r\n",
        "ATTENDEE;CUTYPE=INDIVIDUAL;ROLE=REQ-PARTICIPANT;PARTSTAT=ACCEPTED;CN=Carol\r\n",
        "  Su:mailto:carol@example.com\r\n",
        "LOCATION:Sala \\\"Norte\\\"\\, planta 2\r\n",
        "SEQUENCE:0\r\n",
        "STATUS:CONFIRMED\r\n",
        "SUMMARY:Weekly sync\r\n",
        "END:VEVENT\r\n",
        "END:VCALENDAR\r\n",
    );

    #[test]
    fn parses_google_request() {
        let inv = parse_invite(GOOGLE_REQUEST.as_bytes()).expect("parses");
        assert_eq!(inv.method, InviteMethod::Request);
        assert_eq!(inv.uid, "6ok0f9ss6q0f8dhmkbgeh1sv1c@google.com");
        assert_eq!(inv.sequence, 0);
        assert_eq!(inv.summary.as_deref(), Some("Weekly sync"));
        // Escaped TEXT value is unescaped.
        assert_eq!(inv.location.as_deref(), Some("Sala \"Norte\", planta 2"));
        assert_eq!(
            inv.organizer_email.as_deref(),
            Some("organizer@example.com")
        );
        assert_eq!(inv.organizer_name.as_deref(), Some("Anna Bell"));
        // Folded mid-parameter ATTENDEE lines survive unfolding.
        assert_eq!(inv.attendees.len(), 2);
        assert_eq!(inv.attendees[0].email, "attendee@example.com");
        assert_eq!(inv.attendees[0].partstat.as_deref(), Some("NEEDS-ACTION"));
        assert_eq!(inv.attendees[1].name.as_deref(), Some("Carol Su"));
        // Europe/Madrid is UTC+2 in July.
        let expected = Utc.with_ymd_and_hms(2026, 7, 14, 18, 15, 0).unwrap();
        assert_eq!(inv.starts_at, Some(expected.timestamp()));
        assert_eq!(inv.ends_at, Some(expected.timestamp() + 30 * 60));
        assert!(!inv.is_all_day);
        assert_eq!(inv.rrule.as_deref(), Some("FREQ=WEEKLY;BYDAY=TU"));
    }

    #[test]
    fn parses_all_day() {
        let ics = "BEGIN:VCALENDAR\r\nMETHOD:REQUEST\r\nBEGIN:VEVENT\r\n\
UID:allday@test\r\nDTSTART;VALUE=DATE:20260720\r\nDTEND;VALUE=DATE:20260722\r\n\
SUMMARY:Offsite\r\nORGANIZER:mailto:boss@test.com\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let inv = parse_invite(ics.as_bytes()).unwrap();
        assert!(inv.is_all_day);
        assert_eq!(inv.start_date.as_deref(), Some("2026-07-20"));
        // Exclusive DTEND becomes inclusive display date.
        assert_eq!(inv.end_date.as_deref(), Some("2026-07-21"));
        assert_eq!(inv.starts_at, None);
    }

    #[test]
    fn parses_cancel() {
        let ics = "BEGIN:VCALENDAR\r\nMETHOD:CANCEL\r\nBEGIN:VEVENT\r\n\
UID:x@test\r\nDTSTART:20260714T100000Z\r\nSEQUENCE:1\r\n\
SUMMARY:Dropped\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let inv = parse_invite(ics.as_bytes()).unwrap();
        assert_eq!(inv.method, InviteMethod::Cancel);
        assert_eq!(inv.sequence, 1);
        let expected = Utc.with_ymd_and_hms(2026, 7, 14, 10, 0, 0).unwrap();
        assert_eq!(inv.starts_at, Some(expected.timestamp()));
        // DTEND/DURATION missing: end falls back to start.
        assert_eq!(inv.ends_at, inv.starts_at);
    }

    #[test]
    fn duration_fallback_for_missing_dtend() {
        let ics = "BEGIN:VCALENDAR\r\nMETHOD:REQUEST\r\nBEGIN:VEVENT\r\n\
UID:d@test\r\nDTSTART:20260714T100000Z\r\nDURATION:PT1H30M\r\n\
END:VEVENT\r\nEND:VCALENDAR\r\n";
        let inv = parse_invite(ics.as_bytes()).unwrap();
        assert_eq!(inv.ends_at.unwrap() - inv.starts_at.unwrap(), 5400);
    }

    #[test]
    fn ignores_calendar_without_method() {
        let ics = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:x@test\r\n\
DTSTART:20260714T100000Z\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        assert!(parse_invite(ics.as_bytes()).is_none());
    }

    #[test]
    fn builds_reply() {
        let inv = parse_invite(GOOGLE_REQUEST.as_bytes()).unwrap();
        let now = Utc.with_ymd_and_hms(2026, 7, 13, 17, 0, 0).unwrap();
        let out = build_reply_ics_at(
            &inv,
            "attendee@example.com",
            Some("Alex Doe"),
            "ACCEPTED",
            now,
        );
        // CRLF endings, no line over 75 octets.
        assert!(out.ends_with("END:VCALENDAR\r\n"));
        for line in out.split("\r\n") {
            assert!(line.len() <= 75, "line too long: {line}");
        }
        let unfolded = unfold(&out).join("\n");
        assert!(unfolded.contains("METHOD:REPLY"));
        assert!(unfolded.contains("UID:6ok0f9ss6q0f8dhmkbgeh1sv1c@google.com"));
        assert!(unfolded.contains("SEQUENCE:0"));
        assert!(unfolded.contains("DTSTAMP:20260713T170000Z"));
        assert!(unfolded.contains("ORGANIZER:mailto:organizer@example.com"));
        assert!(unfolded
            .contains("ATTENDEE;PARTSTAT=ACCEPTED;CN=\"Alex Doe\":mailto:attendee@example.com"));
        assert!(unfolded.contains("SUMMARY:Weekly sync"));
    }

    #[test]
    fn reply_echoes_recurrence_id() {
        let ics = "BEGIN:VCALENDAR\r\nMETHOD:REQUEST\r\nBEGIN:VEVENT\r\n\
UID:r@test\r\nDTSTART:20260721T100000Z\r\n\
RECURRENCE-ID;TZID=Europe/Madrid:20260721T120000\r\n\
ORGANIZER:mailto:boss@test.com\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let inv = parse_invite(ics.as_bytes()).unwrap();
        let out = build_reply_ics(&inv, "me@test.com", None, "DECLINED");
        assert!(out.contains("RECURRENCE-ID;TZID=Europe/Madrid:20260721T120000"));
        assert!(out.contains("ATTENDEE;PARTSTAT=DECLINED:mailto:me@test.com"));
    }
}
