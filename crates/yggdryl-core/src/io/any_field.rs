//! [`AnyField`] — the **erased, recursive field descriptor**: a named, nullable column of any type,
//! *leaf* (the flat [`Field`](crate::io::fixed::Field), reused as `var` does) or *nested* (a struct,
//! and — in later phases — list / map). It is the family-agnostic recursion carrier every schema and
//! every [`AnySerie`](crate::io::AnySerie) reports, so it lives at the `io` root rather than inside
//! any one family. A value type: it compares and hashes by content and round-trips its exact logical
//! type through Arrow, so it works as a map key.

use super::fixed::Field;
use super::{DataTypeCategory, DataTypeId, FieldType, Headers, IoError};

/// A **named, nullable column descriptor of any type** — the recursive, erased field. A `Leaf` wraps
/// the flat [`Field`](crate::io::fixed::Field) (every fixed / variable leaf column); the nested
/// variants (`Struct`, `List`, `Map`) hold their child `AnyField`(s) inline, so the whole schema tree
/// is one closed, hashable value.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum AnyField {
    /// A leaf column — any fixed-width or variable-length type (the flat erased field).
    Leaf(Field),
    /// A struct column — a name, nullability, metadata, and ordered child fields.
    Struct {
        /// The column name.
        name: String,
        /// Whether the struct column admits nulls.
        nullable: bool,
        /// The struct's metadata.
        metadata: Headers,
        /// The ordered child fields.
        children: Vec<AnyField>,
    },
    /// A **list** column — a variable-size sequence of a single element type (Arrow `List`). Its one
    /// child field describes the element (item) type.
    List {
        /// The column name.
        name: String,
        /// Whether the list column admits nulls.
        nullable: bool,
        /// The list's metadata.
        metadata: Headers,
        /// The element (item) field.
        item: Box<AnyField>,
    },
    /// A **map** column — an unordered set of `key → value` entries (Arrow `Map`). Its two child
    /// fields describe the key and value types (`entries = [key, value]`).
    Map {
        /// The column name.
        name: String,
        /// Whether the map column admits nulls.
        nullable: bool,
        /// The map's metadata.
        metadata: Headers,
        /// Whether the entries are sorted by key.
        keys_sorted: bool,
        /// The `[key, value]` fields.
        entries: Box<[AnyField; 2]>,
    },
}

impl AnyField {
    /// A leaf field wrapping a flat [`Field`](crate::io::fixed::Field).
    pub fn leaf(field: Field) -> Self {
        Self::Leaf(field)
    }

    /// A struct field from a name, its ordered child fields, and its nullability (empty metadata).
    pub fn struct_(name: &str, children: Vec<AnyField>, nullable: bool) -> Self {
        Self::Struct {
            name: name.to_string(),
            nullable,
            metadata: Headers::new(),
            children,
        }
    }

    /// A list field from a name, its element (item) field, and its nullability (empty metadata).
    ///
    /// ```
    /// use yggdryl_core::io::fixed::Field;
    /// use yggdryl_core::io::{AnyField, DataTypeId, FieldType};
    ///
    /// let items = AnyField::leaf(Field::of("item", DataTypeId::I32, 4, true));
    /// let list = AnyField::list_("scores", items, true);
    /// assert_eq!(FieldType::type_id(&list), DataTypeId::List);
    /// assert!(list.is_list());
    /// assert_eq!(list.children().len(), 1);
    /// ```
    pub fn list_(name: &str, item: AnyField, nullable: bool) -> Self {
        Self::List {
            name: name.to_string(),
            nullable,
            metadata: Headers::new(),
            item: Box::new(item),
        }
    }

    /// A map field from a name, its `key` and `value` fields, its nullability, and whether the
    /// entries are sorted by key (empty metadata).
    ///
    /// ```
    /// use yggdryl_core::io::fixed::Field;
    /// use yggdryl_core::io::{AnyField, DataTypeId, FieldType};
    ///
    /// let key = AnyField::leaf(Field::of("key", DataTypeId::Utf8, 4, false));
    /// let value = AnyField::leaf(Field::of("value", DataTypeId::I64, 8, true));
    /// let map = AnyField::map_("counts", key, value, true, false);
    /// assert_eq!(FieldType::type_id(&map), DataTypeId::Map);
    /// assert!(map.is_map());
    /// assert_eq!(map.children().len(), 2);
    /// ```
    pub fn map_(
        name: &str,
        key: AnyField,
        value: AnyField,
        nullable: bool,
        keys_sorted: bool,
    ) -> Self {
        Self::Map {
            name: name.to_string(),
            nullable,
            metadata: Headers::new(),
            keys_sorted,
            // A map key is never null (Arrow's Map invariant); force the key field non-nullable so
            // the stored schema and every downstream Arrow array agree (see `with_nullable`).
            entries: Box::new([key.with_nullable(false), value]),
        }
    }

    /// The column name.
    pub fn name(&self) -> &str {
        match self {
            Self::Leaf(field) => field.name(),
            Self::Struct { name, .. } | Self::List { name, .. } | Self::Map { name, .. } => name,
        }
    }

    /// Whether the column admits nulls.
    pub fn nullable(&self) -> bool {
        match self {
            Self::Leaf(field) => field.nullable(),
            Self::Struct { nullable, .. }
            | Self::List { nullable, .. }
            | Self::Map { nullable, .. } => *nullable,
        }
    }

    /// The element type's [`DataTypeId`].
    pub fn type_id(&self) -> DataTypeId {
        match self {
            Self::Leaf(field) => FieldType::type_id(field),
            Self::Struct { .. } => DataTypeId::Struct,
            Self::List { .. } => DataTypeId::List,
            Self::Map { .. } => DataTypeId::Map,
        }
    }

    /// The field's metadata [`Headers`].
    pub fn metadata(&self) -> &Headers {
        match self {
            Self::Leaf(field) => field.metadata(),
            Self::Struct { metadata, .. }
            | Self::List { metadata, .. }
            | Self::Map { metadata, .. } => metadata,
        }
    }

    /// The ordered child fields — a struct's children, a list's single item field, a map's `[key,
    /// value]` fields, or an empty slice for a leaf.
    pub fn children(&self) -> &[AnyField] {
        match self {
            Self::Leaf(_) => &[],
            Self::Struct { children, .. } => children,
            Self::List { item, .. } => std::slice::from_ref(&**item),
            Self::Map { entries, .. } => &entries[..],
        }
    }

    /// Whether this field describes a struct column.
    pub fn is_struct(&self) -> bool {
        matches!(self, Self::Struct { .. })
    }

    /// Whether this field describes a list column.
    pub fn is_list(&self) -> bool {
        matches!(self, Self::List { .. })
    }

    /// Whether this field describes a map column.
    pub fn is_map(&self) -> bool {
        matches!(self, Self::Map { .. })
    }

    /// Whether this field describes a nested (composite) column.
    pub fn is_nested(&self) -> bool {
        self.type_id().is_nested()
    }

    /// The flat leaf [`Field`](crate::io::fixed::Field), or `None` for a nested field.
    pub fn as_leaf(&self) -> Option<&Field> {
        match self {
            Self::Leaf(field) => Some(field),
            Self::Struct { .. } | Self::List { .. } | Self::Map { .. } => None,
        }
    }

    /// A fresh field renamed to `name` — the one-line builder used to name a child within a schema.
    pub fn with_name(&self, name: &str) -> Self {
        match self {
            Self::Leaf(field) => Self::Leaf(
                Field::of(
                    name,
                    FieldType::type_id(field),
                    field.byte_width(),
                    field.nullable(),
                )
                .with_metadata(field.metadata().clone()),
            ),
            Self::Struct {
                nullable,
                metadata,
                children,
                ..
            } => Self::Struct {
                name: name.to_string(),
                nullable: *nullable,
                metadata: metadata.clone(),
                children: children.clone(),
            },
            Self::List {
                nullable,
                metadata,
                item,
                ..
            } => Self::List {
                name: name.to_string(),
                nullable: *nullable,
                metadata: metadata.clone(),
                item: item.clone(),
            },
            Self::Map {
                nullable,
                metadata,
                keys_sorted,
                entries,
                ..
            } => Self::Map {
                name: name.to_string(),
                nullable: *nullable,
                metadata: metadata.clone(),
                keys_sorted: *keys_sorted,
                entries: entries.clone(),
            },
        }
    }

    /// A fresh field with its top-level nullability set to `nullable` — the one-line builder that
    /// mirrors [`with_name`](AnyField::with_name). A leaf's flat
    /// [`Field`](crate::io::fixed::Field) is rebuilt from its type/width/metadata; a nested field
    /// keeps its children. Crate-internal: it is the single place the "a map key is never null"
    /// invariant is enforced (see [`map_`](AnyField::map_) and the map serie).
    pub(crate) fn with_nullable(&self, nullable: bool) -> Self {
        match self {
            Self::Leaf(field) => Self::Leaf(
                Field::of(
                    field.name(),
                    FieldType::type_id(field),
                    field.byte_width(),
                    nullable,
                )
                .with_metadata(field.metadata().clone()),
            ),
            Self::Struct {
                name,
                metadata,
                children,
                ..
            } => Self::Struct {
                name: name.clone(),
                nullable,
                metadata: metadata.clone(),
                children: children.clone(),
            },
            Self::List {
                name,
                metadata,
                item,
                ..
            } => Self::List {
                name: name.clone(),
                nullable,
                metadata: metadata.clone(),
                item: item.clone(),
            },
            Self::Map {
                name,
                metadata,
                keys_sorted,
                entries,
                ..
            } => Self::Map {
                name: name.clone(),
                nullable,
                metadata: metadata.clone(),
                keys_sorted: *keys_sorted,
                entries: entries.clone(),
            },
        }
    }

    /// A fresh field with the whole `metadata` replaced — the one setter every `with_*` metadata
    /// builder funnels through, so a leaf's flat [`Field`](crate::io::fixed::Field) is rebuilt from
    /// its type/width/nullability with the new metadata attached.
    fn with_metadata(&self, metadata: Headers) -> Self {
        match self {
            Self::Leaf(field) => Self::Leaf(
                Field::of(
                    field.name(),
                    FieldType::type_id(field),
                    field.byte_width(),
                    field.nullable(),
                )
                .with_metadata(metadata),
            ),
            Self::Struct {
                name,
                nullable,
                children,
                ..
            } => Self::Struct {
                name: name.clone(),
                nullable: *nullable,
                metadata,
                children: children.clone(),
            },
            Self::List {
                name,
                nullable,
                item,
                ..
            } => Self::List {
                name: name.clone(),
                nullable: *nullable,
                metadata,
                item: item.clone(),
            },
            Self::Map {
                name,
                nullable,
                keys_sorted,
                entries,
                ..
            } => Self::Map {
                name: name.clone(),
                nullable: *nullable,
                metadata,
                keys_sorted: *keys_sorted,
                entries: entries.clone(),
            },
        }
    }

    /// A fresh field with `extra`'s user entries overlaid on its metadata — each `extra` entry whose
    /// key is **not** a reserved discriminator (see
    /// [`is_reserved_metadata_key`](DataTypeId::is_reserved_metadata_key)) is inserted (a user entry
    /// **wins** over an existing same-named one), but the intrinsic type-recovery keys the field
    /// already carries are never clobbered. Empty `extra` (or one carrying only reserved keys) leaves
    /// the field byte-identical, so it is a no-op on the carrier path
    /// ([`NamedSerie`](crate::io::NamedSerie) with no metadata).
    ///
    /// ```
    /// use yggdryl_core::io::fixed::Field;
    /// use yggdryl_core::io::{AnyField, DataTypeId, Headers};
    ///
    /// let field = AnyField::leaf(Field::of("x", DataTypeId::I32, 4, false));
    /// let extra = Headers::new()
    ///     .with("origin", "test")
    ///     .with(DataTypeId::METADATA_KEY, "spoofed"); // reserved -> ignored
    /// let overlaid = field.with_metadata_overlay(&extra);
    /// assert_eq!(overlaid.metadata().get("origin"), Some("test"));
    /// assert_eq!(overlaid.metadata().get(DataTypeId::METADATA_KEY), None);
    /// ```
    pub fn with_metadata_overlay(&self, extra: &Headers) -> Self {
        // Empty overlay -> a byte-identical clone (the no-logic carrier path).
        if extra.is_empty() {
            return self.clone();
        }
        let mut metadata = self.metadata().clone();
        for (name, value) in extra.iter() {
            // Skip the reserved discriminator keys — they belong to the intrinsic type mapping and a
            // user overlay must never clobber them.
            if core::str::from_utf8(name).is_ok_and(DataTypeId::is_reserved_metadata_key) {
                continue;
            }
            metadata.insert_bytes(name, value); // user entry wins (replace)
        }
        self.with_metadata(metadata)
    }

    /// This field as an [`arrow_schema::Field`] (feature `arrow`) — **total** and **recursive**: a
    /// leaf maps via the flat field's exact-logical-type round-trip; a struct maps to
    /// `Field(Struct(child fields))`.
    #[cfg(feature = "arrow")]
    pub fn to_arrow(&self) -> arrow_schema::Field {
        use std::sync::Arc;
        match self {
            Self::Leaf(field) => field.to_arrow(),
            Self::Struct {
                name,
                nullable,
                metadata,
                children,
            } => {
                let fields: Vec<arrow_schema::Field> =
                    children.iter().map(AnyField::to_arrow).collect();
                arrow_schema::Field::new(
                    name,
                    arrow_schema::DataType::Struct(arrow_schema::Fields::from(fields)),
                    *nullable,
                )
                .with_metadata(metadata.to_arrow_metadata())
            }
            Self::List {
                name,
                nullable,
                metadata,
                item,
            } => arrow_schema::Field::new(
                name,
                arrow_schema::DataType::List(Arc::new(item.to_arrow())),
                *nullable,
            )
            .with_metadata(metadata.to_arrow_metadata()),
            Self::Map {
                name,
                nullable,
                metadata,
                keys_sorted,
                entries,
            } => {
                // The Map's entries are a non-nullable struct of `[key, value]`. Arrow requires the
                // key field non-nullable (a map key is never null), so force it — a `// DESIGN:`
                // divergence a caller's key-field nullability cannot override.
                let key = entries[0].to_arrow().with_nullable(false);
                let value = entries[1].to_arrow();
                let entries_field = arrow_schema::Field::new(
                    "entries",
                    arrow_schema::DataType::Struct(arrow_schema::Fields::from(vec![key, value])),
                    false,
                );
                arrow_schema::Field::new(
                    name,
                    arrow_schema::DataType::Map(Arc::new(entries_field), *keys_sorted),
                    *nullable,
                )
                .with_metadata(metadata.to_arrow_metadata())
            }
        }
    }

    /// Builds a field from an [`arrow_schema::Field`] (feature `arrow`), or `None` for a type this
    /// crate does not model. Recurses into a `Struct` / `List` / `Map` data type; every other type is
    /// a leaf.
    #[cfg(feature = "arrow")]
    pub fn from_arrow(field: &arrow_schema::Field) -> Option<Self> {
        match field.data_type() {
            arrow_schema::DataType::Struct(child_fields) => {
                let children = child_fields
                    .iter()
                    .map(|child| AnyField::from_arrow(child))
                    .collect::<Option<Vec<_>>>()?;
                Some(Self::Struct {
                    name: field.name().clone(),
                    nullable: field.is_nullable(),
                    metadata: Headers::from_arrow_metadata(field.metadata()),
                    children,
                })
            }
            arrow_schema::DataType::List(item_field) => Some(Self::List {
                name: field.name().clone(),
                nullable: field.is_nullable(),
                metadata: Headers::from_arrow_metadata(field.metadata()),
                item: Box::new(AnyField::from_arrow(item_field)?),
            }),
            arrow_schema::DataType::Map(entries_field, keys_sorted) => {
                // The Map's entries field is a struct of exactly `[key, value]`.
                let arrow_schema::DataType::Struct(entry_fields) = entries_field.data_type() else {
                    return None;
                };
                if entry_fields.len() != 2 {
                    return None;
                }
                Some(Self::Map {
                    name: field.name().clone(),
                    nullable: field.is_nullable(),
                    metadata: Headers::from_arrow_metadata(field.metadata()),
                    keys_sorted: *keys_sorted,
                    // A map key is never null (Arrow's Map invariant) — force it so a foreign schema
                    // declaring a nullable key does not violate the invariant on import.
                    entries: Box::new([
                        AnyField::from_arrow(&entry_fields[0])?.with_nullable(false),
                        AnyField::from_arrow(&entry_fields[1])?,
                    ]),
                })
            }
            _ => Field::from_arrow(field).map(Self::Leaf),
        }
    }
}

impl FieldType for AnyField {
    fn name(&self) -> &str {
        self.name()
    }

    fn type_name(&self) -> &'static str {
        match self {
            Self::Leaf(field) => field.type_name(),
            Self::Struct { .. } => "struct",
            Self::List { .. } => "list",
            Self::Map { .. } => "map",
        }
    }

    fn byte_width(&self) -> usize {
        match self {
            Self::Leaf(field) => field.byte_width(),
            Self::Struct { .. } | Self::List { .. } | Self::Map { .. } => 0,
        }
    }

    fn nullable(&self) -> bool {
        self.nullable()
    }

    fn type_id(&self) -> DataTypeId {
        self.type_id()
    }
}

impl From<Field> for AnyField {
    fn from(field: Field) -> Self {
        Self::Leaf(field)
    }
}

impl AnyField {
    /// The element type's coarse [`DataTypeCategory`].
    pub fn category(&self) -> DataTypeCategory {
        self.type_id().category()
    }

    /// This field's canonical bytes — an Arrow-independent, recursive field-tree codec. A value
    /// type serializes itself, so a schema round-trips without Arrow.
    pub fn serialize_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        self.encode(&mut out);
        out
    }

    /// Appends this field's frame to `out`.
    pub(crate) fn encode(&self, out: &mut Vec<u8>) {
        match self {
            Self::Leaf(field) => {
                out.push(0); // leaf tag
                encode_str(field.name(), out);
                out.extend_from_slice(&FieldType::type_id(field).as_u16().to_le_bytes());
                out.extend_from_slice(&(field.byte_width() as u64).to_le_bytes());
                out.push(u8::from(field.nullable()));
                encode_bytes(&field.metadata().serialize_bytes(), out);
            }
            Self::Struct {
                name,
                nullable,
                metadata,
                children,
            } => Self::encode_struct(name, *nullable, metadata, children, out),
            Self::List {
                name,
                nullable,
                metadata,
                item,
            } => Self::encode_list(name, *nullable, metadata, item, out),
            Self::Map {
                name,
                nullable,
                metadata,
                keys_sorted,
                entries,
            } => Self::encode_map(
                name,
                *nullable,
                metadata,
                *keys_sorted,
                &entries[0],
                &entries[1],
                out,
            ),
        }
    }

    /// Appends a **struct** field frame from its borrowed parts — the single wire format for a
    /// struct field, so a [`StructSerie`](crate::io::nested::StructSerie) can serialize its schema
    /// straight from its `(name, nullable, metadata, fields)` **without cloning** them into an
    /// `AnyField::Struct` first.
    pub(crate) fn encode_struct(
        name: &str,
        nullable: bool,
        metadata: &Headers,
        children: &[AnyField],
        out: &mut Vec<u8>,
    ) {
        out.push(1); // struct tag
        encode_str(name, out);
        out.push(u8::from(nullable));
        encode_bytes(&metadata.serialize_bytes(), out);
        out.extend_from_slice(&(children.len() as u64).to_le_bytes());
        for child in children {
            child.encode(out);
        }
    }

    /// Appends a **list** field frame from its borrowed parts (mirrors
    /// [`encode_struct`](AnyField::encode_struct)) — tag `2`, then the name, nullability, metadata,
    /// and the single recursively-encoded item field.
    pub(crate) fn encode_list(
        name: &str,
        nullable: bool,
        metadata: &Headers,
        item: &AnyField,
        out: &mut Vec<u8>,
    ) {
        out.push(2); // list tag
        encode_str(name, out);
        out.push(u8::from(nullable));
        encode_bytes(&metadata.serialize_bytes(), out);
        item.encode(out);
    }

    /// Appends a **map** field frame from its borrowed parts (mirrors
    /// [`encode_struct`](AnyField::encode_struct)) — tag `3`, then the name, nullability, metadata,
    /// the `keys_sorted` flag, and the recursively-encoded `key` then `value` fields.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn encode_map(
        name: &str,
        nullable: bool,
        metadata: &Headers,
        keys_sorted: bool,
        key: &AnyField,
        value: &AnyField,
        out: &mut Vec<u8>,
    ) {
        out.push(3); // map tag
        encode_str(name, out);
        out.push(u8::from(nullable));
        encode_bytes(&metadata.serialize_bytes(), out);
        out.push(u8::from(keys_sorted));
        key.encode(out);
        value.encode(out);
    }

    /// Reconstructs a field from [`serialize_bytes`](AnyField::serialize_bytes) bytes.
    pub fn deserialize_bytes(bytes: &[u8]) -> Result<Self, IoError> {
        let mut cursor = 0usize;
        Self::decode(bytes, &mut cursor)
    }

    /// Decodes a field from `bytes` at `*cursor`, advancing it.
    pub(crate) fn decode(bytes: &[u8], cursor: &mut usize) -> Result<Self, IoError> {
        match read_byte(bytes, cursor)? {
            0 => {
                let name = decode_str(bytes, cursor)?;
                let type_id_raw = read_u16(bytes, cursor)?;
                let type_id =
                    DataTypeId::from_u16(type_id_raw).ok_or_else(|| IoError::Unsupported {
                        what: format!(
                            "unknown data-type id 0x{type_id_raw:04x} in serialized field"
                        ),
                    })?;
                let byte_width = read_u64(bytes, cursor)? as usize;
                let nullable = read_byte(bytes, cursor)? != 0;
                let metadata = Headers::deserialize_bytes(decode_bytes(bytes, cursor)?)
                    .map_err(|_| corrupt("headers"))?;
                Ok(Self::Leaf(
                    Field::of(&name, type_id, byte_width, nullable).with_metadata(metadata),
                ))
            }
            1 => {
                let name = decode_str(bytes, cursor)?;
                let nullable = read_byte(bytes, cursor)? != 0;
                let metadata = Headers::deserialize_bytes(decode_bytes(bytes, cursor)?)
                    .map_err(|_| corrupt("headers"))?;
                let count = read_u64(bytes, cursor)? as usize;
                // Each child is at least one byte, so a count beyond the input is corrupt; cap the
                // pre-allocation (the loop then errors on the short read).
                let mut children = Vec::with_capacity(count.min(bytes.len()));
                for _ in 0..count {
                    children.push(Self::decode(bytes, cursor)?);
                }
                Ok(Self::Struct {
                    name,
                    nullable,
                    metadata,
                    children,
                })
            }
            2 => {
                let name = decode_str(bytes, cursor)?;
                let nullable = read_byte(bytes, cursor)? != 0;
                let metadata = Headers::deserialize_bytes(decode_bytes(bytes, cursor)?)
                    .map_err(|_| corrupt("headers"))?;
                let item = Box::new(Self::decode(bytes, cursor)?);
                Ok(Self::List {
                    name,
                    nullable,
                    metadata,
                    item,
                })
            }
            3 => {
                let name = decode_str(bytes, cursor)?;
                let nullable = read_byte(bytes, cursor)? != 0;
                let metadata = Headers::deserialize_bytes(decode_bytes(bytes, cursor)?)
                    .map_err(|_| corrupt("headers"))?;
                let keys_sorted = read_byte(bytes, cursor)? != 0;
                let key = Self::decode(bytes, cursor)?;
                let value = Self::decode(bytes, cursor)?;
                Ok(Self::Map {
                    name,
                    nullable,
                    metadata,
                    keys_sorted,
                    entries: Box::new([key, value]),
                })
            }
            other => Err(IoError::Unsupported {
                what: format!("unknown field tag {other} in serialized schema"),
            }),
        }
    }
}

// ---- small length-prefixed codec primitives for the field tree --------------------------

fn corrupt(what: &str) -> IoError {
    IoError::Unsupported {
        what: format!("corrupt {what} in serialized field"),
    }
}

fn encode_str(value: &str, out: &mut Vec<u8>) {
    encode_bytes(value.as_bytes(), out);
}

fn encode_bytes(value: &[u8], out: &mut Vec<u8>) {
    out.extend_from_slice(&(value.len() as u64).to_le_bytes());
    out.extend_from_slice(value);
}

fn take<'a>(bytes: &'a [u8], cursor: &mut usize, len: usize) -> Result<&'a [u8], IoError> {
    match cursor.checked_add(len).filter(|end| *end <= bytes.len()) {
        Some(end) => {
            let slice = &bytes[*cursor..end];
            *cursor = end;
            Ok(slice)
        }
        None => Err(IoError::UnexpectedEof {
            offset: *cursor as u64,
            requested: len,
            available: bytes.len().saturating_sub(*cursor),
        }),
    }
}

fn read_byte(bytes: &[u8], cursor: &mut usize) -> Result<u8, IoError> {
    Ok(take(bytes, cursor, 1)?[0])
}

fn read_u16(bytes: &[u8], cursor: &mut usize) -> Result<u16, IoError> {
    let slice = take(bytes, cursor, 2)?;
    Ok(u16::from_le_bytes([slice[0], slice[1]]))
}

fn read_u64(bytes: &[u8], cursor: &mut usize) -> Result<u64, IoError> {
    let slice = take(bytes, cursor, 8)?;
    Ok(u64::from_le_bytes(slice.try_into().unwrap()))
}

fn decode_bytes<'a>(bytes: &'a [u8], cursor: &mut usize) -> Result<&'a [u8], IoError> {
    let len = read_u64(bytes, cursor)? as usize;
    take(bytes, cursor, len)
}

fn decode_str(bytes: &[u8], cursor: &mut usize) -> Result<String, IoError> {
    let slice = decode_bytes(bytes, cursor)?;
    String::from_utf8(slice.to_vec()).map_err(|_| corrupt("field name"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaf(name: &str, id: DataTypeId, width: usize, nullable: bool) -> AnyField {
        AnyField::leaf(Field::of(name, id, width, nullable))
    }

    #[test]
    fn list_and_map_variants_report_their_shape() {
        let list = AnyField::list_("xs", leaf("item", DataTypeId::I32, 4, true), true);
        assert_eq!(list.name(), "xs");
        assert_eq!(list.type_id(), DataTypeId::List);
        assert_eq!(FieldType::type_name(&list), "list");
        assert_eq!(FieldType::byte_width(&list), 0);
        assert!(list.is_list() && list.is_nested() && !list.is_struct() && !list.is_map());
        assert!(list.as_leaf().is_none());
        assert_eq!(list.children().len(), 1);
        assert_eq!(list.children()[0].name(), "item");

        let map = AnyField::map_(
            "m",
            leaf("key", DataTypeId::Utf8, 4, false),
            leaf("value", DataTypeId::I64, 8, true),
            false,
            true,
        );
        assert_eq!(map.type_id(), DataTypeId::Map);
        assert_eq!(FieldType::type_name(&map), "map");
        assert!(map.is_map() && map.is_nested());
        assert_eq!(map.children().len(), 2);
        assert_eq!(map.children()[1].name(), "value");
    }

    #[test]
    fn list_and_map_codec_round_trips_recursively() {
        // A list of maps of (utf8 -> struct{a: i32}) — several levels of recursion.
        let inner_struct = AnyField::struct_("v", vec![leaf("a", DataTypeId::I32, 4, false)], true);
        let map = AnyField::map_(
            "counts",
            leaf("k", DataTypeId::Utf8, 4, false),
            inner_struct,
            true,
            true,
        );
        let list = AnyField::list_("rows", map, false);
        let back = AnyField::deserialize_bytes(&list.serialize_bytes()).unwrap();
        assert_eq!(back, list);

        // A bare list and a bare map also round-trip.
        let bare_list = AnyField::list_("xs", leaf("item", DataTypeId::F64, 8, true), true);
        assert_eq!(
            AnyField::deserialize_bytes(&bare_list.serialize_bytes()).unwrap(),
            bare_list
        );
    }

    #[test]
    fn with_name_and_rename_preserve_nested_shape() {
        let list = AnyField::list_("xs", leaf("item", DataTypeId::I32, 4, true), true);
        let renamed = list.with_name("ys");
        assert_eq!(renamed.name(), "ys");
        assert_eq!(renamed.children(), list.children());
        assert_eq!(renamed.type_id(), DataTypeId::List);
    }

    #[test]
    fn with_metadata_overlay_adds_user_entries_but_never_reserved() {
        let field = leaf("x", DataTypeId::I32, 4, false);
        // Empty overlay is a byte-identical no-op.
        assert_eq!(field.with_metadata_overlay(&Headers::new()), field);

        let extra = Headers::new()
            .with("origin", "test")
            .with(DataTypeId::METADATA_KEY, "spoofed")
            .with(DataTypeId::PRECISION_METADATA_KEY, "99");
        let overlaid = field.with_metadata_overlay(&extra);
        assert_eq!(overlaid.metadata().get("origin"), Some("test"));
        // Reserved discriminator keys are ignored, never clobbering the intrinsic mapping.
        assert_eq!(overlaid.metadata().get(DataTypeId::METADATA_KEY), None);
        assert_eq!(
            overlaid.metadata().get(DataTypeId::PRECISION_METADATA_KEY),
            None
        );
        // Everything but metadata is unchanged.
        assert_eq!(overlaid.name(), "x");
        assert_eq!(overlaid.type_id(), DataTypeId::I32);
    }

    #[cfg(feature = "arrow")]
    #[test]
    fn list_and_map_round_trip_through_arrow() {
        let list = AnyField::list_("xs", leaf("item", DataTypeId::I32, 4, true), true);
        assert_eq!(AnyField::from_arrow(&list.to_arrow()), Some(list.clone()));

        let map = AnyField::map_(
            "m",
            leaf("key", DataTypeId::Utf8, 4, false),
            leaf("value", DataTypeId::I64, 8, true),
            true,
            true,
        );
        let arrow = map.to_arrow();
        assert!(matches!(
            arrow.data_type(),
            arrow_schema::DataType::Map(_, true)
        ));
        assert_eq!(AnyField::from_arrow(&arrow), Some(map));
    }
}
