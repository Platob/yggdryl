//! Entity tags, RFC 9110 section 8.8.3: an opaque validator, strong or weak,
//! and the two comparisons a condition is evaluated with.

use std::fmt;
use std::str::FromStr;

use smol_str::format_smolstr;

use crate::{Error, Result};

/// An `ETag` header value: `"opaque"`, or `W/"opaque"` for a weak one.
///
/// The opaque text is kept without its quotes and without the weakness
/// marker; [`fmt::Display`] writes both back.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ETag {
    /// The validator between the quotes, as the origin server spelled it.
    pub opaque: String,
    /// Whether the tag is weak (`W/`), naming a semantically equivalent
    /// representation rather than a byte-identical one.
    pub weak: bool,
}

impl ETag {
    /// Parse an `ETag` header value.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] with `target` `http header` when the text is
    /// not a quoted tag, optionally preceded by `W/`: an unquoted value, a
    /// quote or a control byte inside the tag, or text after the closing
    /// quote.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(value: &str) -> Result<Self> {
        <Self as FromStr>::from_str(value)
    }

    /// Whether the tag is weak.
    pub const fn is_weak(&self) -> bool {
        self.weak
    }

    /// The strong comparison: both tags strong and their opaque text equal
    /// byte for byte. What `If-Match` and `If-Range` evaluate.
    pub fn strong_eq(&self, other: &Self) -> bool {
        !self.weak && !other.weak && self.opaque == other.opaque
    }

    /// The weak comparison: the opaque text equal, weakness ignored. What
    /// `If-None-Match` evaluates.
    pub fn weak_eq(&self, other: &Self) -> bool {
        self.opaque == other.opaque
    }
}

impl FromStr for ETag {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        let trimmed = value.trim_matches([' ', '\t']);
        let offset = value.len() - value.trim_start_matches([' ', '\t']).len();
        let (weak, quoted, quoted_offset) = match trimmed.strip_prefix("W/") {
            Some(quoted) => (true, quoted, offset + 2),
            None => (false, trimmed, offset),
        };
        let Some(opaque) = quoted
            .strip_prefix('"')
            .and_then(|rest| rest.strip_suffix('"'))
        else {
            return Err(refuse(value, quoted_offset, "expected a quoted tag"));
        };
        // etagc = %x21 / %x23-7E / obs-text: no quote, no control, no space.
        if let Some(index) = opaque
            .bytes()
            .position(|byte| byte == b'"' || byte < 0x21 || byte == 0x7f)
        {
            return Err(refuse(
                value,
                quoted_offset + 1 + index,
                "expected no quote, space or control byte inside the tag",
            ));
        }
        Ok(Self {
            opaque: opaque.to_owned(),
            weak,
        })
    }
}

impl fmt::Display for ETag {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.weak {
            formatter.write_str("W/")?;
        }
        write!(formatter, "\"{}\"", self.opaque)
    }
}

fn refuse(value: &str, position: usize, reason: &str) -> Error {
    Error::Parse {
        target: "http header",
        position,
        reason: format_smolstr!("ETag: {reason}, got {:?}", crate::text::elide_to(value, 64)),
    }
}
