//! Apache Arrow interoperability for the schema layer.
//!
//! The bridge is the [`ArrowSchema`] node, a dependency-free mirror of Apache Arrow's
//! [C Data Interface] schema: a `format` string, a `name`, a `nullable` flag,
//! byte-keyed `metadata`, and ordered `children`. Every [`DataType`](crate::DataType)
//! has an Arrow format string ([`DataTypeId::arrow_format`]); the concrete primitive
//! types and fields round-trip through [`to_arrow_scalar`](crate::Int32Type::to_arrow_scalar)
//! / [`from_arrow_scalar`](crate::Int32Type::from_arrow_scalar). The dynamic /
//! recursive schema (structs) and its round-trip live in the `yggdryl-scalar` crate,
//! built on the [`ArrowSchema`] / [`ArrowArray`] nodes and the public helpers here.
//!
//! The 128- and 256-bit integers have no native Arrow type, so they encode as a
//! `FixedSizeBinary` (`w:16` / `w:32`) tagged with an Arrow **extension type**
//! (`ARROW:extension:name = yggdryl.int128`, …) — standard Arrow that any reader
//! carries verbatim and from which we recover the exact type losslessly.
//!
//! [C Data Interface]: https://arrow.apache.org/docs/format/CDataInterface.html
//!
//! ```
//! use yggdryl_schema::{Int32Type, UInt256Type};
//!
//! let node = Int32Type::new().to_arrow_scalar();
//! assert_eq!(node.format(), "i");
//! assert_eq!(Int32Type::from_arrow_scalar(&node).unwrap(), Int32Type::new());
//! // The wide integers ride a tagged FixedSizeBinary.
//! assert_eq!(UInt256Type::new().to_arrow_scalar().format(), "w:32");
//! ```

use std::fmt;

use crate::dtype::DataTypeId;
use crate::field::Metadata;

/// The Arrow metadata key under which an extension type records its name.
const ARROW_EXTENSION_NAME_KEY: &[u8] = b"ARROW:extension:name";

impl DataTypeId {
    /// This type's Apache Arrow [C Data Interface] format string (e.g. `"i"` for
    /// `int32`, `"+s"` for a struct). The 128-/256-bit integers, which Arrow lacks,
    /// borrow `FixedSizeBinary` (`"w:16"` / `"w:32"`) and are disambiguated by their
    /// [`arrow_extension_name`](DataTypeId::arrow_extension_name).
    ///
    /// [C Data Interface]: https://arrow.apache.org/docs/format/CDataInterface.html
    ///
    /// ```
    /// use yggdryl_schema::DataTypeId;
    ///
    /// assert_eq!(DataTypeId::Int32.arrow_format(), "i");
    /// assert_eq!(DataTypeId::UInt64.arrow_format(), "L");
    /// assert_eq!(DataTypeId::Int128.arrow_format(), "w:16");
    /// ```
    pub const fn arrow_format(self) -> &'static str {
        match self {
            DataTypeId::Null => "n",
            DataTypeId::Boolean => "b",
            DataTypeId::Int8 => "c",
            DataTypeId::Int16 => "s",
            DataTypeId::Int32 => "i",
            DataTypeId::Int64 => "l",
            DataTypeId::Int128 | DataTypeId::UInt128 => "w:16",
            DataTypeId::Int256 | DataTypeId::UInt256 => "w:32",
            DataTypeId::UInt8 => "C",
            DataTypeId::UInt16 => "S",
            DataTypeId::UInt32 => "I",
            DataTypeId::UInt64 => "L",
            DataTypeId::Utf8 => "u",
            DataTypeId::List => "+l",
            DataTypeId::Struct => "+s",
        }
    }

    /// The Arrow extension-type name for the types Arrow has no native encoding of —
    /// the 128-/256-bit integers — or `None` for a natively-encodable type.
    ///
    /// ```
    /// use yggdryl_schema::DataTypeId;
    ///
    /// assert_eq!(DataTypeId::Int128.arrow_extension_name(), Some("yggdryl.int128"));
    /// assert_eq!(DataTypeId::Int32.arrow_extension_name(), None);
    /// ```
    pub const fn arrow_extension_name(self) -> Option<&'static str> {
        match self {
            DataTypeId::Int128 => Some("yggdryl.int128"),
            DataTypeId::UInt128 => Some("yggdryl.uint128"),
            DataTypeId::Int256 => Some("yggdryl.int256"),
            DataTypeId::UInt256 => Some("yggdryl.uint256"),
            _ => None,
        }
    }
}

/// A node of an Apache Arrow schema — a dependency-free mirror of the Arrow
/// [C Data Interface] `ArrowSchema`. It pairs a `format` string with a `name`, a
/// `nullable` flag, byte-keyed `metadata` and ordered `children`; an Arrow schema is
/// one of these with `format` `"+s"`. The concrete primitive types build nodes with
/// [`to_arrow_scalar`](crate::Int32Type::to_arrow_scalar); the dynamic layer in
/// `yggdryl-scalar` builds recursive ones from the [`primitive`](ArrowSchema::primitive)
/// / [`field`](ArrowSchema::field) constructors and the
/// [`primitive_id`](ArrowSchema::primitive_id) / [`field_metadata`](ArrowSchema::field_metadata)
/// readers.
///
/// [C Data Interface]: https://arrow.apache.org/docs/format/CDataInterface.html
///
/// ```
/// use yggdryl_schema::{DataTypeId, Int32Type};
///
/// let node = Int32Type::new().to_arrow_scalar();
/// assert_eq!(node.format(), "i");
/// assert!(node.children().is_empty());
/// assert_eq!(node.primitive_id().unwrap(), DataTypeId::Int32);
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct ArrowSchema {
    format: String,
    name: String,
    nullable: bool,
    metadata: Metadata,
    children: Vec<ArrowSchema>,
}

impl ArrowSchema {
    /// A node from its explicit parts.
    pub fn from_parts(
        format: String,
        name: String,
        nullable: bool,
        metadata: Metadata,
        children: Vec<ArrowSchema>,
    ) -> Self {
        Self {
            format,
            name,
            nullable,
            metadata,
            children,
        }
    }

    /// The Arrow format string (e.g. `"i"`, `"+s"`, `"w:16"`).
    pub fn format(&self) -> &str {
        &self.format
    }

    /// The node's name (empty for a bare type).
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Whether the node admits null values.
    pub fn nullable(&self) -> bool {
        self.nullable
    }

    /// The node's byte-keyed metadata.
    pub fn metadata(&self) -> &Metadata {
        &self.metadata
    }

    /// The child nodes, in order (a struct's fields, a list's element).
    pub fn children(&self) -> &[ArrowSchema] {
        &self.children
    }

    /// A nameless type node for a primitive `id`, stamped with its extension-type
    /// metadata when Arrow has no native encoding for it.
    pub fn primitive(id: DataTypeId) -> Self {
        let mut metadata = Metadata::new();
        if let Some(ext) = id.arrow_extension_name() {
            metadata.insert(ARROW_EXTENSION_NAME_KEY.to_vec(), ext.as_bytes().to_vec());
        }
        Self {
            format: id.arrow_format().to_owned(),
            name: String::new(),
            nullable: false,
            metadata,
            children: Vec::new(),
        }
    }

    /// A field node: the data type's `type_node`, stamped with the field's `name`,
    /// `nullable` flag and metadata (merged over any type-level extension metadata).
    pub fn field(
        type_node: ArrowSchema,
        name: &str,
        nullable: bool,
        metadata: Option<&Metadata>,
    ) -> Self {
        let mut merged = type_node.metadata;
        if let Some(m) = metadata {
            merged.extend(m.iter().map(|(k, v)| (k.clone(), v.clone())));
        }
        Self {
            format: type_node.format,
            name: name.to_owned(),
            nullable,
            metadata: merged,
            children: type_node.children,
        }
    }

    /// The [`DataTypeId`] of a primitive node, disambiguating the wide integers by
    /// their extension name. Errors on a format this layer does not model (including a
    /// struct `"+s"`, which the dynamic layer handles).
    pub fn primitive_id(&self) -> Result<DataTypeId, ArrowError> {
        let ext = self
            .metadata
            .get(ARROW_EXTENSION_NAME_KEY)
            .map(Vec::as_slice);
        let id = match self.format.as_str() {
            "n" => DataTypeId::Null,
            "b" => DataTypeId::Boolean,
            "c" => DataTypeId::Int8,
            "s" => DataTypeId::Int16,
            "i" => DataTypeId::Int32,
            "l" => DataTypeId::Int64,
            "C" => DataTypeId::UInt8,
            "S" => DataTypeId::UInt16,
            "I" => DataTypeId::UInt32,
            "L" => DataTypeId::UInt64,
            "u" => DataTypeId::Utf8,
            "w:16" | "w:32" => return wide_id(&self.format, ext),
            other => return Err(ArrowError::UnsupportedFormat(other.to_owned())),
        };
        Ok(id)
    }

    /// The field metadata this node carries, stripped of the internal extension-name
    /// key so a field round-trips to exactly what it was built from.
    pub fn field_metadata(&self) -> Option<Metadata> {
        let meta: Metadata = self
            .metadata
            .iter()
            .filter(|(k, _)| k.as_slice() != ARROW_EXTENSION_NAME_KEY)
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        (!meta.is_empty()).then_some(meta)
    }
}

/// The data half of an Apache Arrow array — a dependency-free mirror of the Arrow
/// [C Data Interface] `ArrowArray` header (its `length`, `null_count` and child
/// arrays; the value buffers are the array layer's concern, not the schema's). It is
/// the companion of an [`ArrowSchema`]: pairing the two rebuilds a field whose
/// nullability is what the data *actually* contains — a positive (or unknown, `-1`)
/// `null_count` means the field is nullable.
///
/// [C Data Interface]: https://arrow.apache.org/docs/format/CDataInterface.html
///
/// ```
/// use yggdryl_schema::{ArrowArray, Field, Int32Field};
///
/// // A non-nullable Int32 schema paired with an array that holds nulls…
/// let schema = Int32Field::new("id").to_arrow_scalar();
/// let array = ArrowArray::from_parts(10, 2, vec![]);
/// // …rebuilds a *nullable* field, because the data has nulls.
/// assert!(Int32Field::from_arrow_array(&schema, &array).unwrap().nullable());
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct ArrowArray {
    length: i64,
    null_count: i64,
    children: Vec<ArrowArray>,
}

impl ArrowArray {
    /// An array header from its explicit parts. A `null_count` of `-1` means the
    /// producer left it unknown (Arrow's sentinel), which we treat as nullable.
    pub fn from_parts(length: i64, null_count: i64, children: Vec<ArrowArray>) -> Self {
        Self {
            length,
            null_count,
            children,
        }
    }

    /// The number of elements in the array.
    pub fn length(&self) -> i64 {
        self.length
    }

    /// The number of null elements (`-1` if the producer left it unknown).
    pub fn null_count(&self) -> i64 {
        self.null_count
    }

    /// The child arrays, in order (a struct's field arrays).
    pub fn children(&self) -> &[ArrowArray] {
        &self.children
    }

    /// Whether the data contains (or may contain) nulls — a positive or unknown
    /// (`-1`) `null_count`. This is the nullability a field built from the array takes.
    pub fn nullable(&self) -> bool {
        self.null_count != 0
    }
}

/// The 128-/256-bit [`DataTypeId`] a `FixedSizeBinary` node stands for, recovered
/// from its extension name (the inverse of [`DataTypeId::arrow_extension_name`]).
fn wide_id(format: &str, ext: Option<&[u8]>) -> Result<DataTypeId, ArrowError> {
    let name = ext.ok_or_else(|| ArrowError::MissingExtension(format.to_owned()))?;
    [
        DataTypeId::Int128,
        DataTypeId::UInt128,
        DataTypeId::Int256,
        DataTypeId::UInt256,
    ]
    .into_iter()
    .find(|id| id.arrow_extension_name().map(str::as_bytes) == Some(name))
    .ok_or_else(|| ArrowError::UnknownExtension(String::from_utf8_lossy(name).into_owned()))
}

/// Checks that a decoded type id matches the one a scalar `to_arrow_scalar` /
/// `from_arrow_scalar` expects, so a concrete type/field rejects a mismatched node.
pub(crate) fn check_id(expected: DataTypeId, found: DataTypeId) -> Result<(), ArrowError> {
    if expected == found {
        Ok(())
    } else {
        Err(ArrowError::TypeMismatch { expected, found })
    }
}

/// An error building a schema type or field from an Arrow node.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ArrowError {
    /// The Arrow format string is not one this schema layer models.
    UnsupportedFormat(String),
    /// A wide-integer `FixedSizeBinary` lacked its `ARROW:extension:name` metadata.
    MissingExtension(String),
    /// A wide-integer `FixedSizeBinary` carried an unknown extension name.
    UnknownExtension(String),
    /// A struct decode was asked of a non-struct node.
    NotAStruct(String),
    /// A scalar decode found a node of a different type than the one expected.
    TypeMismatch {
        /// The type the concrete `from_arrow_scalar` was called on.
        expected: DataTypeId,
        /// The type the Arrow node actually resolved to.
        found: DataTypeId,
    },
    /// A struct's schema and array disagreed on the number of child nodes.
    ChildCountMismatch {
        /// The number of child fields the schema declares.
        schema: usize,
        /// The number of child arrays the data supplies.
        array: usize,
    },
}

impl fmt::Display for ArrowError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ArrowError::UnsupportedFormat(format) => write!(
                f,
                "unsupported Arrow format string {format:?}; expected a primitive \
                 (e.g. \"i\", \"u\"), a struct (\"+s\"), or a tagged wide integer \
                 (\"w:16\"/\"w:32\")"
            ),
            ArrowError::MissingExtension(format) => write!(
                f,
                "Arrow FixedSizeBinary {format:?} is missing its \
                 `ARROW:extension:name` metadata; expected one of yggdryl.int128, \
                 yggdryl.uint128, yggdryl.int256 or yggdryl.uint256"
            ),
            ArrowError::UnknownExtension(name) => write!(
                f,
                "unknown Arrow extension name {name:?}; expected one of \
                 yggdryl.int128, yggdryl.uint128, yggdryl.int256 or yggdryl.uint256"
            ),
            ArrowError::NotAStruct(format) => write!(
                f,
                "expected an Arrow struct (\"+s\"), found format {format:?}"
            ),
            ArrowError::TypeMismatch { expected, found } => write!(
                f,
                "expected an Arrow {} node, found {}",
                expected.name(),
                found.name()
            ),
            ArrowError::ChildCountMismatch { schema, array } => write!(
                f,
                "Arrow struct schema declares {schema} child field(s) but the array \
                 supplies {array}"
            ),
        }
    }
}

impl std::error::Error for ArrowError {}
