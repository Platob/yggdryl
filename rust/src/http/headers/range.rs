//! Byte ranges: the `Range` a request asks for and the `Content-Range` an
//! answer states, RFC 9110 sections 14.2 and 14.4.
//!
//! Only the `bytes` unit is spoken, because that is the one a positional read
//! is spelled in.

use std::fmt;
use std::str::FromStr;

use smol_str::format_smolstr;

use crate::{Error, Result};

const UNIT: &str = "bytes";

/// Spell a `Range` request header value: `bytes=start-` when `last` is
/// none, else `bytes=start-last`, both positions inclusive.
///
/// ```
/// use yggdryl::http::range_header;
///
/// assert_eq!(range_header(0, Some(99)), "bytes=0-99");
/// assert_eq!(range_header(4096, None), "bytes=4096-");
/// ```
pub fn range_header(start: u64, last: Option<u64>) -> String {
    match last {
        Some(last) => format!("{UNIT}={start}-{last}"),
        None => format!("{UNIT}={start}-"),
    }
}

/// A `Content-Range` header value: the bytes an answer carries and the
/// length of the whole, or the length alone when the range asked for could
/// not be satisfied.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ContentRange {
    /// `bytes start-end/total`, `end` inclusive; `total` is `None` for `*`,
    /// a whole whose length the sender does not know.
    Bytes {
        /// The first byte position the answer carries.
        start: u64,
        /// The last byte position the answer carries, inclusive.
        end: u64,
        /// The length of the whole resource, when the sender knows it.
        total: Option<u64>,
    },
    /// `bytes */total`: the range was not satisfiable, and this is the length
    /// of the whole, which is how a `416` teaches a reader the size.
    Unsatisfied {
        /// The length of the whole resource.
        total: u64,
    },
}

impl ContentRange {
    /// Parse a `Content-Range` header value.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] with `target` `http header` for another unit
    /// than `bytes`, an end before its start, a total no larger than the end,
    /// or a number a `u64` cannot hold.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(value: &str) -> Result<Self> {
        <Self as FromStr>::from_str(value)
    }

    /// The length of the whole resource, when the header states one.
    pub const fn total(&self) -> Option<u64> {
        match self {
            Self::Bytes { total, .. } => *total,
            Self::Unsatisfied { total } => Some(*total),
        }
    }

    /// The number of bytes the answer carries: none for the unsatisfied form.
    pub const fn len(&self) -> u64 {
        match self {
            Self::Bytes { start, end, .. } => *end - *start + 1,
            Self::Unsatisfied { .. } => 0,
        }
    }

    /// Whether the answer carries no bytes.
    pub const fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl FromStr for ContentRange {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        let trimmed = value.trim_matches([' ', '\t']);
        let offset = value.len() - value.trim_start_matches([' ', '\t']).len();
        let Some((unit, rest)) = trimmed.split_once(' ') else {
            return Err(refuse(value, offset, "expected a unit and a range"));
        };
        if !unit.eq_ignore_ascii_case(UNIT) {
            return Err(refuse(value, offset, "expected the bytes unit"));
        }
        let rest_offset = offset + unit.len() + 1;
        let Some((range, total)) = rest.split_once('/') else {
            return Err(refuse(value, rest_offset, "expected a range and a total"));
        };
        let total_offset = rest_offset + range.len() + 1;
        if range == "*" {
            let total = number(value, total_offset, total)?;
            return Ok(Self::Unsatisfied { total });
        }
        let Some((start, end)) = range.split_once('-') else {
            return Err(refuse(value, rest_offset, "expected first-last"));
        };
        let start = number(value, rest_offset, start)?;
        let end = number(value, rest_offset + range.len() - end.len(), end)?;
        if end < start {
            return Err(refuse(
                value,
                rest_offset,
                "expected the last byte at or after the first",
            ));
        }
        let total = if total == "*" {
            None
        } else {
            let total = number(value, total_offset, total)?;
            if total <= end {
                return Err(refuse(
                    value,
                    total_offset,
                    "expected a total beyond the last byte",
                ));
            }
            Some(total)
        };
        Ok(Self::Bytes { start, end, total })
    }
}

impl fmt::Display for ContentRange {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bytes {
                start,
                end,
                total: Some(total),
            } => write!(formatter, "{UNIT} {start}-{end}/{total}"),
            Self::Bytes {
                start,
                end,
                total: None,
            } => write!(formatter, "{UNIT} {start}-{end}/*"),
            Self::Unsatisfied { total } => write!(formatter, "{UNIT} */{total}"),
        }
    }
}

fn number(value: &str, position: usize, digits: &str) -> Result<u64> {
    if digits.is_empty() || digits.bytes().any(|byte| !byte.is_ascii_digit()) {
        return Err(refuse(value, position, "expected a decimal byte position"));
    }
    digits
        .parse()
        .map_err(|_| refuse(value, position, "expected a byte position a u64 holds"))
}

fn refuse(value: &str, position: usize, reason: &str) -> Error {
    Error::Parse {
        target: "http header",
        position,
        reason: format_smolstr!(
            "Content-Range: {reason}, got {:?}",
            crate::text::elide_to(value, 64)
        ),
    }
}
