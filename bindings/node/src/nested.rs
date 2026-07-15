//! The `yggdryl.types` namespace's **nested (composite) layer** — `StructField` (the centralized
//! struct schema) and `StructSerie` (a nullable struct column of heterogeneous child columns),
//! mirroring `yggdryl_core::io::nested`.
//!
//! A `StructField` is a value type (with `equals` / `hashCode` and a byte codec) describing an
//! ordered, named set of child fields (each a `Field` or a nested `StructField`). A `StructSerie`
//! is a struct column whose children are the crate's existing `Serie` columns, erased through the
//! core's `AnySerie`. Because napi cannot accept an arbitrary one-of-many class instance, a
//! `StructSerie` is assembled from a `StructField` **schema** plus each child's canonical
//! `serializeBytes()` frame — the same cross-language wire form used everywhere — so it round-trips
//! byte-for-byte with the Rust core and the Python extension.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use napi::bindgen_prelude::Buffer;
use napi::Either;
use napi_derive::napi;

use yggdryl_core::io::nested::{StructField as CoreStructField, StructSerie as CoreStructSerie};
use yggdryl_core::io::{read_any_leaf, AnyField, AnySerie, Bytes, DataTypeId};

use crate::types::{DataType, Field};

/// Maps any core error to a thrown JS `Error` (its guided text passes through unchanged).
fn to_error(error: impl std::fmt::Display) -> napi::Error {
    napi::Error::from_reason(error.to_string())
}

/// A Java-style `i32` content hash, folding the 64-bit hash halves.
fn java_hash<T: Hash>(value: &T) -> i32 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    let hash = hasher.finish();
    (hash as u32 ^ (hash >> 32) as u32) as i32
}

/// A `Field` (leaf) or `StructField` (nested) → an erased [`AnyField`].
fn to_any_field(field: Either<&Field, &StructField>) -> AnyField {
    match field {
        Either::A(leaf) => AnyField::leaf(leaf.inner.clone()),
        Either::B(nested) => nested.inner.as_any_field().clone(),
    }
}

/// An erased [`AnyField`] → its concrete `Field` / `StructField`.
fn from_any_field(field: &AnyField) -> Either<Field, StructField> {
    if field.is_struct() {
        let inner = CoreStructField::from_any_field(field.clone())
            .expect("a struct AnyField rebuilds a StructField");
        Either::B(StructField { inner })
    } else {
        let inner = field
            .as_leaf()
            .expect("a non-struct AnyField is a leaf")
            .clone();
        Either::A(Field { inner })
    }
}

/// The **centralized struct schema** — a name, nullability, metadata, and an ordered list of child
/// fields (each a `Field` or nested `StructField`).
#[napi(namespace = "types")]
pub struct StructField {
    pub(crate) inner: CoreStructField,
}

#[napi(namespace = "types")]
impl StructField {
    /// A struct schema from a name, its ordered child fields, and its nullability (default `true`).
    #[napi(constructor)]
    pub fn new(
        name: String,
        fields: Vec<Either<&Field, &StructField>>,
        nullable: Option<bool>,
    ) -> Self {
        let children = fields.into_iter().map(to_any_field).collect();
        Self {
            inner: CoreStructField::new(&name, children, nullable.unwrap_or(true)),
        }
    }

    /// The struct's name.
    #[napi(getter)]
    pub fn name(&self) -> String {
        self.inner.name().to_string()
    }

    /// Whether the struct column admits nulls.
    #[napi(getter)]
    pub fn nullable(&self) -> bool {
        self.inner.nullable()
    }

    /// The element type's name (`"struct"`).
    #[napi(getter)]
    pub fn type_name(&self) -> &'static str {
        "struct"
    }

    /// This schema's [`DataType`].
    #[napi(getter)]
    pub fn data_type(&self) -> DataType {
        DataType::of(DataTypeId::Struct)
    }

    /// The number of child fields.
    #[napi(getter)]
    pub fn num_fields(&self) -> u32 {
        self.inner.num_fields() as u32
    }

    /// The child field at `index` as a `Field` / `StructField`; throws out of range.
    #[napi]
    pub fn field(&self, index: u32) -> napi::Result<Either<Field, StructField>> {
        self.inner
            .field(index as usize)
            .map(from_any_field)
            .ok_or_else(|| to_error("StructField index out of range"))
    }

    /// The child field named `name`, or `null`.
    #[napi]
    pub fn field_named(&self, name: String) -> Option<Either<Field, StructField>> {
        self.inner.field_named(&name).map(from_any_field)
    }

    /// The 0-based index of the child field named `name`, or `null`.
    #[napi]
    pub fn index_of(&self, name: String) -> Option<u32> {
        self.inner.index_of(&name).map(|index| index as u32)
    }

    /// The child fields, in order, as `Field` / `StructField`.
    #[napi]
    pub fn fields(&self) -> Vec<Either<Field, StructField>> {
        self.inner.fields().iter().map(from_any_field).collect()
    }

    /// A fresh schema renamed to `name`.
    #[napi]
    pub fn with_name(&self, name: String) -> Self {
        Self {
            inner: self.inner.with_name(&name),
        }
    }

    /// A fresh schema with `nullable` set.
    #[napi]
    pub fn with_nullable(&self, nullable: bool) -> Self {
        Self {
            inner: self.inner.with_nullable(nullable),
        }
    }

    /// A fresh schema with one more child field appended.
    #[napi]
    pub fn with_field(&self, field: Either<&Field, &StructField>) -> Self {
        Self {
            inner: self.inner.with_field(to_any_field(field)),
        }
    }

    /// A fresh schema with one extra `key = value` metadata entry.
    #[napi]
    pub fn with_metadata_entry(&self, key: String, value: String) -> Self {
        Self {
            inner: self.inner.with_metadata_entry(&key, &value),
        }
    }

    /// This schema's canonical bytes (schema tree codec, Arrow-independent).
    #[napi]
    pub fn serialize_bytes(&self) -> Buffer {
        self.inner.as_any_field().serialize_bytes().into()
    }

    /// Reconstructs a schema from [`serializeBytes`](Self::serialize_bytes).
    #[napi(factory)]
    pub fn deserialize_bytes(bytes: Buffer) -> napi::Result<Self> {
        let field = AnyField::deserialize_bytes(&bytes).map_err(to_error)?;
        CoreStructField::from_any_field(field)
            .map(|inner| Self { inner })
            .ok_or_else(|| to_error("the bytes did not decode to a struct field"))
    }

    /// Value equality (content, metadata included).
    #[napi]
    pub fn equals(&self, other: &StructField) -> bool {
        self.inner == other.inner
    }

    /// A content hash (equal schemas hash equal).
    #[napi]
    pub fn hash_code(&self) -> i32 {
        java_hash(&self.inner)
    }

    /// An explicit copy.
    #[napi]
    pub fn copy(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }

    #[napi(js_name = "toString")]
    pub fn text(&self) -> String {
        format!(
            "StructField(name={:?}, numFields={}, nullable={})",
            self.inner.name(),
            self.inner.num_fields(),
            self.inner.nullable()
        )
    }
}

/// A **nullable struct column** — one child column per field (all the same length), an ordered
/// schema, and an optional top-level validity mask (a null struct row).
#[napi(namespace = "types")]
pub struct StructSerie {
    pub(crate) inner: CoreStructSerie,
}

#[napi(namespace = "types")]
impl StructSerie {
    /// A struct column from a `schema` and each child column's `serializeBytes()` frame, in field
    /// order. (napi cannot accept an arbitrary one-of-many `Serie` instance, so a child crosses as
    /// its canonical bytes — build them with `serie.serializeBytes()` / `serie.toField(name)`.)
    #[napi(factory)]
    pub fn from_columns(schema: &StructField, columns: Vec<Buffer>) -> napi::Result<Self> {
        let fields = schema.inner.fields();
        if fields.len() != columns.len() {
            return Err(to_error(format!(
                "the schema has {} fields but {} column frames were given",
                fields.len(),
                columns.len()
            )));
        }
        let mut cols: Vec<Box<dyn AnySerie>> = Vec::with_capacity(fields.len());
        for (field, bytes) in fields.iter().zip(&columns) {
            let column: Box<dyn AnySerie> = if field.is_struct() {
                Box::new(CoreStructSerie::deserialize_bytes(bytes).map_err(to_error)?)
            } else {
                read_any_leaf(field, &mut Bytes::from_slice(bytes)).map_err(to_error)?
            };
            cols.push(column);
        }
        CoreStructSerie::from_columns(fields.to_vec(), cols, None)
            .map(|inner| Self { inner })
            .map_err(to_error)
    }

    /// The number of rows.
    #[napi(getter)]
    pub fn length(&self) -> u32 {
        self.inner.len() as u32
    }

    /// The number of child columns (fields).
    #[napi(getter)]
    pub fn num_columns(&self) -> u32 {
        self.inner.num_columns() as u32
    }

    /// The number of null struct rows.
    #[napi(getter)]
    pub fn null_count(&self) -> u32 {
        CoreStructSerie::null_count(&self.inner) as u32
    }

    /// Whether any struct row is null.
    #[napi(getter)]
    pub fn has_nulls(&self) -> bool {
        self.inner.has_nulls()
    }

    /// Whether the column has no rows.
    #[napi]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// This column's [`DataType`].
    #[napi(getter)]
    pub fn data_type(&self) -> DataType {
        DataType::of(DataTypeId::Struct)
    }

    /// A [`StructField`] naming this struct column (nullability inferred from its null rows).
    #[napi]
    pub fn to_field(&self, name: String) -> StructField {
        StructField {
            inner: self.inner.to_field(&name),
        }
    }

    /// The child field at `index` as a `Field` / `StructField`; throws out of range.
    #[napi]
    pub fn field(&self, index: u32) -> napi::Result<Either<Field, StructField>> {
        self.inner
            .field(index as usize)
            .map(from_any_field)
            .ok_or_else(|| to_error("StructSerie field index out of range"))
    }

    /// The child column at `index` as its canonical bytes — reconstruct it with the matching
    /// `Serie.deserializeBytes(...)` (its type is `field(index).typeName`). Throws out of range.
    #[napi]
    pub fn column_bytes(&self, index: u32) -> napi::Result<Buffer> {
        self.inner
            .column(index as usize)
            .map(|column| column.serialize_bytes().into())
            .ok_or_else(|| to_error("StructSerie column index out of range"))
    }

    /// The child column named `name` as its canonical bytes, or `null`.
    #[napi]
    pub fn column_bytes_named(&self, name: String) -> Option<Buffer> {
        self.inner
            .column_named(&name)
            .map(|column| column.serialize_bytes().into())
    }

    /// The column's canonical bytes — a self-contained `[schema][len][validity?][children]` frame,
    /// identical across Rust / Python / Node.
    #[napi]
    pub fn serialize_bytes(&self) -> Buffer {
        self.inner.serialize_bytes().into()
    }

    /// Reconstructs a struct column from [`serializeBytes`](Self::serialize_bytes).
    #[napi(factory)]
    pub fn deserialize_bytes(bytes: Buffer) -> napi::Result<Self> {
        CoreStructSerie::deserialize_bytes(&bytes)
            .map(|inner| Self { inner })
            .map_err(to_error)
    }

    /// Value equality (content, nulls included).
    #[napi]
    pub fn equals(&self, other: &StructSerie) -> bool {
        self.inner == other.inner
    }

    /// An explicit copy.
    #[napi]
    pub fn copy(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }

    #[napi(js_name = "toString")]
    pub fn text(&self) -> String {
        format!(
            "StructSerie(len={}, numColumns={}, nullCount={})",
            self.inner.len(),
            self.inner.num_columns(),
            CoreStructSerie::null_count(&self.inner)
        )
    }
}
