//! The [`BinaryType`] data type — variable-length, opaque byte strings.

use std::collections::BTreeMap;

use crate::datatype::{AnyType, BinaryBased, DataType, PrimitiveType, TypeCategory};
use crate::error::TypeError;

/// Arrow's variable-length binary type, in both its 32-bit (`binary`) and 64-bit
/// (`large_binary`) offset flavours.
///
/// This is the *type* descriptor; the in-memory binary *value* is
/// [`Binary`](crate::Binary).
///
/// ```
/// use yggdryl_core::{BinaryBased, BinaryType, DataType};
///
/// let b = BinaryType::new();
/// assert_eq!(b.type_name(), "binary");
/// assert!(!b.is_large());
/// assert_eq!(BinaryType::large().type_name(), "large_binary");
/// assert_eq!(BinaryType::from_str("large_binary").unwrap(), BinaryType::large());
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(into = "String", try_from = "String"))]
pub struct BinaryType {
    large: bool,
}

impl BinaryType {
    /// The 32-bit-offset `binary` type.
    pub fn new() -> Self {
        Self { large: false }
    }

    /// The 64-bit-offset `large_binary` type.
    pub fn large() -> Self {
        Self { large: true }
    }

    /// Parses `"binary"` or `"large_binary"`.
    #[allow(clippy::should_implement_trait)] // `from_str` is the crate-wide naming convention.
    pub fn from_str(value: &str) -> Result<Self, TypeError> {
        crate::log_event!(trace, "BinaryType::from_str {:?}", value);
        match value {
            "binary" => Ok(Self { large: false }),
            "large_binary" => Ok(Self { large: true }),
            other => Err(TypeError::UnknownType(other.to_string())),
        }
    }

    /// Reconstructs the type from its component map (reads the `"type"` key).
    pub fn from_mapping(map: &BTreeMap<String, String>) -> Result<Self, TypeError> {
        let name = map
            .get("type")
            .ok_or_else(|| TypeError::InvalidMapping("missing \"type\" key".to_string()))?;
        Self::from_str(name)
    }

    /// Reconstructs the type from the bytes produced by [`DataType::to_bytes`].
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, TypeError> {
        let name = std::str::from_utf8(bytes).map_err(|_| TypeError::InvalidUtf8)?;
        Self::from_str(name)
    }
}

#[cfg(feature = "json")]
impl crate::Jsonable for BinaryType {}

impl DataType for BinaryType {
    fn type_name(&self) -> &'static str {
        if self.large {
            "large_binary"
        } else {
            "binary"
        }
    }

    fn category(&self) -> TypeCategory {
        TypeCategory::Primitive
    }

    fn to_any(&self) -> AnyType {
        AnyType::Binary(*self)
    }
}

impl PrimitiveType for BinaryType {}

impl BinaryBased for BinaryType {
    fn is_utf8(&self) -> bool {
        false
    }

    fn is_large(&self) -> bool {
        self.large
    }
}

impl From<BinaryType> for String {
    fn from(value: BinaryType) -> Self {
        value.type_name().to_string()
    }
}

impl TryFrom<String> for BinaryType {
    type Error = TypeError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        BinaryType::from_str(&value)
    }
}
