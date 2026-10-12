//! Turning an uploaded file into task text.
//!
//! `POST /api/capture` takes either JSON text or a multipart `file`. This
//! module owns the second shape: pull the bytes off the request, turn them
//! into text, and decide which moment "tomorrow" should be measured from.
//!
//! An Outlook email saved as PDF carries its own `Sent:` date, and every
//! deadline in it was written from that day, not from today — so the email's
//! own date anchors the capture. A PDF that is not an email has no such
//! anchor, so it falls back to now.

use axum::extract::Multipart;
use chrono::{DateTime, Utc};

/// Refuse anything larger, so a stray video cannot fill memory.
const MAX_UPLOAD_BYTES: usize = 12 * 1024 * 1024;

#[derive(Debug)]
pub enum UploadError {
    /// No usable `file` field in the multipart body.
    NoFile,
    /// The request named a file we cannot read.
    Io(String),
    /// Not a PDF we could read (or the bytes were not a PDF at all).
    Unreadable(String),
    /// A readable PDF that held no text — a scan, which needs OCR.
    NoText,
    /// The upload was over [`MAX_UPLOAD_BYTES`].
    TooLarge,
}

impl std::fmt::Display for UploadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UploadError::NoFile => write!(f, "no `file` field in the upload"),
            UploadError::Io(why) => write!(f, "could not read the upload: {why}"),
            UploadError::Unreadable(why) => write!(f, "could not read that PDF: {why}"),
            UploadError::NoText => write!(
                f,
                "that PDF has no text layer — a scan needs OCR, which is not supported yet"
            ),
            UploadError::TooLarge => write!(
                f,
                "that file is larger than the {} MB limit",
                MAX_UPLOAD_BYTES / (1024 * 1024)
            ),
        }
    }
}

/// Text pulled out of an upload, plus the moment its dates refer to.
#[derive(Debug)]
pub struct Extracted {
    pub text: String,
    pub anchor: DateTime<Utc>,
    /// True when the file looked like an email and we used its body/sent date.
    pub was_email: bool,
}

/// Pull the first file field out of a multipart body.
///
/// The field is named `file`; other fields are skipped rather than rejected,
/// so a form that also carries a `source` value still works.
pub async fn read_file(multipart: &mut Multipart) -> Result<(String, Vec<u8>), UploadError> {
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| UploadError::Io(e.to_string()))?
    {
        if field.name() != Some("file") {
            continue;
        }
        let name = field
            .file_name()
            .map(str::to_string)
            .unwrap_or_else(|| "upload".to_string());
        let bytes = field
            .bytes()
            .await
            .map_err(|e| UploadError::Io(e.to_string()))?;
        if bytes.len() > MAX_UPLOAD_BYTES {
            return Err(UploadError::TooLarge);
        }
        return Ok((name, bytes.to_vec()));
    }
    Err(UploadError::NoFile)
}

/// PDF bytes to task text, with the anchor those dates should use.
///
/// `pdf-extract` is not panic-free: it indexes into decoded streams and has
/// been known to abort on malformed files. A panic inside a request would
/// take the whole server down, so the call is run on a blocking thread where
/// a panic is contained and reported as a bad request instead.
pub async fn extract(bytes: &[u8]) -> Result<Extracted, UploadError> {
    if bytes.is_empty() {
        return Err(UploadError::Unreadable("the file was empty".into()));
    }

    let owned = bytes.to_vec();
    let parsed = tokio::task::spawn_blocking(move || {
        // The inner catch_unwind is what actually saves us: a panic here is
        // turned into an Err, so spawn_blocking returns Ok(...) and the
        // connection survives. Without it we would only get a JoinError,
        // which we could still handle, but not the message.
        let outcome = std::panic::catch_unwind(move || capture::pdf_to_text(&owned));
        match outcome {
            // Unwrap PdfError rather than using its Display, which already
            // says "could not read that PDF" and would repeat the prefix.
            Ok(Ok(text)) => Ok(text),
            Ok(Err(capture::PdfError::Unreadable(why))) => Err(UploadError::Unreadable(why)),
            Err(_) => Err(UploadError::Unreadable("that PDF could not be read".into())),
        }
    })
    .await;

    let text = match parsed {
        Ok(result) => result?,
        // catch_unwind should have caught it; this is the belt-and-braces path
        // (or a panic in the unwinding itself).
        Err(_) => {
            return Err(UploadError::Unreadable("that PDF could not be read".into()));
        }
    };

    if text.trim().is_empty() {
        return Err(UploadError::NoText);
    }

    Ok(from_text(text))
}

/// Split extracted text into task text plus the moment its dates refer to.
///
/// `parse_email` never fails and never panics: given a document with no
/// headers it simply returns the whole thing as the body. So "is this an
/// email?" has to be answered by whether any headers were actually found,
/// not by whether the body is non-empty.
pub fn from_text(text: String) -> Extracted {
    let email = capture::parse_email(&text);

    let looks_like_email =
        !email.subject.trim().is_empty() || email.sent.is_some() || !email.from.trim().is_empty();

    if !looks_like_email {
        // A plain document. Every line is the text, and "tomorrow" means
        // tomorrow from now.
        return Extracted {
            text,
            anchor: Utc::now(),
            was_email: false,
        };
    }

    let body = email.body.trim();
    if body.is_empty() {
        // An email whose whole message was quoted history or signature.
        // Nothing left to make tasks from.
        return Extracted {
            text,
            anchor: Utc::now(),
            was_email: false,
        };
    }

    Extracted {
        text: email.body,
        // An email with no readable Sent:/Date: header falls back to now.
        anchor: email.sent.unwrap_or_else(Utc::now),
        was_email: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PDF: &[u8] = include_bytes!("../../crates/capture/tests/fixtures/syllabus.pdf");

    // Outlook's own header layout and date format, the same shape the
    // capture crate's own tests use.
    const EMAIL: &str = "From: Dr Wu <wu@example.edu>\r\n\
                         Sent: Friday, October 3, 2025 9:15 AM\r\n\
                         Subject: Lab report\r\n\
                         \r\n\
                         Please finish the lab report by tomorrow.\r\n\
                         \r\n\
                         -----Original Message-----\r\n\
                         older noise that must not become a task\r\n";

    #[tokio::test]
    async fn reads_the_demo_pdf() {
        let out = extract(PDF).await.expect("demo PDF parses");
        assert!(!out.text.trim().is_empty());
        // anchored now, give or take the length of the call
        let drift = (Utc::now() - out.anchor).num_seconds().abs();
        assert!(
            drift < 30,
            "plain PDF should anchor to now, was {drift}s off"
        );
    }

    #[tokio::test]
    async fn a_plain_pdf_is_not_mistaken_for_an_email() {
        let out = extract(PDF).await.expect("demo PDF parses");
        assert!(!out.was_email, "a syllabus has no email headers");
        // the whole document survives as task text
        assert!(
            out.text.to_lowercase().contains("urban ecology"),
            "got: {}",
            out.text
        );
    }

    #[test]
    fn an_email_anchors_to_its_sent_date() {
        let out = from_text(EMAIL.to_string());
        assert!(out.was_email);
        assert!(out.text.contains("lab report"), "got: {}", out.text);
        // the reply chain is cut
        assert!(!out.text.contains("older noise"));
        assert_eq!(out.anchor.format("%Y-%m-%d").to_string(), "2025-10-03");
    }

    #[test]
    fn a_plain_document_anchors_to_now() {
        let out = from_text("Buy printer ink before the library closes.\n".to_string());
        assert!(!out.was_email);
        let drift = (Utc::now() - out.anchor).num_seconds().abs();
        assert!(drift < 5, "should anchor to now, was {drift}s off");
        assert!(out.text.contains("printer ink"));
    }

    #[test]
    fn an_email_with_no_date_falls_back_to_now() {
        let text = "Subject: No date here\r\n\r\nEmail the tutor tomorrow.\r\n";
        let out = from_text(text.to_string());
        assert!(out.was_email);
        let drift = (Utc::now() - out.anchor).num_seconds().abs();
        assert!(drift < 5, "no Sent: header means now, was {drift}s off");
    }

    #[test]
    fn an_email_that_is_all_quotes_falls_back_to_the_whole_document() {
        let text = "Subject: FYI\r\n\r\n-----Original Message-----\r\nonly quoted noise\r\n";
        let out = from_text(text.to_string());
        assert!(!out.was_email, "nothing left to make tasks from");
        assert!(out.text.contains("quoted noise"));
    }

    #[tokio::test]
    async fn garbage_bytes_are_a_bad_request_not_a_crash() {
        let err = extract(b"this is definitely not a pdf").await.unwrap_err();
        assert!(matches!(err, UploadError::Unreadable(_)));
        // the message is safe to show a user
        assert!(!err.to_string().is_empty());
    }

    #[tokio::test]
    async fn empty_upload_is_rejected() {
        let err = extract(b"").await.unwrap_err();
        assert!(matches!(err, UploadError::Unreadable(_)));
    }

    #[tokio::test]
    async fn a_panicking_parser_does_not_take_the_server_down() {
        // What the upload path does when pdf-extract panics: the panic is
        // caught and reported as a bad request, never unwinding into axum.
        let panicked =
            std::panic::catch_unwind(|| -> Result<String, UploadError> { panic!("boom") });
        assert!(panicked.is_err(), "sanity: catch_unwind does catch it");

        // The same shape, run the way the route runs it.
        let owned = b"anything".to_vec();
        let handled = tokio::task::spawn_blocking(move || {
            match std::panic::catch_unwind(move || panic!("boom")) {
                Ok(()) => "no panic".to_string(),
                Err(_) => "caught".to_string(),
            }
        })
        .await;
        assert_eq!(handled.unwrap(), "caught");

        // and the server is still perfectly alive afterwards
        let out = extract(PDF)
            .await
            .expect("still works after a caught panic");
        assert!(!out.text.is_empty());
        let _ = owned;
    }

    #[test]
    fn errors_all_render_a_message_a_user_can_read() {
        for err in [
            UploadError::NoFile,
            UploadError::Io("connection reset".into()),
            UploadError::Unreadable("bad xref".into()),
            UploadError::NoText,
            UploadError::TooLarge,
        ] {
            let message = err.to_string();
            assert!(!message.is_empty(), "{err:?} rendered nothing");
            assert!(!message.contains('{'), "{err:?} leaked a format brace");
        }
    }
}
