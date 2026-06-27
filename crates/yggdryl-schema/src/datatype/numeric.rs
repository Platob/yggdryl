//! The [`Numeric`] trait — the shared interface of the numeric data types
//! (integers, floats and decimals): their physical width and signedness.

use super::DataType;

/// The common interface of the **numeric** data types — [`Int`](DataType::Int),
/// [`Float`](DataType::Float) and [`Decimal`](DataType::Decimal) — mutualising the
/// two properties they all share: a physical bit width and a signedness. Every method
/// returns `None` for a non-numeric type, so it is safe to call on any [`DataType`].
///
/// (This is broader than the [`is_numeric`](DataType::is_numeric) predicate, which
/// counts only integers and floats; a decimal is numeric *here* because it carries a
/// width and a sign.)
///
/// ```
/// use yggdryl_schema::{DataType, Numeric};
///
/// assert_eq!(DataType::int(32, false).numeric_bits(), Some(32));
/// assert_eq!(DataType::int(32, false).signed(), Some(false));
/// assert_eq!(DataType::float(64).signed(), Some(true));   // floats are always signed
/// assert_eq!(DataType::decimal(10, 2).signed(), Some(true));
/// assert_eq!(DataType::varchar().signed(), None);         // not numeric
/// ```
pub trait Numeric {
    /// The storage width in bits (e.g. `32` for `int32` / `float32`, the decimal's
    /// storage width), or `None` if the type is not numeric.
    fn numeric_bits(&self) -> Option<u16>;

    /// Whether the value is **signed** — the explicit flag for integers, always
    /// `true` for floats and decimals — or `None` if the type is not numeric.
    fn signed(&self) -> Option<bool>;

    /// Whether this type carries a numeric value (integer, float or decimal).
    fn is_numeric_kind(&self) -> bool {
        self.numeric_bits().is_some()
    }
}

impl Numeric for DataType {
    fn numeric_bits(&self) -> Option<u16> {
        match self {
            DataType::Int { bits, .. }
            | DataType::Float { bits }
            | DataType::Decimal { bits, .. } => Some(*bits),
            _ => None,
        }
    }

    fn signed(&self) -> Option<bool> {
        match self {
            DataType::Int { signed, .. } => Some(*signed),
            DataType::Float { .. } | DataType::Decimal { .. } => Some(true),
            _ => None,
        }
    }
}
