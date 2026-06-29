//! # yggdryl-field
//!
//! Arrow fields — a named, nullable, typed column with optional metadata.
//!
//! [`Field`] is generic over its data type `T`, so a `Field<BinaryType>` carries
//! the static knowledge that it is a [`PrimitiveField`], while the type-erased
//! [`AnyField`] (`Field<AnyType>`) is what the bindings and schemas store. The
//! category marker traits ([`PrimitiveField`], [`NestedField`], [`LogicalField`])
//! mirror the type-side categories ([`PrimitiveType`], [`NestedType`],
//! [`LogicalType`]) and are implemented automatically from `T`'s category.

/// Emits a `log` event when the `log` feature is enabled, and expands to nothing
/// otherwise. Shared by every submodule via `crate::log_event!`.
macro_rules! log_event {
    ($level:ident, $($arg:tt)+) => {{
        #[cfg(feature = "log")]
        log::$level!($($arg)+);
    }};
}
pub(crate) use log_event;

use std::collections::BTreeMap;

use yggdryl_core::mapping::{decode_pairs, encode_pairs};
use yggdryl_core::FieldError;
use yggdryl_dtype::{AnyType, DataType, LogicalType, NestedType, PrimitiveType};

/// A named, nullable, typed field with string→string metadata.
///
/// ```
/// use yggdryl_dtype::{BinaryType, DataType};
/// use yggdryl_field::{AnyField, Field};
///
/// let field = Field::new("payload", BinaryType::new(), true);
/// assert_eq!(field.name(), "payload");
/// assert!(field.is_nullable());
///
/// let any: AnyField = field.to_any();
/// assert_eq!(any.data_type().to_str(), "binary");
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Field<T = AnyType> {
    name: String,
    #[cfg_attr(feature = "serde", serde(rename = "type"))]
    data_type: T,
    nullable: bool,
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "BTreeMap::is_empty")
    )]
    metadata: BTreeMap<String, String>,
}

/// A [`Field`] whose data type has been erased to [`AnyType`] — the form stored
/// by schemas and surfaced to the bindings.
pub type AnyField = Field<AnyType>;

impl<T> Field<T> {
    /// Builds a field from its parts (metadata starts empty).
    pub fn new(name: impl Into<String>, data_type: T, nullable: bool) -> Self {
        Self {
            name: name.into(),
            data_type,
            nullable,
            metadata: BTreeMap::new(),
        }
    }

    /// The field's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The field's data type.
    pub fn data_type(&self) -> &T {
        &self.data_type
    }

    /// Whether the field admits null values.
    pub fn is_nullable(&self) -> bool {
        self.nullable
    }

    /// The field's metadata map.
    pub fn metadata(&self) -> &BTreeMap<String, String> {
        &self.metadata
    }
}

impl<T: Clone> Field<T> {
    /// Returns a copy with a different name.
    pub fn with_name(&self, name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..self.clone()
        }
    }

    /// Returns a copy with a different nullability.
    pub fn with_nullable(&self, nullable: bool) -> Self {
        Self {
            nullable,
            ..self.clone()
        }
    }

    /// Returns a copy with the given metadata.
    pub fn with_metadata(&self, metadata: BTreeMap<String, String>) -> Self {
        Self {
            metadata,
            ..self.clone()
        }
    }

    /// Returns a copy with the metadata cleared.
    pub fn without_metadata(&self) -> Self {
        Self {
            metadata: BTreeMap::new(),
            ..self.clone()
        }
    }
}

impl<T: DataType + Clone> Field<T> {
    /// Erases the field's data type to [`AnyType`].
    pub fn to_any(&self) -> AnyField {
        Field {
            name: self.name.clone(),
            data_type: self.data_type.to_any(),
            nullable: self.nullable,
            metadata: self.metadata.clone(),
        }
    }

    /// The component map: `name`, `type`, `nullable`, and each metadata entry
    /// under a `metadata.<key>` key.
    pub fn to_mapping(&self) -> BTreeMap<String, String> {
        let mut map = BTreeMap::new();
        map.insert("name".to_string(), self.name.clone());
        map.insert("type".to_string(), self.data_type.to_str().into_owned());
        map.insert("nullable".to_string(), self.nullable.to_string());
        for (key, value) in &self.metadata {
            map.insert(format!("metadata.{key}"), value.clone());
        }
        map
    }

    /// The canonical, length-prefixed byte form (the encoded [`to_mapping`]).
    ///
    /// [`to_mapping`]: Field::to_mapping
    pub fn to_bytes(&self) -> Vec<u8> {
        encode_pairs(&self.to_mapping())
    }
}

impl AnyField {
    /// Reconstructs a field from the component map produced by
    /// [`to_mapping`](Field::to_mapping). A missing `nullable` defaults to `true`
    /// (Arrow's convention).
    pub fn from_mapping(map: &BTreeMap<String, String>) -> Result<Self, FieldError> {
        crate::log_event!(trace, "AnyField::from_mapping");
        let name = map
            .get("name")
            .ok_or(FieldError::MissingKey("name"))?
            .clone();
        let type_name = map.get("type").ok_or(FieldError::MissingKey("type"))?;
        let data_type = AnyType::from_str(type_name)?;
        let nullable = match map.get("nullable").map(String::as_str) {
            Some("true") => true,
            Some("false") => false,
            None => {
                crate::log_event!(
                    warn,
                    "AnyField::from_mapping missing \"nullable\", defaulting to true"
                );
                true
            }
            Some(other) => {
                return Err(FieldError::InvalidMapping(format!(
                    "nullable must be \"true\" or \"false\", got {other:?}"
                )))
            }
        };
        let mut metadata = BTreeMap::new();
        for (key, value) in map {
            if let Some(meta_key) = key.strip_prefix("metadata.") {
                metadata.insert(meta_key.to_string(), value.clone());
            }
        }
        Ok(Field {
            name,
            data_type,
            nullable,
            metadata,
        })
    }

    /// Reconstructs a field from the bytes produced by [`to_bytes`](Field::to_bytes).
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, FieldError> {
        crate::log_event!(trace, "AnyField::from_bytes {} bytes", bytes.len());
        let map = decode_pairs(bytes).map_err(FieldError::InvalidMapping)?;
        Self::from_mapping(&map)
    }
}

/// JSON (`{"name", "type", "nullable", "metadata"}`) for any field whose data type
/// is itself serializable.
#[cfg(feature = "json")]
impl<T> yggdryl_core::Jsonable for Field<T> where T: serde::Serialize + serde::de::DeserializeOwned {}

/// A field whose data type is a [`PrimitiveType`].
pub trait PrimitiveField {}
impl<T: PrimitiveType> PrimitiveField for Field<T> {}

/// A field whose data type is a [`NestedType`].
pub trait NestedField {}
impl<T: NestedType> NestedField for Field<T> {}

/// A field whose data type is a [`LogicalType`].
pub trait LogicalField {}
impl<T: LogicalType> LogicalField for Field<T> {}
