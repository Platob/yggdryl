//! [`AnyField`] — the **erased, recursive field descriptor**: a named, nullable column of any type,
//! *leaf* (the flat [`Field`](crate::io::fixed::Field), reused as `var` does) or *nested* (a struct,
//! and — in later phases — list / map). It is the family-agnostic recursion carrier every schema and
//! every [`AnySerie`](crate::io::AnySerie) reports, so it lives at the `io` root rather than inside
//! any one family. A value type: it compares and hashes by content and round-trips its exact logical
//! type through Arrow, so it works as a map key.

use super::fixed::Field;
use super::{DataTypeCategory, DataTypeId, FieldType, Headers, IoError};

/// A **named, nullable column descriptor of any type** — the recursive, erased field. A `Leaf` wraps
/// the flat [`Field`](crate::io::fixed::Field) (every fixed / variable leaf column); a `Struct` holds
/// its ordered child `AnyField`s inline, so the whole schema tree is one closed, hashable value.
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

    /// The column name.
    pub fn name(&self) -> &str {
        match self {
            Self::Leaf(field) => field.name(),
            Self::Struct { name, .. } => name,
        }
    }

    /// Whether the column admits nulls.
    pub fn nullable(&self) -> bool {
        match self {
            Self::Leaf(field) => field.nullable(),
            Self::Struct { nullable, .. } => *nullable,
        }
    }

    /// The element type's [`DataTypeId`].
    pub fn type_id(&self) -> DataTypeId {
        match self {
            Self::Leaf(field) => FieldType::type_id(field),
            Self::Struct { .. } => DataTypeId::Struct,
        }
    }

    /// The field's metadata [`Headers`].
    pub fn metadata(&self) -> &Headers {
        match self {
            Self::Leaf(field) => field.metadata(),
            Self::Struct { metadata, .. } => metadata,
        }
    }

    /// The ordered child fields — a struct's children, or an empty slice for a leaf.
    pub fn children(&self) -> &[AnyField] {
        match self {
            Self::Leaf(_) => &[],
            Self::Struct { children, .. } => children,
        }
    }

    /// Whether this field describes a struct column.
    pub fn is_struct(&self) -> bool {
        matches!(self, Self::Struct { .. })
    }

    /// Whether this field describes a nested (composite) column.
    pub fn is_nested(&self) -> bool {
        self.type_id().is_nested()
    }

    /// The flat leaf [`Field`](crate::io::fixed::Field), or `None` for a nested field.
    pub fn as_leaf(&self) -> Option<&Field> {
        match self {
            Self::Leaf(field) => Some(field),
            Self::Struct { .. } => None,
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
        }
    }

    /// This field as an [`arrow_schema::Field`] (feature `arrow`) — **total** and **recursive**: a
    /// leaf maps via the flat field's exact-logical-type round-trip; a struct maps to
    /// `Field(Struct(child fields))`.
    #[cfg(feature = "arrow")]
    pub fn to_arrow(&self) -> arrow_schema::Field {
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
        }
    }

    /// Builds a field from an [`arrow_schema::Field`] (feature `arrow`), or `None` for a type this
    /// crate does not model. Recurses into a `Struct` data type; every other type is a leaf.
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
        }
    }

    fn byte_width(&self) -> usize {
        match self {
            Self::Leaf(field) => field.byte_width(),
            Self::Struct { .. } => 0,
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
