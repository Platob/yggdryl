//! The integer identifier of a time unit.

use core::fmt;

use arrow_schema::TimeUnit as ArrowTimeUnit;

use crate::DataTypeError;

/// The largest identifier currently assigned; update when appending a
/// variant.
const MAX_UNIT_ID: u8 = TimeUnitId::Year as u8;

/// The value-level identifier of a [`TimeUnit`](crate::TimeUnit), shared by
/// erased code paths the same way [`DataTypeId`](crate::DataTypeId) is for
/// data types.
///
/// Arrow's type system knows the four sub-second units; the coarser units
/// ([`Minute`](Self::Minute) through [`Year`](Self::Year)) map to `None` in
/// [`to_arrow`](Self::to_arrow) and anchor on a physical type plus `ygg.*`
/// metadata instead. Discriminants are explicit and append-only, so the ids
/// are stable across versions and safe to persist.
///
/// ```
/// use yggdryl_schema::TimeUnitId;
///
/// assert_eq!(TimeUnitId::Millisecond.to_arrow(), Some(arrow_schema::TimeUnit::Millisecond));
/// assert_eq!(TimeUnitId::Year.to_arrow(), None);
/// assert_eq!(TimeUnitId::from_u8(TimeUnitId::Minute.to_u8()), Ok(TimeUnitId::Minute));
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
#[repr(u8)]
pub enum TimeUnitId {
    /// One second.
    Second = 0,
    /// One thousandth of a second.
    Millisecond = 1,
    /// One millionth of a second.
    Microsecond = 2,
    /// One billionth of a second.
    Nanosecond = 3,
    /// Sixty seconds.
    Minute = 4,
    /// Sixty minutes.
    Hour = 5,
    /// Twenty-four hours.
    Day = 6,
    /// Seven days.
    Week = 7,
    /// One calendar month; not a fixed span of time.
    Month = 8,
    /// Three calendar months; not a fixed span of time.
    Quarter = 9,
    /// One calendar year; not a fixed span of time.
    Year = 10,
}

impl TimeUnitId {
    /// The identifier's stable integer value.
    pub fn to_u8(&self) -> u8 {
        *self as u8
    }

    /// Builds the identifier from its stable integer value, rejecting
    /// unassigned values.
    pub fn from_u8(id: u8) -> Result<Self, DataTypeError> {
        match id {
            0 => Ok(Self::Second),
            1 => Ok(Self::Millisecond),
            2 => Ok(Self::Microsecond),
            3 => Ok(Self::Nanosecond),
            4 => Ok(Self::Minute),
            5 => Ok(Self::Hour),
            6 => Ok(Self::Day),
            7 => Ok(Self::Week),
            8 => Ok(Self::Month),
            9 => Ok(Self::Quarter),
            10 => Ok(Self::Year),
            _ => Err(DataTypeError::UnknownTimeUnitId {
                id,
                max: MAX_UNIT_ID,
            }),
        }
    }

    /// The Arrow time unit this unit maps to, `None` for the units Arrow's
    /// type system lacks (those anchor on a physical type plus `ygg.*`
    /// metadata).
    pub fn to_arrow(&self) -> Option<ArrowTimeUnit> {
        match self {
            Self::Second => Some(ArrowTimeUnit::Second),
            Self::Millisecond => Some(ArrowTimeUnit::Millisecond),
            Self::Microsecond => Some(ArrowTimeUnit::Microsecond),
            Self::Nanosecond => Some(ArrowTimeUnit::Nanosecond),
            _ => None,
        }
    }

    /// Converts an Arrow time unit; total, since Arrow's set is a subset of
    /// ours.
    pub fn from_arrow(unit: ArrowTimeUnit) -> Self {
        match unit {
            ArrowTimeUnit::Second => Self::Second,
            ArrowTimeUnit::Millisecond => Self::Millisecond,
            ArrowTimeUnit::Microsecond => Self::Microsecond,
            ArrowTimeUnit::Nanosecond => Self::Nanosecond,
        }
    }

    /// The unit's fixed span in nanoseconds; `None` for calendar units
    /// (month, quarter, year), whose span depends on the date.
    pub fn fixed_nanoseconds(&self) -> Option<i64> {
        match self {
            Self::Nanosecond => Some(1),
            Self::Microsecond => Some(1_000),
            Self::Millisecond => Some(1_000_000),
            Self::Second => Some(1_000_000_000),
            Self::Minute => Some(60 * 1_000_000_000),
            Self::Hour => Some(3_600 * 1_000_000_000),
            Self::Day => Some(86_400 * 1_000_000_000),
            Self::Week => Some(7 * 86_400 * 1_000_000_000),
            Self::Month | Self::Quarter | Self::Year => None,
        }
    }

    /// Serializes the identifier as its one-byte value.
    pub fn to_bytes(&self) -> Vec<u8> {
        vec![self.to_u8()]
    }

    /// Deserializes the identifier from the encoding produced by
    /// [`to_bytes`](TimeUnitId::to_bytes).
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, DataTypeError> {
        match bytes {
            [id] => Self::from_u8(*id),
            _ => Err(DataTypeError::InvalidByteLength {
                expected: 1,
                actual: bytes.len(),
            }),
        }
    }

    /// The rendered form, doubling as the `ygg.time_unit` metadata value.
    fn as_str(&self) -> &'static str {
        match self {
            Self::Second => "s",
            Self::Millisecond => "ms",
            Self::Microsecond => "us",
            Self::Nanosecond => "ns",
            Self::Minute => "min",
            Self::Hour => "h",
            Self::Day => "d",
            Self::Week => "w",
            Self::Month => "mo",
            Self::Quarter => "q",
            Self::Year => "y",
        }
    }

    /// The `ygg.time_unit` metadata value restoring this unit.
    pub(crate) fn metadata_value(&self) -> &'static str {
        self.as_str()
    }

    /// The inverse of [`metadata_value`](Self::metadata_value), for
    /// `from_arrow` validation only — not a public parsing constructor.
    pub(crate) fn from_metadata_value(value: &str) -> Option<Self> {
        [
            Self::Second,
            Self::Millisecond,
            Self::Microsecond,
            Self::Nanosecond,
            Self::Minute,
            Self::Hour,
            Self::Day,
            Self::Week,
            Self::Month,
            Self::Quarter,
            Self::Year,
        ]
        .into_iter()
        .find(|unit| unit.as_str() == value)
    }
}

impl fmt::Display for TimeUnitId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
