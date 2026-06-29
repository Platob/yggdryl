//! The [`Field`] trait — a named, typed, nullable column with byte-keyed metadata.

use std::collections::BTreeMap;

use crate::data_type::DataType;

/// Column metadata: an ordered map of opaque byte-string key/value pairs.
///
/// `BTreeMap` keeps the entries ordered so the map hashes and serializes
/// deterministically.
pub type Metadata = BTreeMap<Vec<u8>, Vec<u8>>;

/// A named column: a [`name`](Field::name), its [`dtype`](Field::dtype), whether it
/// is [`nullable`](Field::nullable), and its [`metadata`](Field::metadata).
///
/// [`from_parts`](Field::from_parts) is the single constructor primitive; the
/// functional [`copy`](Field::copy) and the `with_*` helpers are one-line
/// delegations to it. Under the `arrow` feature, [`to_arrow`](Field::to_arrow) /
/// [`from_arrow`](Field::from_arrow) bridge to Apache Arrow, leveraging
/// [`DataType::to_arrow`] / [`DataType::from_arrow`] for the data type.
///
/// ```
/// use yggdryl_schema::{DataType, DataTypeId, Field, Metadata};
///
/// #[derive(Clone)]
/// struct Int32;
/// impl DataType for Int32 {
///     fn name(&self) -> &'static str { "int32" }
///     fn type_id(&self) -> DataTypeId { DataTypeId::Int32 }
/// #     #[cfg(feature = "arrow")]
/// #     fn to_arrow(&self) -> arrow_schema::DataType { arrow_schema::DataType::Int32 }
/// #     #[cfg(feature = "arrow")]
/// #     fn from_arrow(_dtype: &arrow_schema::DataType) -> Result<Self, yggdryl_schema::SchemaError> { Ok(Int32) }
/// }
///
/// struct Col { name: String, dtype: Int32, nullable: bool, metadata: Metadata }
/// impl Field for Col {
///     type Type = Int32;
///     fn name(&self) -> &str { &self.name }
///     fn dtype(&self) -> &Int32 { &self.dtype }
///     fn nullable(&self) -> bool { self.nullable }
///     fn metadata(&self) -> &Metadata { &self.metadata }
///     fn from_parts(name: String, dtype: Int32, nullable: bool, metadata: Metadata) -> Self {
///         Col { name, dtype, nullable, metadata }
///     }
/// }
///
/// let col = Col::from_parts("id".into(), Int32, false, Metadata::new());
/// assert!(col.with_nullable(true).nullable());
/// assert_eq!(col.with_name("key".into()).name(), "key");
/// ```
pub trait Field {
    /// The concrete [`DataType`] this field's values carry.
    type Type: DataType + Clone;

    /// The column name.
    fn name(&self) -> &str;

    /// The column's data type.
    fn dtype(&self) -> &Self::Type;

    /// Whether the column admits nulls.
    fn nullable(&self) -> bool;

    /// The column's key/value metadata.
    fn metadata(&self) -> &Metadata;

    /// Builds a field from its parts — the single constructor the other helpers
    /// build on.
    fn from_parts(name: String, dtype: Self::Type, nullable: bool, metadata: Metadata) -> Self
    where
        Self: Sized;

    /// Returns a copy with the given components overridden; each `None` is taken
    /// from `self`.
    fn copy(
        &self,
        name: Option<String>,
        dtype: Option<Self::Type>,
        nullable: Option<bool>,
        metadata: Option<Metadata>,
    ) -> Self
    where
        Self: Sized,
    {
        Self::from_parts(
            name.unwrap_or_else(|| self.name().to_owned()),
            dtype.unwrap_or_else(|| self.dtype().clone()),
            nullable.unwrap_or_else(|| self.nullable()),
            metadata.unwrap_or_else(|| self.metadata().clone()),
        )
    }

    /// Returns a copy with a new name.
    fn with_name(&self, name: String) -> Self
    where
        Self: Sized,
    {
        self.copy(Some(name), None, None, None)
    }

    /// Returns a copy with a new data type.
    fn with_dtype(&self, dtype: Self::Type) -> Self
    where
        Self: Sized,
    {
        self.copy(None, Some(dtype), None, None)
    }

    /// Returns a copy with a new nullability.
    fn with_nullable(&self, nullable: bool) -> Self
    where
        Self: Sized,
    {
        self.copy(None, None, Some(nullable), None)
    }

    /// Returns a copy with new metadata.
    fn with_metadata(&self, metadata: Metadata) -> Self
    where
        Self: Sized,
    {
        self.copy(None, None, None, Some(metadata))
    }

    /// Converts this field to an Apache Arrow [`Field`](arrow_schema::Field),
    /// leveraging [`DataType::to_arrow`]. Errors if any metadata key or value is
    /// not valid UTF-8 (Arrow field metadata is string-keyed).
    #[cfg(feature = "arrow")]
    fn to_arrow(&self) -> Result<arrow_schema::Field, crate::SchemaError>
    where
        Self: Sized,
    {
        let metadata = self
            .metadata()
            .iter()
            .map(|(key, value)| {
                let key = String::from_utf8(key.clone())
                    .map_err(|_| crate::SchemaError::NonUtf8Metadata)?;
                let value = String::from_utf8(value.clone())
                    .map_err(|_| crate::SchemaError::NonUtf8Metadata)?;
                Ok((key, value))
            })
            .collect::<Result<std::collections::HashMap<String, String>, crate::SchemaError>>()?;
        Ok(
            arrow_schema::Field::new(self.name(), self.dtype().to_arrow(), self.nullable())
                .with_metadata(metadata),
        )
    }

    /// Builds a field from an Apache Arrow [`Field`](arrow_schema::Field),
    /// leveraging [`DataType::from_arrow`].
    #[cfg(feature = "arrow")]
    fn from_arrow(field: &arrow_schema::Field) -> Result<Self, crate::SchemaError>
    where
        Self: Sized,
    {
        let dtype = Self::Type::from_arrow(field.data_type())?;
        let metadata = field
            .metadata()
            .iter()
            .map(|(key, value)| (key.clone().into_bytes(), value.clone().into_bytes()))
            .collect();
        Ok(Self::from_parts(
            field.name().to_owned(),
            dtype,
            field.is_nullable(),
            metadata,
        ))
    }
}
