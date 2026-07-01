//! Apache Arrow interoperability — every schema node round-trips through Arrow.
//!
//! The bridge is the [`ArrowSchema`] node, a dependency-free mirror of Apache Arrow's
//! [C Data Interface] schema: a `format` string, a `name`, a `nullable` flag,
//! byte-keyed `metadata`, and ordered `children`. Because an Arrow schema is just a
//! [`StructField`], nesting is fully recursive — a struct's children are their own
//! nodes.
//!
//! Every [`DataType`](crate::DataType) has an Arrow format string
//! ([`DataTypeId::arrow_format`]); the dynamic [`AnyType`] / [`AnyField`] /
//! [`StructType`] / [`StructField`] carry the full [`to_arrow`](StructField::to_arrow)
//! / [`from_arrow`](StructField::from_arrow) round-trip (names, nullability, nesting
//! and metadata). The 128- and 256-bit integers have no native Arrow type, so they
//! encode as a `FixedSizeBinary` (`w:16` / `w:32`) tagged with an Arrow **extension
//! type** (`ARROW:extension:name = yggdryl.int128`, …) — standard Arrow that any
//! reader carries verbatim and from which we recover the exact type losslessly.
//!
//! [C Data Interface]: https://arrow.apache.org/docs/format/CDataInterface.html
//!
//! ```
//! use yggdryl_schema::{AnyField, AnyType, DataTypeId, StructField};
//!
//! let schema = StructField::new(
//!     "record",
//!     vec![
//!         AnyField::new("id", AnyType::primitive(DataTypeId::Int64)),
//!         AnyField::new("big", AnyType::primitive(DataTypeId::Int128)),
//!     ],
//! );
//! let arrow = schema.to_arrow();
//! assert_eq!(arrow.format(), "+s");
//! assert_eq!(StructField::from_arrow(&arrow).unwrap(), schema); // lossless
//! ```

use std::fmt;

use crate::dtype::{AnyType, DataTypeId, StructType};
use crate::field::{AnyField, Field, Metadata, StructField};

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
/// one of these with `format` `"+s"`. Build our schema nodes with
/// [`to_arrow`](StructField::to_arrow) and read foreign ones back with
/// [`from_arrow`](StructField::from_arrow).
///
/// [C Data Interface]: https://arrow.apache.org/docs/format/CDataInterface.html
///
/// ```
/// use yggdryl_schema::{AnyType, DataTypeId};
///
/// let node = AnyType::primitive(DataTypeId::Int32).to_arrow();
/// assert_eq!(node.format(), "i");
/// assert!(node.children().is_empty());
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
    pub(crate) fn primitive(id: DataTypeId) -> Self {
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
    fn field(
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
}

impl AnyType {
    /// The type encoded as an Arrow node (with no name).
    pub fn to_arrow(&self) -> ArrowSchema {
        match self {
            AnyType::Primitive(id) => ArrowSchema::primitive(*id),
            AnyType::Struct(ty) => ty.to_arrow(),
        }
    }

    /// The type built from an Arrow node, or an [`ArrowError`] if its format is one
    /// this schema layer does not model.
    pub fn from_arrow(schema: &ArrowSchema) -> Result<Self, ArrowError> {
        if schema.format == DataTypeId::Struct.arrow_format() {
            StructType::from_arrow(schema).map(AnyType::Struct)
        } else {
            primitive_id(schema).map(AnyType::Primitive)
        }
    }
}

impl StructType {
    /// The struct type encoded as an Arrow `"+s"` node whose children are its fields.
    pub fn to_arrow(&self) -> ArrowSchema {
        ArrowSchema {
            format: DataTypeId::Struct.arrow_format().to_owned(),
            name: String::new(),
            nullable: false,
            metadata: Metadata::new(),
            children: self.fields().iter().map(AnyField::to_arrow).collect(),
        }
    }

    /// The struct type built from an Arrow `"+s"` node, recursing into its children.
    pub fn from_arrow(schema: &ArrowSchema) -> Result<Self, ArrowError> {
        expect_struct(&schema.format)?;
        let fields = schema
            .children
            .iter()
            .map(AnyField::from_arrow)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(StructType::new(fields))
    }
}

impl AnyField {
    /// The field encoded as an Arrow node (its type, stamped with name/nullability
    /// and metadata).
    pub fn to_arrow(&self) -> ArrowSchema {
        ArrowSchema::field(
            self.any_type().to_arrow(),
            self.name(),
            self.nullable(),
            self.metadata(),
        )
    }

    /// The field built from an Arrow node, or an [`ArrowError`] on an unmodelled type.
    pub fn from_arrow(schema: &ArrowSchema) -> Result<Self, ArrowError> {
        let dtype = AnyType::from_arrow(schema)?;
        Ok(AnyField::from_parts(
            schema.name.clone(),
            dtype,
            schema.nullable,
            field_metadata(schema),
        ))
    }
}

impl StructField {
    /// The schema encoded as an Arrow `"+s"` node — the canonical Arrow schema form.
    pub fn to_arrow(&self) -> ArrowSchema {
        ArrowSchema::field(
            self.dtype().to_arrow(),
            self.name(),
            self.nullable(),
            self.metadata(),
        )
    }

    /// The schema built from an Arrow `"+s"` node, or an [`ArrowError`] if the node is
    /// not a struct or holds an unmodelled child type.
    pub fn from_arrow(schema: &ArrowSchema) -> Result<Self, ArrowError> {
        let dtype = StructType::from_arrow(schema)?;
        Ok(StructField::from_parts(
            schema.name.clone(),
            dtype,
            schema.nullable,
            field_metadata(schema),
        ))
    }
}

/// The [`DataTypeId`] of a primitive Arrow node, disambiguating the wide integers by
/// their extension name.
fn primitive_id(schema: &ArrowSchema) -> Result<DataTypeId, ArrowError> {
    let ext = schema
        .metadata
        .get(ARROW_EXTENSION_NAME_KEY)
        .map(Vec::as_slice);
    let id = match schema.format.as_str() {
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
        "w:16" | "w:32" => return wide_id(&schema.format, ext),
        other => return Err(ArrowError::UnsupportedFormat(other.to_owned())),
    };
    Ok(id)
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

/// The field metadata carried by an Arrow node, stripped of the internal
/// extension-name key so a field round-trips to exactly what it was built from.
fn field_metadata(schema: &ArrowSchema) -> Option<Metadata> {
    let meta: Metadata = schema
        .metadata
        .iter()
        .filter(|(k, _)| k.as_slice() != ARROW_EXTENSION_NAME_KEY)
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    (!meta.is_empty()).then_some(meta)
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

/// Checks that an Arrow format string denotes a struct.
fn expect_struct(format: &str) -> Result<(), ArrowError> {
    if format == DataTypeId::Struct.arrow_format() {
        Ok(())
    } else {
        Err(ArrowError::NotAStruct(format.to_owned()))
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
        }
    }
}

impl std::error::Error for ArrowError {}
