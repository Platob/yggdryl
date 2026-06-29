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
/// [`copy`](Field::copy) is the single functional-update primitive — it rebuilds
/// the field with selected components overridden, taking the rest from `self`. The
/// `with_*` helpers are one-line delegations to it.
///
/// ```
/// use yggdryl_schema::{DataType, DataTypeId, Field, Metadata};
///
/// #[derive(Clone)]
/// struct Int32;
/// impl DataType for Int32 {
///     fn name(&self) -> &'static str { "int32" }
///     fn type_id(&self) -> DataTypeId { DataTypeId::Int32 }
/// }
///
/// struct Col { name: String, dtype: Int32, nullable: bool, metadata: Metadata }
/// impl Field for Col {
///     type Type = Int32;
///     fn name(&self) -> &str { &self.name }
///     fn dtype(&self) -> &Int32 { &self.dtype }
///     fn nullable(&self) -> bool { self.nullable }
///     fn metadata(&self) -> &Metadata { &self.metadata }
///     fn copy(&self, name: Option<String>, dtype: Option<Int32>, nullable: Option<bool>, metadata: Option<Metadata>) -> Self {
///         Col {
///             name: name.unwrap_or_else(|| self.name.clone()),
///             dtype: dtype.unwrap_or_else(|| self.dtype.clone()),
///             nullable: nullable.unwrap_or(self.nullable),
///             metadata: metadata.unwrap_or_else(|| self.metadata.clone()),
///         }
///     }
/// }
///
/// let col = Col { name: "id".into(), dtype: Int32, nullable: false, metadata: Metadata::new() };
/// assert!(col.with_nullable(true).nullable());
/// assert_eq!(col.with_name("key".into()).name(), "key");
/// ```
pub trait Field {
    /// The concrete [`DataType`] this field's values carry.
    type Type: DataType;

    /// The column name.
    fn name(&self) -> &str;

    /// The column's data type.
    fn dtype(&self) -> &Self::Type;

    /// Whether the column admits nulls.
    fn nullable(&self) -> bool;

    /// The column's key/value metadata.
    fn metadata(&self) -> &Metadata;

    /// Returns a copy with the given components overridden; each `None` is taken
    /// from `self`. The single primitive the `with_*` helpers delegate to.
    fn copy(
        &self,
        name: Option<String>,
        dtype: Option<Self::Type>,
        nullable: Option<bool>,
        metadata: Option<Metadata>,
    ) -> Self
    where
        Self: Sized;

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
}
