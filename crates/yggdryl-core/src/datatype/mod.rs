//! Arrow-centric data types.
//!
//! The type system mirrors Apache Arrow's: every type is one of three
//! [categories](TypeCategory) — *primitive*, *nested* or *logical* — exposed
//! through the base [`DataType`] trait and the marker sub-traits
//! [`PrimitiveType`], [`NestedType`] and [`LogicalType`]. Concrete type
//! descriptors live in their own module ([`BinaryType`], [`Utf8Type`], …) and the
//! [`AnyType`] enum is the hashable, serializable carrier a [`Field`](crate::Field)
//! stores. The matching in-memory *values* are the separate scalars
//! [`Binary`](crate::Binary) and [`Utf8`](crate::Utf8).
//!
//! This crate deliberately does **not** depend on `arrow-schema`; the conversion
//! to Arrow's own `DataType` belongs to `yggdryl-schema`. Here the types only
//! match Arrow's taxonomy and semantics.

mod binary;
mod string;

pub use binary::BinaryType;
pub use string::Utf8Type;

use std::borrow::Cow;
use std::collections::BTreeMap;

use crate::error::TypeError;

/// The three top-level categories an Arrow data type falls into.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum TypeCategory {
    /// Fixed-width or variable-length binary-backed values (ints, floats,
    /// [`Binary`], [`Utf8`], …).
    Primitive,
    /// Types built from child fields (`List`, `Struct`, `Map`, …).
    Nested,
    /// A physical type reinterpreted with extra meaning (`Decimal`, `Date`,
    /// `Timestamp`, dictionaries, …).
    Logical,
}

/// Behaviour shared by every Arrow data type.
///
/// Implementors provide only [`type_name`](DataType::type_name),
/// [`category`](DataType::category) and [`to_any`](DataType::to_any); the
/// serialization surface ([`to_str`](DataType::to_str),
/// [`to_mapping`](DataType::to_mapping), [`to_bytes`](DataType::to_bytes)) is
/// derived from the canonical name by default.
pub trait DataType {
    /// The canonical type name, e.g. `"binary"` or `"large_string"`.
    fn type_name(&self) -> &'static str;

    /// Which of the three [categories](TypeCategory) this type belongs to.
    fn category(&self) -> TypeCategory;

    /// This type as the hashable, serializable [`AnyType`] carrier.
    fn to_any(&self) -> AnyType;

    /// The canonical string form. Borrowed for parameterless types, so it does
    /// not allocate.
    fn to_str(&self) -> Cow<'static, str> {
        Cow::Borrowed(self.type_name())
    }

    /// The component map `{"type": <name>, …}`.
    fn to_mapping(&self) -> BTreeMap<String, String> {
        let mut map = BTreeMap::new();
        map.insert("type".to_string(), self.type_name().to_string());
        map
    }

    /// The canonical string's UTF-8 bytes.
    fn to_bytes(&self) -> Vec<u8> {
        self.to_str().into_owned().into_bytes()
    }
}

/// Marker for *primitive* types — fixed-width or variable-length binary-backed
/// values that hold no child fields.
pub trait PrimitiveType: DataType {}

/// Marker for *nested* types — those built from one or more child fields.
pub trait NestedType: DataType {}

/// Marker for *logical* types — physical types reinterpreted with extra meaning.
pub trait LogicalType: DataType {}

/// Variable-length, binary-backed primitive types ([`Binary`], [`Utf8`] and
/// their `large_*` 64-bit-offset variants).
pub trait BinaryBased: PrimitiveType {
    /// Whether the bytes are guaranteed to be valid UTF-8 (i.e. a string type).
    fn is_utf8(&self) -> bool;

    /// Whether offsets are 64-bit (`large_binary` / `large_string`).
    fn is_large(&self) -> bool;
}

/// A concrete, hashable, serializable Arrow data type.
///
/// `AnyType` is the carrier a [`Field`](crate::Field) stores and what every
/// concrete type converts into via [`DataType::to_any`]. It serializes to its
/// canonical string (`"binary"`, `"large_string"`, …).
///
/// ```
/// use yggdryl_core::{AnyType, BinaryType, DataType};
///
/// let ty = AnyType::from_str("large_binary").unwrap();
/// assert_eq!(ty.to_str(), "large_binary");
/// assert_eq!(ty, AnyType::from(BinaryType::large()));
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(into = "String", try_from = "String"))]
pub enum AnyType {
    /// A [`BinaryType`] (`binary` / `large_binary`).
    Binary(BinaryType),
    /// A [`Utf8Type`] (`string` / `large_string`).
    Utf8(Utf8Type),
}

impl AnyType {
    /// Parses a canonical type name into the matching variant.
    #[allow(clippy::should_implement_trait)] // `from_str` is the crate-wide naming convention.
    pub fn from_str(value: &str) -> Result<Self, TypeError> {
        crate::log_event!(trace, "AnyType::from_str {:?}", value);
        if let Ok(binary) = BinaryType::from_str(value) {
            return Ok(AnyType::Binary(binary));
        }
        if let Ok(utf8) = Utf8Type::from_str(value) {
            return Ok(AnyType::Utf8(utf8));
        }
        crate::log_event!(warn, "AnyType::from_str unknown type {:?}", value);
        Err(TypeError::UnknownType(value.to_string()))
    }

    /// Reconstructs a type from its component map (reads the `"type"` key).
    pub fn from_mapping(map: &BTreeMap<String, String>) -> Result<Self, TypeError> {
        let name = map
            .get("type")
            .ok_or_else(|| TypeError::InvalidMapping("missing \"type\" key".to_string()))?;
        Self::from_str(name)
    }

    /// Reconstructs a type from the bytes produced by [`DataType::to_bytes`].
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, TypeError> {
        let name = std::str::from_utf8(bytes).map_err(|_| TypeError::InvalidUtf8)?;
        Self::from_str(name)
    }

    /// The JSON form (the canonical string as a JSON string).
    #[cfg(feature = "json")]
    pub fn to_json(&self) -> String {
        crate::json::render(self)
    }

    /// Parses the JSON form produced by [`AnyType::to_json`].
    #[cfg(feature = "json")]
    pub fn from_json(value: &str) -> Result<Self, TypeError> {
        serde_json::from_str(value).map_err(|err| TypeError::InvalidMapping(err.to_string()))
    }
}

impl DataType for AnyType {
    fn type_name(&self) -> &'static str {
        match self {
            AnyType::Binary(inner) => inner.type_name(),
            AnyType::Utf8(inner) => inner.type_name(),
        }
    }

    fn category(&self) -> TypeCategory {
        match self {
            AnyType::Binary(inner) => inner.category(),
            AnyType::Utf8(inner) => inner.category(),
        }
    }

    fn to_any(&self) -> AnyType {
        self.clone()
    }
}

impl From<BinaryType> for AnyType {
    fn from(inner: BinaryType) -> Self {
        AnyType::Binary(inner)
    }
}

impl From<Utf8Type> for AnyType {
    fn from(inner: Utf8Type) -> Self {
        AnyType::Utf8(inner)
    }
}

impl From<AnyType> for String {
    fn from(value: AnyType) -> Self {
        value.to_str().into_owned()
    }
}

impl TryFrom<String> for AnyType {
    type Error = TypeError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        AnyType::from_str(&value)
    }
}
