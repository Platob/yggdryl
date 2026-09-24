//! The refusal document both XML stores answer with, over the crate's scanner.
//!
//! Amazon S3 and Azure Blob Storage spell a failure the same way - an `<Error>`
//! with a `<Code>` and a `<Message>` - so it is read once here rather than in
//! each dialect, beside the escaping every rendered body needs. The scanner
//! itself is [`crate::xml`], and each dialect reads its own vocabulary on top
//! of it: `aws::xml` reads `ListBucketResult`, `azure::xml` reads
//! `EnumerationResults`.

pub(crate) use crate::xml::{Element, XmlError, parse_document, parse_root};

use super::answer::ErrorBody;

/// Read the refusal document both XML stores answer with.
///
/// `None` when the bytes are not one.
pub(crate) fn parse_error(xml: &[u8]) -> Option<ErrorBody> {
    let root = parse_document(xml)
        .ok()
        .filter(|root| root.name() == "Error")?;
    Some(error_body(&root))
}

/// The fields of one `<Error>` element; an absent code or message reads as
/// empty rather than failing, since the document still says something went
/// wrong.
pub(crate) fn error_body(error: &Element) -> ErrorBody {
    ErrorBody {
        code: error.child_text("Code").unwrap_or_default().to_owned(),
        message: error.child_text("Message").unwrap_or_default().to_owned(),
        request_id: error.child_text("RequestId").map(str::to_owned),
        resource: error.child_text("Resource").map(str::to_owned),
    }
}

/// Escape `& < > " '` for element text.
pub(crate) fn escape_text(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&apos;"),
            other => escaped.push(other),
        }
    }
    escaped
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/s3/xml.rs` and the two dialect readers' suites pin
    //! and a caller cannot reach.
    //!
    //! The refusal document is the one shape both stores answer with, and the
    //! escaping is what every rendered body goes through. Each forwards, and
    //! changes no visibility; the scanner beneath them is pinned through
    //! `yggdryl::internals::xml`.
    use crate::s3::answer::ErrorBody;

    /// Read the refusal document both XML stores answer with.
    pub fn parse_error(xml: &[u8]) -> Option<ErrorBody> {
        super::parse_error(xml)
    }

    /// `text` with the five XML metacharacters escaped.
    pub fn escape_text(text: &str) -> String {
        super::escape_text(text)
    }
}
