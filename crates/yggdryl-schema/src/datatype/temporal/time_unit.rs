//! The resolution of a temporal data type.

use core::fmt;

use arrow_schema::TimeUnit as ArrowTimeUnit;

use crate::DataTypeError;

/// The resolution of a temporal type, mapping to Arrow `TimeUnit`.
///
/// ```
/// use yggdryl_schema::TimeUnit;
///
/// let arrow = TimeUnit::Millisecond.to_arrow();
/// assert_eq!(TimeUnit::from_arrow(arrow), TimeUnit::Millisecond);
/// assert_eq!(TimeUnit::Millisecond.to_string(), "ms");
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum TimeUnit {
    /// One second.
    Second,
    /// One thousandth of a second.
    Millisecond,
    /// One millionth of a second.
    Microsecond,
    /// One billionth of a second.
    Nanosecond,
}

impl TimeUnit {
    /// The Arrow time unit this unit maps to.
    pub fn to_arrow(&self) -> ArrowTimeUnit {
        match self {
            Self::Second => ArrowTimeUnit::Second,
            Self::Millisecond => ArrowTimeUnit::Millisecond,
            Self::Microsecond => ArrowTimeUnit::Microsecond,
            Self::Nanosecond => ArrowTimeUnit::Nanosecond,
        }
    }

    /// Converts an Arrow time unit; total, since the sets are identical.
    pub fn from_arrow(unit: ArrowTimeUnit) -> Self {
        match unit {
            ArrowTimeUnit::Second => Self::Second,
            ArrowTimeUnit::Millisecond => Self::Millisecond,
            ArrowTimeUnit::Microsecond => Self::Microsecond,
            ArrowTimeUnit::Nanosecond => Self::Nanosecond,
        }
    }

    /// Serializes the unit as its one-byte tag.
    pub fn to_bytes(&self) -> Vec<u8> {
        // The tag is the declaration index; the decode below is the other
        // half of the contract, so the variants must never be reordered.
        vec![*self as u8]
    }

    /// Deserializes the unit from the encoding produced by
    /// [`to_bytes`](TimeUnit::to_bytes).
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, DataTypeError> {
        match bytes {
            [0] => Ok(Self::Second),
            [1] => Ok(Self::Millisecond),
            [2] => Ok(Self::Microsecond),
            [3] => Ok(Self::Nanosecond),
            [other] => Err(DataTypeError::InvalidBytes {
                message: format!("unknown time unit tag {other}, expected 0, 1, 2 or 3"),
            }),
            _ => Err(DataTypeError::InvalidByteLength {
                expected: 1,
                actual: bytes.len(),
            }),
        }
    }
}

impl fmt::Display for TimeUnit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Second => "s",
            Self::Millisecond => "ms",
            Self::Microsecond => "us",
            Self::Nanosecond => "ns",
        })
    }
}
