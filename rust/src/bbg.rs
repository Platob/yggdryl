//! Bloomberg securities identifiers: `bbg`.

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
pub struct Bbg(SmolStr);

impl Bbg {
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
        let value = crate::ascii_text(BBG_WIDTH, value.as_ref().as_bytes())?;
        if value.is_empty() {
            return Err(crate::Error::InvalidDataType {
                kind: "bbg",
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
    /// than a registry's. Canonical means nonempty ASCII that fits the width,
    /// with no trailing padding. Case is preserved, as by [`Self::new`], so a
    /// terminal spelling such as `AAPL US Equity` remains the same identifier.
    #[must_use]
    pub fn is_canonical(text: &str) -> bool {
        !text.is_empty()
            && crate::ascii_text(BBG_WIDTH, text.as_bytes())
                .is_ok_and(|canonical| canonical == text)
    }
}

impl fmt::Display for Bbg {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

code_value!(Bbg, Bbg, BBG_WIDTH);

/// The extension name a Bloomberg identifier rides.
pub(crate) const BBG_EXTENSION_NAME: &str = "yggdryl.bbg";

/// The most bytes a Bloomberg identifier may be.
///
/// The one code here whose width is a bound rather than a shape. An ISIN is
/// twelve characters because the standard says twelve; a Bloomberg
/// identifier is a ticker, a market and a yellow key with spaces between
/// them - `AAPL US Equity`, `EURUSD Curncy`, `SPX Index` - or a twelve-byte
/// FIGI, and no two are the same length. Thirty-two holds every spelling a
/// terminal writes and still fits one `SmolStr` allocation.
pub(crate) const BBG_WIDTH: usize = 32;

impl DataType {
    /// Creates the Bloomberg identifier datatype.
    ///
    /// ```
    /// use yggdryl::DataType;
    ///
    /// assert_eq!(DataType::bbg(), DataType::Bbg);
    /// assert_eq!(DataType::bbg().to_string(), "bbg");
    /// assert_eq!(DataType::bbg().code_width(), Some(32));
    /// ```
    #[must_use]
    pub const fn bbg() -> Self {
        Self::Bbg
    }
}

define_field_types!(BbgType, Bbg);
