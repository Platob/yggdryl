//! The timestamp data type.

use core::fmt;
use core::str;
use std::sync::Arc;

use arrow_schema::DataType as ArrowDataType;

use crate::{DataType, DataTypeError, DataTypeId, Int64, LogicalType, PrimitiveType, TimeUnit};

/// An instant as a 64-bit offset since the UNIX epoch at a given resolution,
/// with an optional timezone, mapping to Arrow `Timestamp(unit, timezone)`
/// and anchored on [`Int64`].
///
/// ```
/// use yggdryl_schema::{DataType, TimeUnit, Timestamp};
///
/// let utc = Timestamp::from_parts(TimeUnit::Millisecond, Some("UTC".into()));
/// assert_eq!(Timestamp::from_arrow(&utc.to_arrow()), Ok(utc));
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Timestamp {
    unit: TimeUnit,
    timezone: Option<Arc<str>>,
}

impl Timestamp {
    /// Builds the type from its resolution and optional timezone.
    pub fn from_parts(unit: TimeUnit, timezone: Option<Arc<str>>) -> Self {
        Self { unit, timezone }
    }

    /// The resolution of the offset.
    pub fn unit(&self) -> TimeUnit {
        self.unit
    }

    /// The timezone the instant is rendered in, if any.
    pub fn timezone(&self) -> Option<&str> {
        self.timezone.as_deref()
    }

    /// Returns a copy with any of the parts overridden; omitted parts come
    /// from `self`.
    pub fn copy(&self, unit: Option<TimeUnit>, timezone: Option<Option<Arc<str>>>) -> Self {
        Self::from_parts(
            unit.unwrap_or(self.unit),
            timezone.unwrap_or_else(|| self.timezone.clone()),
        )
    }

    /// Returns a copy with the resolution replaced.
    pub fn with_unit(&self, unit: TimeUnit) -> Self {
        self.copy(Some(unit), None)
    }

    /// Returns a copy with the timezone replaced.
    pub fn with_timezone(&self, timezone: impl Into<Arc<str>>) -> Self {
        self.copy(None, Some(Some(timezone.into())))
    }

    /// Returns a copy with the timezone cleared.
    pub fn without_timezone(&self) -> Self {
        self.copy(None, Some(None))
    }
}

impl DataType for Timestamp {
    const TYPE_ID: DataTypeId = DataTypeId::Timestamp;

    fn to_arrow(&self) -> ArrowDataType {
        ArrowDataType::Timestamp(self.unit.to_arrow(), self.timezone.clone())
    }

    fn from_arrow(data_type: &ArrowDataType) -> Result<Self, DataTypeError> {
        match data_type {
            ArrowDataType::Timestamp(unit, timezone) => Ok(Self::from_parts(
                TimeUnit::from_arrow(*unit),
                timezone.clone(),
            )),
            other => Err(DataTypeError::ArrowTypeMismatch {
                expected: "timestamp",
                actual: other.clone(),
            }),
        }
    }

    fn to_bytes(&self) -> Vec<u8> {
        // `unit tag | timezone flag | timezone UTF-8` — the flag disambiguates
        // a missing timezone from an empty string.
        let mut out = self.unit.to_bytes();
        match &self.timezone {
            Some(timezone) => {
                out.push(1);
                out.extend_from_slice(timezone.as_bytes());
            }
            None => out.push(0),
        }
        out
    }

    fn from_bytes(bytes: &[u8]) -> Result<Self, DataTypeError> {
        let [unit, has_timezone, timezone @ ..] = bytes else {
            return Err(DataTypeError::InvalidByteLength {
                expected: 2,
                actual: bytes.len(),
            });
        };
        let unit = TimeUnit::from_bytes(&[*unit])?;
        let timezone = match has_timezone {
            0 if timezone.is_empty() => None,
            0 => {
                return Err(DataTypeError::InvalidBytes {
                    message: format!(
                        "{} trailing bytes after a timezone-less timestamp",
                        timezone.len()
                    ),
                })
            }
            1 => Some(
                str::from_utf8(timezone)
                    .map_err(|_| DataTypeError::InvalidBytes {
                        message: "timestamp timezone is not valid UTF-8".to_string(),
                    })?
                    .into(),
            ),
            other => {
                return Err(DataTypeError::InvalidBytes {
                    message: format!("unknown timezone flag {other}, expected 0 or 1"),
                })
            }
        };
        Ok(Self::from_parts(unit, timezone))
    }
}

impl PrimitiveType for Timestamp {
    type Native = i64;
    const BIT_WIDTH: usize = 64;
}

impl LogicalType for Timestamp {
    type Physical = Int64;

    fn physical(&self) -> Int64 {
        Int64
    }
}

impl fmt::Display for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.timezone {
            Some(timezone) => write!(f, "timestamp({}, {timezone})", self.unit),
            None => write!(f, "timestamp({})", self.unit),
        }
    }
}
