//! The timestamp implementation covering every time unit.

use core::fmt;
use core::str;
use std::collections::BTreeMap;
use std::sync::Arc;

use arrow_schema::DataType as ArrowDataType;

use crate::{
    metadata, DataType, DataTypeError, DataTypeId, Int64, LogicalType, PrimitiveType, TimeUnit,
    TimeUnitId, Timestamp,
};

/// The concrete [`Timestamp`] implementation generic over its unit — the one
/// implementation that gives every [`TimeUnit`] its corresponding timestamp
/// (`TypedTimestamp<Nanosecond>` through `TypedTimestamp<Year>`), anchored on
/// [`Int64`].
///
/// Arrow's four native units map to `Timestamp(unit, timezone)` directly.
/// The coarser units (minute through year) have no Arrow counterpart, so
/// they anchor on Arrow `Int64` plus the `ygg.type` / `ygg.time_unit` /
/// `ygg.timezone` field metadata that restores the semantics losslessly —
/// convert those through a [`Field`](crate::Field), which carries the
/// metadata.
///
/// ```
/// use yggdryl_schema::{DataType, Millisecond, Minute, Timestamp, TypedTimestamp};
///
/// let millis = TypedTimestamp::from_parts(Millisecond, Some("UTC".into()));
/// assert_eq!(TypedTimestamp::from_arrow(&millis.to_arrow()), Ok(millis));
///
/// let minutes = TypedTimestamp::from_parts(Minute, None);
/// assert_eq!(minutes.to_arrow(), arrow_schema::DataType::Int64); // anchored
/// assert_eq!(minutes.to_string(), "timestamp(min)");
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TypedTimestamp<U: TimeUnit> {
    unit: U,
    timezone: Option<Arc<str>>,
}

impl<U: TimeUnit> Timestamp for TypedTimestamp<U> {
    type Unit = U;

    fn from_parts(unit: U, timezone: Option<Arc<str>>) -> Self {
        Self { unit, timezone }
    }

    fn unit(&self) -> U {
        self.unit.clone()
    }

    fn timezone(&self) -> Option<&str> {
        self.timezone.as_deref()
    }

    fn copy(&self, unit: Option<U>, timezone: Option<Option<Arc<str>>>) -> Self {
        // Overrides the provided default so a kept timezone stays a cheap
        // `Arc` clone instead of a fresh copy.
        Self::from_parts(
            unit.unwrap_or_else(|| self.unit.clone()),
            timezone.unwrap_or_else(|| self.timezone.clone()),
        )
    }
}

impl<U: TimeUnit> DataType for TypedTimestamp<U> {
    fn type_id(&self) -> DataTypeId {
        DataTypeId::Timestamp
    }

    fn to_arrow(&self) -> ArrowDataType {
        match self.unit.to_arrow() {
            Some(unit) => ArrowDataType::Timestamp(unit, self.timezone.clone()),
            // Arrow lacks this unit: anchor on the physical type and restore
            // the semantics through `arrow_metadata`.
            None => ArrowDataType::Int64,
        }
    }

    fn arrow_metadata(&self) -> BTreeMap<String, String> {
        let mut restored = BTreeMap::new();
        if self.unit.to_arrow().is_none() {
            restored.insert(metadata::TYPE.to_owned(), "timestamp".to_owned());
            restored.insert(
                metadata::TIME_UNIT.to_owned(),
                self.unit.unit_id().metadata_value().to_owned(),
            );
            if let Some(timezone) = &self.timezone {
                restored.insert(metadata::TIMEZONE.to_owned(), timezone.to_string());
            }
        }
        restored
    }

    fn from_arrow(data_type: &ArrowDataType) -> Result<Self, DataTypeError> {
        match data_type {
            ArrowDataType::Timestamp(unit, timezone) => Ok(Self::from_parts(
                U::from_unit_id(TimeUnitId::from_arrow(*unit))?,
                timezone.clone(),
            )),
            // A bare Int64 is never a timestamp on its own; the unit lives in
            // the field metadata.
            ArrowDataType::Int64 => Err(DataTypeError::MissingMetadata {
                key: metadata::TIME_UNIT,
            }),
            other => Err(DataTypeError::ArrowTypeMismatch {
                expected: "timestamp",
                actual: other.clone(),
            }),
        }
    }

    fn from_arrow_parts(
        data_type: &ArrowDataType,
        metadata_map: &BTreeMap<String, String>,
    ) -> Result<Self, DataTypeError> {
        if let Some(key) = metadata_map.keys().find(|key| {
            key.starts_with(metadata::PREFIX)
                && ![metadata::TYPE, metadata::TIME_UNIT, metadata::TIMEZONE]
                    .contains(&key.as_str())
        }) {
            return Err(DataTypeError::UnknownMetadata { key: key.clone() });
        }
        match metadata_map.get(metadata::TYPE).map(String::as_str) {
            None if !metadata_map.contains_key(metadata::TIME_UNIT) => Self::from_arrow(data_type),
            None => Err(DataTypeError::MissingMetadata {
                key: metadata::TYPE,
            }),
            Some("timestamp") => {
                if data_type != &ArrowDataType::Int64 {
                    return Err(DataTypeError::ArrowTypeMismatch {
                        expected: "int64 (anchoring a timestamp)",
                        actual: data_type.clone(),
                    });
                }
                let value = metadata_map.get(metadata::TIME_UNIT).ok_or(
                    DataTypeError::MissingMetadata {
                        key: metadata::TIME_UNIT,
                    },
                )?;
                let unit_id = TimeUnitId::from_metadata_value(value).ok_or_else(|| {
                    DataTypeError::InvalidMetadata {
                        key: metadata::TIME_UNIT,
                        value: value.clone(),
                    }
                })?;
                Ok(Self::from_parts(
                    U::from_unit_id(unit_id)?,
                    metadata_map
                        .get(metadata::TIMEZONE)
                        .map(|timezone| Arc::from(timezone.as_str())),
                ))
            }
            Some(other) => Err(DataTypeError::InvalidMetadata {
                key: metadata::TYPE,
                value: other.to_owned(),
            }),
        }
    }

    fn to_bytes(&self) -> Vec<u8> {
        // `unit tag | timezone flag | timezone UTF-8` — the flag disambiguates
        // a missing timezone from an empty string.
        let mut out = self.unit.unit_id().to_bytes();
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
        let unit = U::from_unit_id(TimeUnitId::from_bytes(&[*unit])?)?;
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

impl<U: TimeUnit> PrimitiveType for TypedTimestamp<U> {
    type Native = i64;
    const BIT_WIDTH: usize = 64;
}

impl<U: TimeUnit> LogicalType for TypedTimestamp<U> {
    type Physical = Int64;

    fn physical(&self) -> Int64 {
        Int64
    }
}

impl<U: TimeUnit> fmt::Display for TypedTimestamp<U> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.timezone {
            Some(timezone) => write!(f, "timestamp({}, {timezone})", self.unit),
            None => write!(f, "timestamp({})", self.unit),
        }
    }
}
