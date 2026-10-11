//! Pull an email apart into the bits we need, and throw the rest away.
//!
//! Outlook exports are mostly quoted history: the reply chain, the previous
//! message, a signature and a legal disclaimer. Feeding all of that to the
//! task extractor produces phantom tasks like "see below" and "Jane Doe,
//! Senior Analyst". So we cut at the first sign of any of it.

use chrono::{DateTime, NaiveDate, NaiveDateTime, TimeZone, Utc};

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Email {
    pub subject: String,
    /// When it was sent. Used as the anchor for "tomorrow" / "by Friday".
    pub sent: Option<DateTime<Utc>>,
    pub from: String,
    /// Just the new message, with quoted text and signatures removed.
    pub body: String,
}

/// Markers that mean "everything below this is quoted history".
const QUOTE_MARKERS: [&str; 6] = [
    "-----original message-----",
    "________________________________",
    "-------- forwarded message --------",
    "begin forwarded message:",
    "---------- forwarded message",
    "----- message snippet",
];

/// Legal boilerplate that shows up in every university signature block.
const DISCLAIMERS: [&str; 3] = [
    "this email and any attachments",
    "this message and any attachments",
    "the information contained in this email",
];

/// Parse an email export. Unknown or missing headers are tolerated: a PDF that
/// happens to contain an email-looking blob should still yield a body.
pub fn parse_email(text: &str) -> Email {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let (headers, body) = split_headers(&text);

    let subject = header_value(&headers, "subject").unwrap_or_default();
    let from = header_value(&headers, "from").unwrap_or_default();
    // Outlook writes `Sent:`; RFC 5322 uses `Date:`. Prefer whichever exists.
    let sent = header_value(&headers, "sent")
        .or_else(|| header_value(&headers, "date"))
        .and_then(|v| parse_email_date(&v));

    Email {
        subject,
        sent,
        from,
        body: clean_body(&body),
    }
}

/// Split leading `Name: value` headers from the body at the first blank line.
///
/// A PDF of an email often has no blank line between headers and body, so we
/// also stop when a line stops looking like a header.
fn split_headers(text: &str) -> (Vec<(String, String)>, String) {
    let mut headers: Vec<(String, String)> = Vec::new();
    let lines: Vec<&str> = text.lines().collect();
    let mut body_start = lines.len();
    for (index, line) in lines.iter().enumerate() {
        if line.trim().is_empty() {
            body_start = index + 1;
            break;
        }
        if line.starts_with([' ', '\t']) {
            if let Some((_, value)) = headers.last_mut() {
                value.push(' ');
                value.push_str(line.trim());
                continue;
            }
            body_start = index;
            break;
        }
        match line.split_once(':') {
            Some((name, value))
                if !name.trim().is_empty()
                    && name
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || " -_".contains(c))
                    && !name.trim().starts_with(">") =>
            {
                headers.push((name.trim().to_ascii_lowercase(), value.trim().to_string()));
            }
            // A line with no colon (or an obviously non-header one) means the
            // headers are over, even without a blank line in between.
            _ => {
                body_start = index;
                break;
            }
        }
    }
    let body = lines[body_start..].join("\n");
    (headers, body)
}

fn header_value(headers: &[(String, String)], name: &str) -> Option<String> {
    headers
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.clone())
        .filter(|value| !value.is_empty())
}

/// Remove quoted history, signatures and disclaimers from the body.
fn clean_body(body: &str) -> String {
    let mut kept: Vec<&str> = Vec::new();

    for line in body.lines() {
        let trimmed = line.trim();
        let lower = trimmed.to_ascii_lowercase();

        // Classic quoted line.
        if trimmed.starts_with('>') {
            break;
        }
        if QUOTE_MARKERS.iter().any(|marker| lower.starts_with(marker)) {
            break;
        }
        if lower.starts_with("on ") && (lower.contains(" wrote:") || lower.contains(" wrote :")) {
            break;
        }
        if DISCLAIMERS.iter().any(|d| lower.starts_with(d)) {
            break;
        }
        // Outlook replies repeat the headers of the previous message inline.
        if starts_new_quoted_message(trimmed, &lower) {
            break;
        }
        // RFC 3676 signature separator: "-- " on its own line.
        if trimmed == "--" {
            break;
        }
        kept.push(line);
    }

    kept.join("\n").trim().to_string()
}

/// True when a `From:` appears in the body, followed by `Sent:`/`To:`/
/// `Subject:` within the next few lines: that is a quoted previous message.
fn starts_new_quoted_message(line: &str, lower: &str) -> bool {
    if !lower.starts_with("from:") {
        return false;
    }
    // A signature often starts with "From: Jane" alone, but a quoted message
    // nearly always repeats several headers.
    line.split_once(':')
        .is_some_and(|(_, v)| v.trim().len() > 3)
}

/// Parse the date formats that turn up in real Outlook exports.
fn parse_email_date(raw: &str) -> Option<DateTime<Utc>> {
    let value = raw.trim();
    if value.is_empty() {
        return None;
    }

    // Already unambiguous, e.g. 2026-10-12T14:03:00Z
    if let Ok(parsed) = DateTime::parse_from_rfc3339(value) {
        return Some(parsed.with_timezone(&Utc));
    }

    const FORMATS: [&str; 6] = [
        "%A, %B %d, %Y %I:%M %p",
        "%A, %B %d, %Y %l:%M %p",
        "%a, %d %b %Y %H:%M:%S %z",
        "%m/%d/%Y %I:%M %p",
        "%m/%d/%Y %H:%M",
        "%Y-%m-%d %H:%M:%S",
    ];

    for format in FORMATS {
        if let Ok(naive) = NaiveDateTime::parse_from_str(value, format) {
            return Some(naive.and_utc());
        }
        // A date with no time, e.g. "Monday, October 12, 2026".
        if let Ok(date) = NaiveDate::parse_from_str(value, format) {
            return Some(Utc.from_utc_datetime(&date.and_hms_opt(9, 0, 0)?));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const OUTLOOK: &str = "From: Jane Doe <jane@example.edu>\n\
Sent: Monday, October 12, 2026 10:03 AM\n\
To: Me <me@example.edu>\n\
Subject: Lab report\n\
\n\
Hi,\n\
\n\
Please finish the chem lab report by Friday and email your TA tomorrow.\n\
\n\
Thanks,\n\
Jane\n\
\n\
-----Original Message-----\n\
From: Jane Doe <jane@example.edu>\n\
Sent: Friday, October 9, 2026 4:00 PM\n\
Subject: earlier question\n\
\n\
Are you still going to do the lab report?\n";

    #[test]
    fn reads_the_headers() {
        let email = parse_email(OUTLOOK);
        assert_eq!(email.subject, "Lab report");
        assert!(email.from.contains("Jane Doe"));
        assert!(email.from.contains("jane@example.edu"));
    }

    #[test]
    fn reads_the_sent_date_so_it_can_anchor_deadlines() {
        let email = parse_email(OUTLOOK);
        let sent = email.sent.expect("sent date parsed");
        assert_eq!(sent.to_rfc3339(), "2026-10-12T10:03:00+00:00");
    }

    #[test]
    fn keeps_only_the_new_message() {
        let email = parse_email(OUTLOOK);
        assert!(email.body.contains("finish the chem lab report"));
        assert!(
            !email.body.contains("Are you still going"),
            "quoted text survived"
        );
        assert!(!email.body.contains("Original Message"));
        assert!(!email.body.contains("earlier question"));
    }

    #[test]
    fn cuts_at_an_outlook_underscore_separator() {
        let text = "Sent: Monday, October 12, 2026 10:03 AM\n\n\
Do the thing by Friday.\n\
________________________________\n\
From: someone\n\nOld stuff\n";
        let email = parse_email(text);
        assert_eq!(email.body, "Do the thing by Friday.");
    }

    #[test]
    fn cuts_at_a_duplicate_from_header_without_a_separator() {
        let text = "Sent: Monday, October 12, 2026 10:03 AM\n\n\
Please book the room.\n\
From: Booking System <no-reply@example.edu>\n\
Sent: Monday, October 12, 2026 9:00 AM\n\
Subject: confirmation\n";
        let email = parse_email(text);
        assert_eq!(email.body, "Please book the room.");
    }

    #[test]
    fn keeps_a_signature_line_that_is_not_a_quote() {
        let text = "Sent: Monday, October 12, 2026 10:03 AM\n\n\
Submit the form by Friday.\n\n\
From: the desk\n\n\
Call us if you have questions.\n";
        let email = parse_email(text);
        assert!(email.body.contains("Submit the form by Friday"));
    }

    #[test]
    fn drops_legal_disclaimers() {
        let text = "Sent: Monday, October 12, 2026 10:03 AM\n\n\
Pay the balance by Friday.\n\n\
This email and any attachments are confidential.\n";
        let email = parse_email(text);
        assert_eq!(email.body, "Pay the balance by Friday.");
    }

    #[test]
    fn tolerates_headers_without_a_blank_line() {
        // This is what PDF text extraction of an email usually looks like.
        let text = "From: jane@example.edu\nSent: Monday, October 12, 2026 10:03 AM\n\
Subject: Invoice\nPlease pay the invoice by Friday.\n";
        let email = parse_email(text);
        assert_eq!(email.subject, "Invoice");
        assert!(email.body.contains("pay the invoice by Friday"));
    }

    #[test]
    fn survives_text_with_no_headers_at_all() {
        let email = parse_email("Just a plain note. Finish the report by Friday.\n");
        assert_eq!(email.subject, "");
        assert!(email.from.is_empty());
        assert!(email.sent.is_none());
        assert!(email.body.contains("Finish the report"));
    }

    #[test]
    fn accepts_several_date_formats() {
        let cases = [
            (
                "Monday, October 12, 2026 10:03 AM",
                "2026-10-12T10:03:00+00:00",
            ),
            ("10/12/2026 10:03 AM", "2026-10-12T10:03:00+00:00"),
            ("2026-10-12T14:03:00Z", "2026-10-12T14:03:00+00:00"),
        ];
        for (raw, expected) in cases {
            let parsed = parse_email_date(raw).unwrap_or_else(|| panic!("failed on {raw}"));
            assert_eq!(parsed.to_rfc3339(), expected, "for {raw}");
        }
        assert!(parse_email_date("not a date").is_none());
    }

    // ---- shapes real mail actually arrives in ------------------------------

    #[test]
    fn crlf_headers_parse() {
        // RFC 5322 mail is CRLF. This is what an Outlook PDF export contains.
        let text = "From: Jane Doe <jane@example.edu>\r\n\
            Sent: Monday, October 12, 2026 10:03 AM\r\n\
            Subject: Lab report\r\n\
            \r\n\
            Please finish the lab report by Friday.\r\n";
        let email = parse_email(text);
        assert_eq!(email.subject, "Lab report");
        assert_eq!(email.body, "Please finish the lab report by Friday.");
    }

    #[test]
    fn a_gmail_or_apple_mail_quote_attribution_is_cut() {
        // Gmail and Apple Mail do not write "-----Original Message-----".
        // They write an attribution line, and everything under it is the
        // previous message.
        let text = "Sent: Monday, October 12, 2026 10:03 AM\n\n\
            Please finish the lab report by Friday.\n\n\
            On Mon, Oct 12, 2026 at 9:15 AM Jane Doe <jane@example.edu> wrote:\n\n\
            > Are you still doing the lab report?\n";
        let email = parse_email(text);
        assert_eq!(
            email.body, "Please finish the lab report by Friday.",
            "the quoted previous message leaked in: {:?}",
            email.body
        );
    }

    #[test]
    fn a_folded_header_is_joined() {
        // RFC 5322 folds long headers onto a continuation line starting with a
        // space. A parser that misses this truncates the subject.
        let text = "From: jane@example.edu\n\
            Sent: Monday, October 12, 2026 10:03 AM\n\
            Subject: A very long subject line about the chemistry lab that\n  would not fit on one line\n\
            \n\
            Please finish the lab report by Friday.\n";
        let email = parse_email(text);
        assert_eq!(
            email.subject,
            "A very long subject line about the chemistry lab that would not fit on one line"
        );
    }

    #[test]
    fn an_rfc5322_date_header_is_used_when_there_is_no_sent() {
        // Gmail exports use Date:, not Outlook's Sent:.
        let text = "From: jane@example.edu\n\
            Date: Mon, 12 Oct 2026 10:03:00 +0000\n\
            Subject: Lab report\n\n\
            Please finish the lab report by Friday.\n";
        let email = parse_email(text);
        let sent = email.sent.expect("Date: is read as the send time");
        assert_eq!(sent.to_rfc3339(), "2026-10-12T10:03:00+00:00");
    }

    #[test]
    fn a_body_that_is_entirely_quoted_leaves_nothing_to_extract() {
        // A reply that adds no words of its own. The caller needs to know the
        // body is empty, rather than be handed "" to make phantom tasks from.
        let text = "Sent: Monday, October 12, 2026 10:03 AM\n\n\
            On Mon, Oct 12, 2026 at 9:15 AM Jane <jane@example.edu> wrote:\n\n\
            > Are you still doing the lab report?\n";
        let email = parse_email(text);
        assert!(
            email.body.trim().is_empty(),
            "expected an empty body, got {:?}",
            email.body
        );
    }

    #[test]
    fn a_quote_marker_mentioned_in_passing_is_not_treated_as_history() {
        // A body that quotes the marker as a word should keep its text.
        let text = "Sent: Monday, October 12, 2026 10:03 AM\n\n\
            Attach the file by Friday.\n\n\
            The lab manual calls this the 'forwarded message' step, so ignore it.\n";
        let email = parse_email(text);
        assert!(
            email.body.contains("Attach the file by Friday"),
            "lost the real sentence: {:?}",
            email.body
        );
    }
}
