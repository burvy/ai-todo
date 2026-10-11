//! Turning an email or a PDF into task text.
//!
//! `pdf_to_text` turns PDF bytes into plain text, and `parse_email` pulls the
//! useful part out of an email export: the headers we care about, and a body
//! with quoted reply chains and signatures removed.

pub mod email;
pub mod pdf;

pub use email::{Email, parse_email};
pub use pdf::{PdfError, pdf_to_text};
