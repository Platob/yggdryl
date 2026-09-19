//! Bloomberg securities identifiers.

use std::fmt;

use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::code::code_value;
use crate::typed::define_field_types;
use crate::value::CodeValue;
use crate::{DataType, Result, Scalar, Value};

/// One validated Bloomberg identifier.
#[repr(transparent)]
#[derive(Clone, Debug, Default, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct Bloomberg(SmolStr);

impl Bloomberg {
    /// Validate and construct an identifier.
    ///
    /// An identifier has no neutral member: the empty text names no
    /// security, so it is refused here as it is by the other identifiers,
    /// and a column of identifiers refuses it the same way.
    ///
    /// # Errors
    ///
    /// Returns an error naming the width when the text is not ASCII text
    /// that fits it, or when it is empty.
    pub fn new(value: impl AsRef<str>) -> Result<Self> {
        let value = crate::ascii_text(BLOOMBERG_WIDTH, value.as_ref().as_bytes())?;
        if value.is_empty() {
            return Err(crate::Error::InvalidDataType {
                kind: "bloomberg",
                reason: SmolStr::new_static("expected a securities identifier, got \"\""),
            });
        }
        Ok(Self(SmolStr::new(value)))
    }

    /// Borrow the validated identifier.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// Borrow the shared storage without copying the identifier.
    #[must_use]
    pub const fn storage(&self) -> &SmolStr {
        &self.0
    }

    /// Whether `text` is already the canonical spelling of an identifier.
    ///
    /// The one code here with no shape to check: a Bloomberg identifier is a
    /// ticker, a market and a yellow key with spaces between them, or a
    /// FIGI, and the standard that would say which is a terminal's rather
    /// than a registry's. So canonical is what it is for every code - ASCII
    /// that fits the width, in upper case - and no more, because refusing a
    /// spelling nobody published would be a guess.
    #[must_use]
    pub fn is_canonical(text: &str) -> bool {
        !text.is_empty()
            && text.len() <= BLOOMBERG_WIDTH
            && text.is_ascii()
            && !text.bytes().any(|byte| byte.is_ascii_lowercase())
    }
}

impl fmt::Display for Bloomberg {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

code_value!(Bloomberg, Bloomberg, BLOOMBERG_WIDTH);

/// The extension name a Bloomberg identifier rides.
pub(crate) const BLOOMBERG_EXTENSION_NAME: &str = "yggdryl.bloomberg";

/// The most bytes a Bloomberg identifier may be.
///
/// The one code here whose width is a bound rather than a shape. An ISIN is
/// twelve characters because the standard says twelve; a Bloomberg
/// identifier is a ticker, a market and a yellow key with spaces between
/// them - `AAPL US Equity`, `EURUSD Curncy`, `SPX Index` - or a twelve-byte
/// FIGI, and no two are the same length. Thirty-two holds every spelling a
/// terminal writes and still fits one `SmolStr` allocation.
pub(crate) const BLOOMBERG_WIDTH: usize = 32;

define_field_types!(BloombergType, Bloomberg);
