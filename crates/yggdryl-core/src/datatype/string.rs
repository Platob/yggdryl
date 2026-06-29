//! The [`Utf8`] data type — variable-length UTF-8 strings.

use std::collections::BTreeMap;

use crate::datatype::{AnyType, BinaryBased, DataType, PrimitiveType, TypeCategory};
use crate::error::TypeError;

/// Arrow's variable-length UTF-8 string type, in both its 32-bit (`string`) and
/// 64-bit (`large_string`) offset flavours.
///
/// Named `Utf8` after Arrow's own type (and to stay clear of Rust's `String` and
/// the JavaScript `String` global); `from_str` also accepts the aliases `"utf8"`
/// and `"large_utf8"`.
///
/// ```
/// use yggdryl_core::{BinaryBased, DataType, Utf8};
///
/// let s = Utf8::new();
/// assert_eq!(s.type_name(), "string");
/// assert!(s.is_utf8());
/// assert_eq!(Utf8::from_str("utf8").unwrap(), Utf8::new());
/// assert_eq!(Utf8::large().type_name(), "large_string");
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(into = "String", try_from = "String"))]
pub struct Utf8 {
    large: bool,
}

impl Utf8 {
    /// The 32-bit-offset `string` type.
    pub fn new() -> Self {
        Self { large: false }
    }

    /// The 64-bit-offset `large_string` type.
    pub fn large() -> Self {
        Self { large: true }
    }

    /// Parses `"string"`/`"utf8"` or `"large_string"`/`"large_utf8"`.
    #[allow(clippy::should_implement_trait)] // `from_str` is the crate-wide naming convention.
    pub fn from_str(value: &str) -> Result<Self, TypeError> {
        crate::log_event!(trace, "Utf8::from_str {:?}", value);
        match value {
            "string" | "utf8" => Ok(Self { large: false }),
            "large_string" | "large_utf8" => Ok(Self { large: true }),
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

    /// The JSON form (the canonical string as a JSON string).
    #[cfg(feature = "json")]
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("Utf8 serializes to a JSON string")
    }

    /// Parses the JSON form produced by [`Utf8::to_json`].
    #[cfg(feature = "json")]
    pub fn from_json(value: &str) -> Result<Self, TypeError> {
        serde_json::from_str(value).map_err(|err| TypeError::InvalidMapping(err.to_string()))
    }
}

impl DataType for Utf8 {
    fn type_name(&self) -> &'static str {
        if self.large {
            "large_string"
        } else {
            "string"
        }
    }

    fn category(&self) -> TypeCategory {
        TypeCategory::Primitive
    }

    fn to_any(&self) -> AnyType {
        AnyType::Utf8(*self)
    }
}

impl PrimitiveType for Utf8 {}

impl BinaryBased for Utf8 {
    fn is_utf8(&self) -> bool {
        true
    }

    fn is_large(&self) -> bool {
        self.large
    }
}

impl From<Utf8> for String {
    fn from(value: Utf8) -> Self {
        value.type_name().to_string()
    }
}

impl TryFrom<String> for Utf8 {
    type Error = TypeError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Utf8::from_str(&value)
    }
}
