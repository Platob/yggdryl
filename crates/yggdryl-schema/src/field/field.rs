//! The abstract base every field implementation satisfies.

use core::fmt::{Debug, Display};
use core::hash::Hash;
use std::collections::BTreeMap;

use arrow_schema::Field as ArrowField;

use crate::bytes::{put_len, put_str, Reader};
use crate::{DataType, FieldError};

/// A named, typed slot in a schema: the abstract base tying a name, a
/// [`DataType`], a nullability flag and free-form metadata together.
///
/// Implementors supply [`from_parts`](Field::from_parts) and the four
/// accessors — everything else (functional updates, the Arrow mapping and the
/// byte round-trip) is provided on top of them. The generic
/// [`TypedField`](crate::TypedField) is the implementation covering every
/// data type; specialised field types implement the same trait in one-line
/// methods.
///
/// ```
/// use yggdryl_schema::{Field, Int32, TypedField};
///
/// let field = TypedField::from_parts("id", Int32, false, Default::default());
/// assert_eq!(field.name(), "id");
/// assert_eq!(TypedField::from_arrow(&field.to_arrow()), Ok(field));
/// ```
pub trait Field: Clone + Debug + Display + Eq + Hash + Send + Sync + Sized + 'static {
    /// The data type this field describes.
    type DataType: DataType;

    /// Builds the field from its parts; any name — including the empty
    /// string — is valid.
    fn from_parts(
        name: impl Into<String>,
        data_type: Self::DataType,
        nullable: bool,
        metadata: BTreeMap<String, String>,
    ) -> Self;

    /// The field's name.
    fn name(&self) -> &str;

    /// The field's data type.
    fn data_type(&self) -> &Self::DataType;

    /// Whether values of the field may be null.
    fn nullable(&self) -> bool;

    /// The field's free-form metadata.
    fn metadata(&self) -> &BTreeMap<String, String>;

    /// Returns a copy with any of the parts overridden; omitted parts come
    /// from `self`.
    fn copy(
        &self,
        name: Option<String>,
        data_type: Option<Self::DataType>,
        nullable: Option<bool>,
        metadata: Option<BTreeMap<String, String>>,
    ) -> Self {
        Self::from_parts(
            name.unwrap_or_else(|| self.name().to_owned()),
            data_type.unwrap_or_else(|| self.data_type().clone()),
            nullable.unwrap_or_else(|| self.nullable()),
            metadata.unwrap_or_else(|| self.metadata().clone()),
        )
    }

    /// Returns a copy with the name replaced.
    fn with_name(&self, name: impl Into<String>) -> Self {
        self.copy(Some(name.into()), None, None, None)
    }

    /// Returns a copy with the data type replaced.
    fn with_data_type(&self, data_type: Self::DataType) -> Self {
        self.copy(None, Some(data_type), None, None)
    }

    /// Returns a copy with the nullability replaced.
    fn with_nullable(&self, nullable: bool) -> Self {
        self.copy(None, None, Some(nullable), None)
    }

    /// Returns a copy with the metadata replaced.
    fn with_metadata(&self, metadata: BTreeMap<String, String>) -> Self {
        self.copy(None, None, None, Some(metadata))
    }

    /// Returns a copy with the metadata cleared.
    fn without_metadata(&self) -> Self {
        self.copy(None, None, None, Some(BTreeMap::new()))
    }

    /// The Arrow field this field maps to. The data type's
    /// [`arrow_metadata`](crate::DataType::arrow_metadata) is merged in — the
    /// `ygg.*` prefix is reserved for it, so user metadata under that prefix
    /// is overwritten.
    fn to_arrow(&self) -> ArrowField {
        crate::log_event!(trace, "Field::to_arrow name={}", self.name());
        let mut metadata: std::collections::HashMap<String, String> = self
            .metadata()
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        metadata.extend(self.data_type().arrow_metadata());
        ArrowField::new(
            self.name().to_owned(),
            self.data_type().to_arrow(),
            self.nullable(),
        )
        .with_metadata(metadata)
    }

    /// Validates and converts an Arrow field back into this type — the only
    /// inbound conversion. The field metadata is handed to
    /// [`from_arrow_parts`](crate::DataType::from_arrow_parts) so anchored
    /// types restore themselves; the consumed `ygg.*` keys stay out of the
    /// field's own metadata.
    fn from_arrow(field: &ArrowField) -> Result<Self, FieldError> {
        crate::log_event!(trace, "Field::from_arrow name={}", field.name());
        let metadata: BTreeMap<String, String> = field
            .metadata()
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        let data_type = Self::DataType::from_arrow_parts(field.data_type(), &metadata)?;
        Ok(Self::from_parts(
            field.name().to_owned(),
            data_type,
            field.is_nullable(),
            metadata
                .into_iter()
                .filter(|(key, _)| !key.starts_with(crate::metadata::PREFIX))
                .collect(),
        ))
    }

    /// Encodes the field as `nullable | name | data type | metadata`, every
    /// variable-size part length-prefixed.
    fn to_bytes(&self) -> Vec<u8> {
        let mut out = vec![u8::from(self.nullable())];
        put_str(&mut out, self.name());
        let data_type = self.data_type().to_bytes();
        put_len(&mut out, data_type.len());
        out.extend_from_slice(&data_type);
        put_len(&mut out, self.metadata().len());
        for (key, value) in self.metadata() {
            put_str(&mut out, key);
            put_str(&mut out, value);
        }
        out
    }

    /// Deserializes the field from the encoding produced by
    /// [`to_bytes`](Field::to_bytes), validating fully.
    fn from_bytes(bytes: &[u8]) -> Result<Self, FieldError> {
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
        let data_type = Self::DataType::from_bytes(reader.take_len_prefixed()?)?;
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
