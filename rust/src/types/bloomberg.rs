//! Bloomberg securities identifiers.

use std::fmt;

use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::types;
use crate::types::code::{CodeValue, code_leaf, code_value};
use crate::types::typed::define_field_types;
use crate::{DataType, Result, Scalar, Value};

code_leaf!(Bloomberg, BLOOMBERG_WIDTH);

impl Bloomberg {
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
