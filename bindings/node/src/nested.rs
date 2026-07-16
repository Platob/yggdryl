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

use napi::bindgen_prelude::{Buffer, Either4};
use napi_derive::napi;

use yggdryl_core::io::fixed::Field as CoreField;
use yggdryl_core::io::nested::{
    ListField as CoreListField, ListSerie as CoreListSerie, MapField as CoreMapField,
    MapSerie as CoreMapSerie, StructField as CoreStructField, StructSerie as CoreStructSerie,
};
use yggdryl_core::io::{
    read_any_column, AnyField, AnyScalar, AnySerie, Bytes, DataTypeId, FieldType,
};

use crate::types::{DataType, Field};

/// Names a (self-describing) erased column in place — the one-line replacement for the removed
/// `NamedSerie` carrier (the name goes straight into the column's own header).
fn named_column(mut column: Box<dyn AnySerie>, name: &str) -> Box<dyn AnySerie> {
    column.set_name(name);
    column
}

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

/// Any Node field class (leaf `Field`, nested `StructField` / `ListField` / `MapField`) → an erased
/// [`AnyField`]. A nested child (a struct field, a list item, a map key/value) can itself be any of
/// these, so every nested schema constructor takes this four-way union.
fn to_any_field(field: Either4<&Field, &StructField, &ListField, &MapField>) -> AnyField {
    match field {
        Either4::A(leaf) => AnyField::leaf(leaf.inner.clone()),
        Either4::B(nested) => nested.inner.as_any_field().clone(),
        Either4::C(list) => list.inner.as_any_field().clone(),
        Either4::D(map) => map.inner.as_any_field().clone(),
    }
}

/// An erased [`AnyField`] → its concrete `Field` / `StructField` / `ListField` / `MapField`.
fn from_any_field(field: &AnyField) -> Either4<Field, StructField, ListField, MapField> {
    if field.is_struct() {
        Either4::B(StructField {
            inner: CoreStructField::from_any_field(field.clone())
                .expect("a struct AnyField rebuilds a StructField"),
        })
    } else if field.is_list() {
        Either4::C(ListField {
            inner: CoreListField::from_any_field(field.clone())
                .expect("a list AnyField rebuilds a ListField"),
        })
    } else if field.is_map() {
        Either4::D(MapField {
            inner: CoreMapField::from_any_field(field.clone())
                .expect("a map AnyField rebuilds a MapField"),
        })
    } else {
        Either4::A(Field {
            inner: field
                .as_leaf()
                .expect("a non-nested AnyField is a leaf")
                .clone(),
        })
    }
}

/// Reconstructs one erased child column from its schema `field` and its canonical
/// [`serializeBytes`] frame, via the core's central recursive dispatch — a leaf, struct, list, or
/// map child all round-trip through the same call. This is the byte hand-off napi uses in place of
/// passing a heterogeneous child column instance across the boundary.
fn read_child(field: &AnyField, bytes: &[u8]) -> napi::Result<Box<dyn AnySerie>> {
    read_any_column(field, &mut Bytes::from_slice(bytes)).map_err(to_error)
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
        fields: Vec<Either4<&Field, &StructField, &ListField, &MapField>>,
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

    /// The child field at `index` as a `Field` / `StructField` / `ListField` / `MapField`; throws
    /// out of range.
    #[napi]
    pub fn field(
        &self,
        index: u32,
    ) -> napi::Result<Either4<Field, StructField, ListField, MapField>> {
        self.inner
            .field(index as usize)
            .map(from_any_field)
            .ok_or_else(|| to_error("StructField index out of range"))
    }

    /// The child field named `name`, or `null`.
    #[napi]
    pub fn field_named(
        &self,
        name: String,
    ) -> Option<Either4<Field, StructField, ListField, MapField>> {
        self.inner.field_named(&name).map(from_any_field)
    }

    /// The 0-based index of the child field named `name`, or `null`.
    #[napi]
    pub fn index_of(&self, name: String) -> Option<u32> {
        self.inner.index_of(&name).map(|index| index as u32)
    }

    /// The child fields, in order, as `Field` / `StructField` / `ListField` / `MapField`.
    #[napi]
    pub fn fields(&self) -> Vec<Either4<Field, StructField, ListField, MapField>> {
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
    pub fn with_field(&self, field: Either4<&Field, &StructField, &ListField, &MapField>) -> Self {
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
            cols.push(read_child(field, bytes)?);
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

    /// The child field at `index` as a `Field` / `StructField` / `ListField` / `MapField`; throws
    /// out of range.
    #[napi]
    pub fn field(
        &self,
        index: u32,
    ) -> napi::Result<Either4<Field, StructField, ListField, MapField>> {
        self.inner
            .field(index as usize)
            .map(|field| from_any_field(&field))
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

/// The **centralized list schema** — a name, nullability, metadata, and a single element (item)
/// field (a `Field` or a nested `StructField` / `ListField` / `MapField`).
#[napi(namespace = "types")]
pub struct ListField {
    pub(crate) inner: CoreListField,
}

#[napi(namespace = "types")]
impl ListField {
    /// A list schema from a name, its element (item) field, and its nullability (default `true`).
    #[napi(constructor)]
    pub fn new(
        name: String,
        item: Either4<&Field, &StructField, &ListField, &MapField>,
        nullable: Option<bool>,
    ) -> Self {
        Self {
            inner: CoreListField::new(&name, to_any_field(item), nullable.unwrap_or(true)),
        }
    }

    /// The list's name.
    #[napi(getter)]
    pub fn name(&self) -> String {
        self.inner.name().to_string()
    }

    /// Whether the list column admits nulls.
    #[napi(getter)]
    pub fn nullable(&self) -> bool {
        self.inner.nullable()
    }

    /// The element type's name (`"list"`).
    #[napi(getter)]
    pub fn type_name(&self) -> &'static str {
        "list"
    }

    /// This schema's [`DataType`].
    #[napi(getter)]
    pub fn data_type(&self) -> DataType {
        DataType::of(DataTypeId::List)
    }

    /// The element (item) field as a `Field` / `StructField` / `ListField` / `MapField`.
    #[napi(getter)]
    pub fn item(&self) -> Either4<Field, StructField, ListField, MapField> {
        from_any_field(self.inner.item())
    }

    /// A fresh list schema renamed to `name`.
    #[napi]
    pub fn with_name(&self, name: String) -> Self {
        Self {
            inner: self.inner.with_name(&name),
        }
    }

    /// A fresh list schema with `nullable` set.
    #[napi]
    pub fn with_nullable(&self, nullable: bool) -> Self {
        Self {
            inner: self.inner.with_nullable(nullable),
        }
    }

    /// A fresh list schema with a new element (item) field.
    #[napi]
    pub fn with_item(&self, item: Either4<&Field, &StructField, &ListField, &MapField>) -> Self {
        Self {
            inner: self.inner.with_item(to_any_field(item)),
        }
    }

    /// A fresh list schema with one extra `key = value` metadata entry.
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
        CoreListField::from_any_field(field)
            .map(|inner| Self { inner })
            .ok_or_else(|| to_error("the bytes did not decode to a list field"))
    }

    /// Value equality (content, metadata included).
    #[napi]
    pub fn equals(&self, other: &ListField) -> bool {
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
            "ListField(name={:?}, item={}, nullable={})",
            self.inner.name(),
            self.inner.item().type_name(),
            self.inner.nullable()
        )
    }
}

/// A **nullable list column** — `i32` offsets over one flattened child column, plus an optional
/// top-level validity mask (a null list row). Row `i` is the child sub-range
/// `child[offsets[i] .. offsets[i + 1]]`.
#[napi(namespace = "types")]
pub struct ListSerie {
    pub(crate) inner: CoreListSerie,
}

#[napi(namespace = "types")]
impl ListSerie {
    /// A list column from its element (item) `field`, the flattened child column's
    /// `serializeBytes()` frame (`itemBytes`), the row `offsets` (`len + 1` entries into the child),
    /// and an optional per-row **present** mask (`present[i] === false` marks row `i` a null list).
    /// (napi cannot accept an arbitrary one-of-many `Serie` instance, so the child crosses as its
    /// canonical bytes — build them with `serie.serializeBytes()` / `serie.toField(name)`.)
    #[napi(factory)]
    pub fn from_parts(
        item_field: Either4<&Field, &StructField, &ListField, &MapField>,
        item_bytes: Buffer,
        offsets: Vec<i32>,
        present: Option<Vec<bool>>,
    ) -> napi::Result<Self> {
        let item = to_any_field(item_field);
        let column = read_child(&item, &item_bytes)?;
        let items = named_column(column, item.name());
        CoreListSerie::from_values(items, &offsets, present.as_deref())
            .map(|inner| Self { inner })
            .map_err(to_error)
    }

    /// The number of rows.
    #[napi(getter)]
    pub fn length(&self) -> u32 {
        self.inner.len() as u32
    }

    /// The number of null list rows.
    #[napi(getter)]
    pub fn null_count(&self) -> u32 {
        self.inner.null_count() as u32
    }

    /// Whether any list row is null.
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
        DataType::of(DataTypeId::List)
    }

    /// The row offsets (`len + 1` entries into the flattened child).
    #[napi(getter)]
    pub fn offsets(&self) -> Vec<i32> {
        self.inner.offsets().to_vec()
    }

    /// The flattened child column as its canonical bytes — reconstruct it with the matching
    /// `Serie.deserializeBytes(...)` (its schema is `toField(name).item`).
    #[napi]
    pub fn item_bytes(&self) -> Buffer {
        self.inner.values().serialize_bytes().into()
    }

    /// A [`ListField`] naming this list column (nullability inferred from its null rows).
    #[napi]
    pub fn to_field(&self, name: String) -> ListField {
        ListField {
            inner: self.inner.to_field(&name),
        }
    }

    /// The column's canonical bytes — a self-contained `[schema][len][validity?][offsets][child]`
    /// frame, identical across Rust / Python / Node.
    #[napi]
    pub fn serialize_bytes(&self) -> Buffer {
        self.inner.serialize_bytes().into()
    }

    /// Reconstructs a list column from [`serializeBytes`](Self::serialize_bytes).
    #[napi(factory)]
    pub fn deserialize_bytes(bytes: Buffer) -> napi::Result<Self> {
        CoreListSerie::deserialize_bytes(&bytes)
            .map(|inner| Self { inner })
            .map_err(to_error)
    }

    /// Value equality (content, nulls included).
    #[napi]
    pub fn equals(&self, other: &ListSerie) -> bool {
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
            "ListSerie(len={}, nullCount={})",
            self.inner.len(),
            self.inner.null_count()
        )
    }
}

/// The **centralized map schema** — a name, nullability, metadata, a `keysSorted` flag, and the
/// `key` / `value` fields (each a `Field` or a nested `StructField` / `ListField` / `MapField`).
#[napi(namespace = "types")]
pub struct MapField {
    pub(crate) inner: CoreMapField,
}

#[napi(namespace = "types")]
impl MapField {
    /// A map schema from a name, its `key` and `value` fields, its nullability (default `true`), and
    /// whether the entries are sorted by key (default `false`).
    #[napi(constructor)]
    pub fn new(
        name: String,
        key: Either4<&Field, &StructField, &ListField, &MapField>,
        value: Either4<&Field, &StructField, &ListField, &MapField>,
        nullable: Option<bool>,
        keys_sorted: Option<bool>,
    ) -> Self {
        Self {
            inner: CoreMapField::new(
                &name,
                to_any_field(key),
                to_any_field(value),
                nullable.unwrap_or(true),
                keys_sorted.unwrap_or(false),
            ),
        }
    }

    /// The map's name.
    #[napi(getter)]
    pub fn name(&self) -> String {
        self.inner.name().to_string()
    }

    /// Whether the map column admits nulls.
    #[napi(getter)]
    pub fn nullable(&self) -> bool {
        self.inner.nullable()
    }

    /// The element type's name (`"map"`).
    #[napi(getter)]
    pub fn type_name(&self) -> &'static str {
        "map"
    }

    /// This schema's [`DataType`].
    #[napi(getter)]
    pub fn data_type(&self) -> DataType {
        DataType::of(DataTypeId::Map)
    }

    /// The key field as a `Field` / `StructField` / `ListField` / `MapField`.
    #[napi(getter)]
    pub fn key(&self) -> Either4<Field, StructField, ListField, MapField> {
        from_any_field(self.inner.key())
    }

    /// The value field as a `Field` / `StructField` / `ListField` / `MapField`.
    #[napi(getter)]
    pub fn value(&self) -> Either4<Field, StructField, ListField, MapField> {
        from_any_field(self.inner.value())
    }

    /// Whether the entries are sorted by key.
    #[napi(getter)]
    pub fn keys_sorted(&self) -> bool {
        self.inner.keys_sorted()
    }

    /// A fresh map schema renamed to `name`.
    #[napi]
    pub fn with_name(&self, name: String) -> Self {
        Self {
            inner: self.inner.with_name(&name),
        }
    }

    /// A fresh map schema with `nullable` set.
    #[napi]
    pub fn with_nullable(&self, nullable: bool) -> Self {
        Self {
            inner: self.inner.with_nullable(nullable),
        }
    }

    /// A fresh map schema with the `keysSorted` flag set.
    #[napi]
    pub fn with_keys_sorted(&self, keys_sorted: bool) -> Self {
        Self {
            inner: self.inner.with_keys_sorted(keys_sorted),
        }
    }

    /// A fresh map schema with one extra `key = value` metadata entry.
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
        CoreMapField::from_any_field(field)
            .map(|inner| Self { inner })
            .ok_or_else(|| to_error("the bytes did not decode to a map field"))
    }

    /// Value equality (content, metadata included).
    #[napi]
    pub fn equals(&self, other: &MapField) -> bool {
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
            "MapField(name={:?}, key={}, value={}, nullable={}, keysSorted={})",
            self.inner.name(),
            self.inner.key().type_name(),
            self.inner.value().type_name(),
            self.inner.nullable(),
            self.inner.keys_sorted()
        )
    }
}

/// A **nullable map column** — the optimized alias of `List<Struct<{key, value}>>`: `i32` offsets
/// over a flattened two-column entries store (keys non-null, values nullable), an optional top-level
/// validity mask, and a `keysSorted` flag. Row `i` is the entries `key[j] -> value[j]` for `j` in
/// `[offsets[i], offsets[i + 1])`.
#[napi(namespace = "types")]
pub struct MapSerie {
    pub(crate) inner: CoreMapSerie,
}

#[napi(namespace = "types")]
impl MapSerie {
    /// A map column from its `key` / `value` fields, each flattened child column's
    /// `serializeBytes()` frame (`keyBytes` / `valueBytes`), the row `offsets` (`len + 1` entries
    /// into the entries), an optional per-row **present** mask (`present[i] === false` marks row `i`
    /// a null map), and whether the entries are sorted by key (default `false`). A map key is never
    /// null (Arrow's Map invariant): the key column must not carry nulls.
    #[napi(factory)]
    pub fn from_parts(
        key_field: Either4<&Field, &StructField, &ListField, &MapField>,
        key_bytes: Buffer,
        value_field: Either4<&Field, &StructField, &ListField, &MapField>,
        value_bytes: Buffer,
        offsets: Vec<i32>,
        present: Option<Vec<bool>>,
        keys_sorted: Option<bool>,
    ) -> napi::Result<Self> {
        let key = to_any_field(key_field);
        let value = to_any_field(value_field);
        let keys = named_column(read_child(&key, &key_bytes)?, key.name());
        let values = named_column(read_child(&value, &value_bytes)?, value.name());
        CoreMapSerie::from_entries(
            keys,
            values,
            &offsets,
            present.as_deref(),
            keys_sorted.unwrap_or(false),
        )
        .map(|inner| Self { inner })
        .map_err(to_error)
    }

    /// The number of rows.
    #[napi(getter)]
    pub fn length(&self) -> u32 {
        self.inner.len() as u32
    }

    /// The number of null map rows.
    #[napi(getter)]
    pub fn null_count(&self) -> u32 {
        self.inner.null_count() as u32
    }

    /// Whether any map row is null.
    #[napi(getter)]
    pub fn has_nulls(&self) -> bool {
        self.inner.has_nulls()
    }

    /// Whether the column has no rows.
    #[napi]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Whether the entries are sorted by key.
    #[napi(getter)]
    pub fn keys_sorted(&self) -> bool {
        self.inner.keys_sorted()
    }

    /// This column's [`DataType`].
    #[napi(getter)]
    pub fn data_type(&self) -> DataType {
        DataType::of(DataTypeId::Map)
    }

    /// The row offsets (`len + 1` entries into the flattened entries).
    #[napi(getter)]
    pub fn offsets(&self) -> Vec<i32> {
        self.inner.offsets().to_vec()
    }

    /// The flattened key column (entries column 0) as its canonical bytes — reconstruct it with the
    /// matching `Serie.deserializeBytes(...)`.
    #[napi]
    pub fn keys(&self) -> Buffer {
        self.inner.keys().serialize_bytes().into()
    }

    /// The flattened value column (entries column 1) as its canonical bytes.
    #[napi]
    pub fn values(&self) -> Buffer {
        self.inner.values().serialize_bytes().into()
    }

    /// The value mapped to a probe key in row `row`, as the value's canonical little-endian bytes,
    /// or `null` if the row is null / out of range or the key is absent. The probe `keyBytes` are a
    /// leaf key's canonical bytes (what `Serie` cells serialize to); the lookup is the core's
    /// allocation-free [`MapSerie::get_value`]. Throws for a nested (non-leaf) key type.
    #[napi]
    pub fn get_value_bytes(&self, row: u32, key_bytes: Buffer) -> napi::Result<Option<Buffer>> {
        let key_field = self.inner.key_field();
        if key_field.is_struct() || key_field.is_list() || key_field.is_map() {
            return Err(to_error(
                "getValueBytes supports only a leaf map key; a nested key is not a byte-probe key",
            ));
        }
        // Rebuild the probe as the bare-leaf scalar a leaf column's `value()` produces, so the core's
        // allocation-free `cell_eq` compares canonical bytes directly (name `""`, non-null, empty
        // metadata, the key's type id + byte width).
        let probe = AnyScalar::leaf(
            CoreField::of("", key_field.type_id(), key_field.byte_width(), false),
            key_bytes.to_vec(),
        );
        Ok(self
            .inner
            .get_value(row as usize, &probe)
            .and_then(|value| value.bytes().map(|bytes| bytes.to_vec().into())))
    }

    /// A [`MapField`] naming this map column (nullability inferred from its null rows).
    #[napi]
    pub fn to_field(&self, name: String) -> MapField {
        MapField {
            inner: self.inner.to_field(&name),
        }
    }

    /// The column's canonical bytes — a self-contained `[schema][len][validity?][offsets][entries]`
    /// frame, identical across Rust / Python / Node.
    #[napi]
    pub fn serialize_bytes(&self) -> Buffer {
        self.inner.serialize_bytes().into()
    }

    /// Reconstructs a map column from [`serializeBytes`](Self::serialize_bytes).
    #[napi(factory)]
    pub fn deserialize_bytes(bytes: Buffer) -> napi::Result<Self> {
        CoreMapSerie::deserialize_bytes(&bytes)
            .map(|inner| Self { inner })
            .map_err(to_error)
    }

    /// Value equality (content, nulls included).
    #[napi]
    pub fn equals(&self, other: &MapSerie) -> bool {
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
            "MapSerie(len={}, nullCount={}, keysSorted={})",
            self.inner.len(),
            self.inner.null_count(),
            self.inner.keys_sorted()
        )
    }
}
