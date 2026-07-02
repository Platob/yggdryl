//! A named, typed slot in a schema.

use core::fmt;
use std::collections::BTreeMap;
use std::sync::Arc;

use arrow_schema::Field as ArrowField;

use crate::bytes::{put_len, put_str, Reader};
use crate::{DataType, FieldError};

/// The shared handle to a [`Field`]; the `Arc` clone is the cheap sharing
/// mechanism.
pub type FieldRef<T> = Arc<Field<T>>;

/// A named, typed slot in a schema: a name, a data type `T`, a nullability
/// flag and free-form metadata, mapping to Arrow `Field`.
///
/// Metadata is a `BTreeMap` (not a `HashMap`) so iteration order — and with
/// it `Hash` and the byte encoding — is deterministic.
///
/// ```
/// use yggdryl_schema::{Field, Int32};
///
/// let field = Field::from_parts("id", Int32, false, Default::default());
/// let arrow = field.to_arrow();
/// assert_eq!(Field::from_arrow(&arrow), Ok(field));
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Field<T: DataType> {
    name: String,
    data_type: T,
    nullable: bool,
    metadata: BTreeMap<String, String>,
}

impl<T: DataType> Field<T> {
    /// Builds the field from its parts; any name — including the empty
    /// string — is valid.
    pub fn from_parts(
        name: impl Into<String>,
        data_type: T,
        nullable: bool,
        metadata: BTreeMap<String, String>,
    ) -> Self {
        Self {
            name: name.into(),
            data_type,
            nullable,
            metadata,
        }
    }

    /// The field's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The field's data type.
    pub fn data_type(&self) -> &T {
        &self.data_type
    }

    /// Whether values of the field may be null.
    pub fn nullable(&self) -> bool {
        self.nullable
    }

    /// The field's free-form metadata.
    pub fn metadata(&self) -> &BTreeMap<String, String> {
        &self.metadata
    }

    /// Returns a copy with any of the parts overridden; omitted parts come
    /// from `self`.
    pub fn copy(
        &self,
        name: Option<String>,
        data_type: Option<T>,
        nullable: Option<bool>,
        metadata: Option<BTreeMap<String, String>>,
    ) -> Self {
        Self::from_parts(
            name.unwrap_or_else(|| self.name.clone()),
            data_type.unwrap_or_else(|| self.data_type.clone()),
            nullable.unwrap_or(self.nullable),
            metadata.unwrap_or_else(|| self.metadata.clone()),
        )
    }

    /// Returns a copy with the name replaced.
    pub fn with_name(&self, name: impl Into<String>) -> Self {
        self.copy(Some(name.into()), None, None, None)
    }

    /// Returns a copy with the data type replaced.
    pub fn with_data_type(&self, data_type: T) -> Self {
        self.copy(None, Some(data_type), None, None)
    }

    /// Returns a copy with the nullability replaced.
    pub fn with_nullable(&self, nullable: bool) -> Self {
        self.copy(None, None, Some(nullable), None)
    }

    /// Returns a copy with the metadata replaced.
    pub fn with_metadata(&self, metadata: BTreeMap<String, String>) -> Self {
        self.copy(None, None, None, Some(metadata))
    }

    /// Returns a copy with the metadata cleared.
    pub fn without_metadata(&self) -> Self {
        self.copy(None, None, None, Some(BTreeMap::new()))
    }

    /// The Arrow field this field maps to.
    pub fn to_arrow(&self) -> ArrowField {
        crate::log_event!(trace, "Field::to_arrow name={}", self.name);
        ArrowField::new(self.name.clone(), self.data_type.to_arrow(), self.nullable).with_metadata(
            self.metadata
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
        )
    }

    /// Validates and converts an Arrow field back into this type — the only
    /// inbound conversion.
    pub fn from_arrow(field: &ArrowField) -> Result<Self, FieldError> {
        crate::log_event!(trace, "Field::from_arrow name={}", field.name());
        Ok(Self::from_parts(
            field.name().to_owned(),
            T::from_arrow(field.data_type())?,
            field.is_nullable(),
            field
                .metadata()
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
        ))
    }

    /// Encodes the field as `nullable | name | data type | metadata`, every
    /// variable-size part length-prefixed.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = vec![u8::from(self.nullable)];
        put_str(&mut out, &self.name);
        let data_type = self.data_type.to_bytes();
        put_len(&mut out, data_type.len());
        out.extend_from_slice(&data_type);
        put_len(&mut out, self.metadata.len());
        for (key, value) in &self.metadata {
            put_str(&mut out, key);
            put_str(&mut out, value);
        }
        out
    }

    /// Deserializes the field from the encoding produced by
    /// [`to_bytes`](Field::to_bytes), validating fully.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, FieldError> {
        let mut reader = Reader::new(bytes);
        let nullable = match reader.take_u8()? {
            0 => false,
            1 => true,
            other => {
                return Err(FieldError::InvalidBytes {
                    message: format!("unknown nullable flag {other}, expected 0 or 1"),
                })
            }
        };
        let name = reader.take_str()?.to_owned();
        let data_type = T::from_bytes(reader.take_len_prefixed()?)?;
        let mut metadata = BTreeMap::new();
        for _ in 0..reader.take_len()? {
            let key = reader.take_str()?.to_owned();
            let value = reader.take_str()?.to_owned();
            metadata.insert(key, value);
        }
        reader.finish()?;
        Ok(Self::from_parts(name, data_type, nullable, metadata))
    }
}

impl<T: DataType> fmt::Display for Field<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.name, self.data_type)?;
        if self.nullable {
            f.write_str("?")?;
        }
        Ok(())
    }
}
