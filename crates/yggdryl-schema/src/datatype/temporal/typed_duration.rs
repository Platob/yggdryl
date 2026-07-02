//! The duration implementation covering every time unit.

use core::fmt;
use std::collections::BTreeMap;

use arrow_schema::DataType as ArrowDataType;

use crate::{
    metadata, DataType, DataTypeError, DataTypeId, Duration, Int64, LogicalType, PrimitiveType,
    TimeUnit, TimeUnitId,
};

/// The concrete [`Duration`] implementation generic over its unit — the one
/// implementation that gives every [`TimeUnit`] its corresponding duration
/// (`TypedDuration<Nanosecond>` through `TypedDuration<Year>`), anchored on
/// [`Int64`].
///
/// Arrow's four native units map to `Duration(unit)` directly. The coarser
/// units have no Arrow counterpart, so they anchor on Arrow `Int64` plus the
/// `ygg.type` / `ygg.time_unit` field metadata that restores the semantics
/// losslessly — convert those through a [`Field`](crate::Field), which
/// carries the metadata.
///
/// ```
/// use yggdryl_schema::{DataType, Duration, Second, TypedDuration, Week};
///
/// let seconds = TypedDuration::from_parts(Second);
/// assert_eq!(TypedDuration::from_arrow(&seconds.to_arrow()), Ok(seconds));
///
/// let weeks = TypedDuration::from_parts(Week);
/// assert_eq!(weeks.to_arrow(), arrow_schema::DataType::Int64); // anchored
/// assert_eq!(weeks.to_string(), "duration(w)");
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TypedDuration<U: TimeUnit> {
    unit: U,
}

impl<U: TimeUnit> Duration for TypedDuration<U> {
    type Unit = U;

    fn from_parts(unit: U) -> Self {
        Self { unit }
    }

    fn unit(&self) -> U {
        self.unit.clone()
    }
}

impl<U: TimeUnit> DataType for TypedDuration<U> {
    fn type_id(&self) -> DataTypeId {
        DataTypeId::Duration
    }

    fn to_arrow(&self) -> ArrowDataType {
        match self.unit.to_arrow() {
            Some(unit) => ArrowDataType::Duration(unit),
            // Arrow lacks this unit: anchor on the physical type and restore
            // the semantics through `arrow_metadata`.
            None => ArrowDataType::Int64,
        }
    }

    fn arrow_metadata(&self) -> BTreeMap<String, String> {
        let mut restored = BTreeMap::new();
        if self.unit.to_arrow().is_none() {
            restored.insert(metadata::TYPE.to_owned(), "duration".to_owned());
            restored.insert(
                metadata::TIME_UNIT.to_owned(),
                self.unit.unit_id().metadata_value().to_owned(),
            );
        }
        restored
    }

    fn from_arrow(data_type: &ArrowDataType) -> Result<Self, DataTypeError> {
        match data_type {
            ArrowDataType::Duration(unit) => Ok(Self::from_parts(U::from_unit_id(
                TimeUnitId::from_arrow(*unit),
            )?)),
            // A bare Int64 is never a duration on its own; the unit lives in
            // the field metadata.
            ArrowDataType::Int64 => Err(DataTypeError::MissingMetadata {
                key: metadata::TIME_UNIT,
            }),
            other => Err(DataTypeError::ArrowTypeMismatch {
                expected: "duration",
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
                && ![metadata::TYPE, metadata::TIME_UNIT].contains(&key.as_str())
        }) {
            return Err(DataTypeError::UnknownMetadata { key: key.clone() });
        }
        match metadata_map.get(metadata::TYPE).map(String::as_str) {
            None if !metadata_map.contains_key(metadata::TIME_UNIT) => Self::from_arrow(data_type),
            None => Err(DataTypeError::MissingMetadata {
                key: metadata::TYPE,
            }),
            Some("duration") => {
                if data_type != &ArrowDataType::Int64 {
                    return Err(DataTypeError::ArrowTypeMismatch {
                        expected: "int64 (anchoring a duration)",
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
                Ok(Self::from_parts(U::from_unit_id(unit_id)?))
            }
            Some(other) => Err(DataTypeError::InvalidMetadata {
                key: metadata::TYPE,
                value: other.to_owned(),
            }),
        }
    }

    fn to_bytes(&self) -> Vec<u8> {
        self.unit.unit_id().to_bytes()
    }

    fn from_bytes(bytes: &[u8]) -> Result<Self, DataTypeError> {
        Ok(Self::from_parts(U::from_unit_id(TimeUnitId::from_bytes(
            bytes,
        )?)?))
    }
}

impl<U: TimeUnit> PrimitiveType for TypedDuration<U> {
    type Native = i64;
    const BIT_WIDTH: usize = 64;
}

impl<U: TimeUnit> LogicalType for TypedDuration<U> {
    type Physical = Int64;

    fn physical(&self) -> Int64 {
        Int64
    }
}

impl<U: TimeUnit> fmt::Display for TypedDuration<U> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "duration({})", self.unit)
    }
}
