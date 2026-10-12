//! Turning an email, a PDF or spoken text into task drafts.
//!
//! `pdf_to_text` turns PDF bytes into plain text, and `parse_email` pulls the
//! useful part out of an email export: the headers we care about, and a body
//! with quoted reply chains and signatures removed. `capture_text` turns the text
//! into [`todo_core::TaskDraft`]s with a [`judge::Judge`].

pub mod email;
pub mod pdf;
pub mod pipeline;
pub mod segment;

pub use email::{Email, parse_email};
pub use pdf::{PdfError, pdf_to_text};
pub use pipeline::{DEFAULT_DURATION_MIN, capture_text, clean_title};
pub use segment::split_clauses;
