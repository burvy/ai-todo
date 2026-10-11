//! PDF bytes to plain text, via the `pdf-extract` crate.

use std::fmt;

#[derive(Debug)]
pub enum PdfError {
    /// The bytes were not a PDF we could read.
    Unreadable(String),
}

impl fmt::Display for PdfError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PdfError::Unreadable(why) => write!(f, "could not read that PDF: {why}"),
        }
    }
}

impl std::error::Error for PdfError {}

/// Extract the text layer of a PDF.
///
/// Returns an error rather than empty text for anything unreadable, so the
/// caller can tell "this was not a PDF" apart from "this PDF had no text"
/// (a scan, which needs OCR and is out of scope).
pub fn pdf_to_text(bytes: &[u8]) -> Result<String, PdfError> {
    if bytes.is_empty() {
        return Err(PdfError::Unreadable("the file was empty".into()));
    }
    let text = pdf_extract::extract_text_from_mem(bytes)
        .map_err(|e| PdfError::Unreadable(e.to_string()))?;
    Ok(normalise(&text))
}

/// Collapse the ragged line breaks PDF text extraction leaves behind.
///
/// Text inside a paragraph is joined into one line (task extraction splits on
/// sentence boundaries anyway), and a single blank line separates paragraphs.
/// Repeated blank lines collapse, which keeps page furniture from opening huge
/// gaps in the output.
fn normalise(text: &str) -> String {
    let cleaned = text.replace("\r\n", "\n").replace('\r', "\n");

    let mut paragraphs: Vec<String> = Vec::new();
    let mut current = String::new();

    for line in cleaned.lines() {
        let line = line.trim();
        if line.is_empty() {
            if !current.is_empty() {
                paragraphs.push(std::mem::take(&mut current));
            }
            continue;
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(line);
    }
    if !current.is_empty() {
        paragraphs.push(current);
    }

    paragraphs.join("\n").trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const PDF: &[u8] = include_bytes!("../../../demo/syllabus.pdf");

    #[test]
    fn reads_a_real_pdf() {
        let text = pdf_to_text(PDF).expect("demo PDF parses");
        assert!(!text.trim().is_empty(), "should recover some text");
        assert!(
            text.chars().count() > 100,
            "expected real content, got {:?}",
            &text[..text.len().min(120)]
        );
    }

    #[test]
    fn rejects_junk_instead_of_returning_empty_text() {
        assert!(pdf_to_text(b"this is not a pdf at all").is_err());
        assert!(pdf_to_text(&[]).is_err());
    }

    #[test]
    fn rejoins_wrapped_lines_into_sentences() {
        let wrapped =
            "The quarterly report is due.\nI need to file the tax\nreturn before Friday.\n";
        let out = normalise(wrapped);
        assert!(
            out.contains("file the tax return before Friday"),
            "got {out:?}"
        );
    }

    #[test]
    fn keeps_paragraph_breaks() {
        let out = normalise("First line\nsecond line\n\nNew paragraph\n");
        assert_eq!(out, "First line second line\nNew paragraph");
    }

    #[test]
    fn collapses_repeated_blank_lines() {
        let out = normalise("A\n\n\n\n\nB\n");
        assert_eq!(out, "A\nB");
    }

    #[test]
    fn it_does_not_panic_on_hostile_or_truncated_pdfs() {
        // pdf-extract walks xref tables and object streams by hand and is
        // known to panic on malformed input. If that panic reaches the server
        // it takes the whole process down, so pdf_to_text must turn every one
        // of these into an Err. docs/TASKS-pdf.md calls this out explicitly.
        let cases: Vec<(&str, &[u8])> = vec![
            ("just a header", b"%PDF-1.4\n"),
            ("header and EOF only", b"%PDF-1.4\n%%EOF\n"),
            (
                "empty xref table",
                b"%PDF-1.4\nxref\n0 0\ntrailer\n<< /Size 0 >>\n%%EOF\n",
            ),
            (
                "dangling object reference",
                b"%PDF-1.4\n1 0 obj\n<< /Type /Catalog /Pages 99 0 R >>\nendobj\n\
                 xref\n0 2\n0000000000 65535 f \ntrailer\n<< /Size 2 /Root 1 0 R >>\n\
                 startxref\n9\n%%EOF\n",
            ),
            (
                "binary junk after the header",
                b"%PDF-1.4\n\x00\x01\x02\x03\xff\xfe\n",
            ),
            ("truncated mid-object", b"%PDF-1.4\n1 0 obj\n<< /Type /Pa"),
            (
                "negative object count",
                b"%PDF-1.4\nxref\n0 -5\ntrailer\n<< /Size -1 >>\n%%EOF\n",
            ),
        ];

        for (name, bytes) in cases {
            let outcome = std::panic::catch_unwind(|| pdf_to_text(bytes));
            match outcome {
                Ok(Ok(text)) => println!("  {name}: parsed ({})", text.len()),
                Ok(Err(_)) => println!("  {name}: rejected cleanly"),
                Err(_) => panic!("pdf_to_text PANICKED on: {name}"),
            }
        }
    }
}
