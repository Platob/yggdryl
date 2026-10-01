//! The naming policy a record encoding uses for its columns.
//!
//! CSV has one optional header record. Excel additionally has physical
//! multirow headers and a conservative inferred read policy. Each medium
//! validates the variants it can actually interpret at its options boundary.

use smol_str::{SmolStr, format_smolstr};

use crate::{Error, Result, Scalar};

/// How record columns are named at read and write boundaries.
///
/// `Rows(1)` is an explicit physical worksheet row in Excel. CSV resolves it
/// to the same binary state as `Source`, because a CSV record has no worksheet
/// coordinate or table metadata.
///
/// ```
/// use yggdryl::{RecordHeader, Scalar};
/// assert_eq!(RecordHeader::from(true), RecordHeader::Source);
/// assert_eq!(RecordHeader::from_scalar(&Scalar::from(2_i64))?, RecordHeader::Rows(2));
/// # Ok::<(), yggdryl::Error>(())
/// ```
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum RecordHeader {
    /// Name columns from their source: the CSV's first record, Excel's first
    /// present worksheet row, or authoritative named-table metadata.
    #[default]
    Source,
    /// Do not consume a naming record or row; Excel uses physical letters.
    None,
    /// Consume exactly this many physical worksheet rows as a hierarchy.
    Rows(u32),
    /// Infer a worksheet header only when its selected source proves one
    /// interpretation; this policy is read-only.
    Infer,
}

impl From<bool> for RecordHeader {
    fn from(value: bool) -> Self {
        if value { Self::Source } else { Self::None }
    }
}

impl RecordHeader {
    /// Resolve a user value into a typed header policy.
    ///
    /// `null` and false mean `None`; true means `Source`. A positive integer
    /// is `Rows(n)`. An absent property must skip the setter rather than be
    /// supplied here as null. Medium-specific support and Excel's grid bound
    /// are checked when options are set or used.
    ///
    /// # Errors
    ///
    /// Refuses another spelling, kind, zero or an integer above `u32::MAX`
    /// at `$.header`.
    pub fn from_scalar(value: &Scalar) -> Result<Self> {
        if let Some(boolean) = value.as_bool() {
            return Ok(Self::from(boolean));
        }
        let rows = value
            .as_i128()
            .and_then(|number| u32::try_from(number).ok())
            .or_else(|| {
                value
                    .as_u128()
                    .and_then(|number| u32::try_from(number).ok())
            });
        if let Some(rows) = rows.filter(|rows| *rows > 0) {
            return Ok(Self::Rows(rows));
        }
        if value.as_i128().is_some() || value.as_u128().is_some() {
            return Err(Error::InvalidRecord {
                path: SmolStr::new_static("$.header"),
                reason: format_smolstr!(
                    "expected a physical header count from 1 to {}, got {value:?}",
                    u32::MAX
                ),
            });
        }
        match value.as_str() {
            Some("source") => Ok(Self::Source),
            Some("none") => Ok(Self::None),
            Some("infer") => Ok(Self::Infer),
            _ if value.is_null() => Ok(Self::None),
            _ => Err(Error::InvalidRecord {
                path: SmolStr::new_static("$.header"),
                reason: format_smolstr!(
                    "expected source, none, infer, a positive row count, a Boolean or null, got {value:?}"
                ),
            }),
        }
    }
}
