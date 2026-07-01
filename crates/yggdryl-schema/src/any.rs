//! The dynamic [`AnyType`] — a hashable, serializable enum that can hold any
//! concrete yggdryl data type, so a [`Field`](crate::Field) can carry a data type
//! chosen at run time.
//!
//! `AnyType` is the carrier every concrete type converts into via `From`, and it
//! [delegates](DataType) each [`DataType`] method to the wrapped type — its
//! category, `byte_size`, metadata and Arrow mapping are exactly the inner type's.
//! [`from_arrow_type`](DataType::from_arrow_type) reads the reserved `yggdryl:type`
//! metadata to pick the exact variant (so types Arrow maps lossily still round-trip),
//! falling back to the plain Arrow type when that metadata is absent.
//!
//! ```
//! use yggdryl_schema::{AnyType, BinaryType, DataType, DataTypeId, LargeBinaryViewType};
//!
//! let ty = AnyType::from(BinaryType::new().with_byte_size(8));
//! assert_eq!(ty.name(), "binary");
//! assert_eq!(ty.type_id(), DataTypeId::Binary);
//! assert_eq!(ty.max_byte_size(), Some(8));
//! assert!(ty.is_physical());
//!
//! // A lossily-mapped type still reports its own identity through the enum.
//! let view = AnyType::from(LargeBinaryViewType::new());
//! assert_eq!(view.type_id(), DataTypeId::LargeBinaryView);
//! ```

use crate::binary::{BinaryType, BinaryViewType, LargeBinaryType, LargeBinaryViewType};
use crate::data_type::DataType;
use crate::data_type_id::DataTypeId;
use crate::metadata::Metadata;

/// A concrete, hashable, serializable yggdryl data type — the dynamic carrier a
/// [`Field`](crate::Field) stores when its type is chosen at run time. Every
/// concrete type converts into it via [`From`], and it delegates the whole
/// [`DataType`] surface to the wrapped type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum AnyType {
    /// A [`BinaryType`].
    Binary(BinaryType),
    /// A [`LargeBinaryType`].
    LargeBinary(LargeBinaryType),
    /// A [`BinaryViewType`].
    BinaryView(BinaryViewType),
    /// A [`LargeBinaryViewType`].
    LargeBinaryView(LargeBinaryViewType),
}

/// Runs `$body` against the wrapped concrete type, bound as `$inner`, whichever
/// variant `$self` holds.
macro_rules! dispatch {
    ($self:ident, $inner:ident => $body:expr) => {
        match $self {
            AnyType::Binary($inner) => $body,
            AnyType::LargeBinary($inner) => $body,
            AnyType::BinaryView($inner) => $body,
            AnyType::LargeBinaryView($inner) => $body,
        }
    };
}

impl DataType for AnyType {
    fn name(&self) -> &'static str {
        dispatch!(self, inner => inner.name())
    }

    fn type_id(&self) -> DataTypeId {
        dispatch!(self, inner => inner.type_id())
    }

    fn max_byte_size(&self) -> Option<i64> {
        dispatch!(self, inner => inner.max_byte_size())
    }

    fn metadata(&self) -> Metadata {
        dispatch!(self, inner => inner.metadata())
    }

    #[cfg(feature = "arrow")]
    fn to_arrow_type(&self) -> arrow_schema::DataType {
        dispatch!(self, inner => inner.to_arrow_type())
    }

    #[cfg(feature = "arrow")]
    fn from_arrow_type(
        dtype: &arrow_schema::DataType,
        metadata: &Metadata,
    ) -> Result<Self, crate::SchemaError> {
        // The reserved `yggdryl:type` name is authoritative — it disambiguates the
        // types Arrow maps lossily (e.g. `large_binary_view` → `BinaryView`).
        // Without it (a plain Arrow field), infer the variant from the Arrow type.
        match metadata.get(&crate::metadata::reserved_key(crate::metadata::TYPE_KEY)) {
            Some(name) => {
                let name =
                    std::str::from_utf8(name).map_err(|_| crate::SchemaError::NonUtf8Metadata)?;
                Self::from_named_arrow_type(name, dtype, metadata)
            }
            None => Self::infer_from_arrow_type(dtype, metadata),
        }
    }
}

impl AnyType {
    /// Rebuilds the variant named by the reserved `yggdryl:type` metadata,
    /// delegating to that concrete type's own
    /// [`from_arrow_type`](DataType::from_arrow_type).
    #[cfg(feature = "arrow")]
    fn from_named_arrow_type(
        name: &str,
        dtype: &arrow_schema::DataType,
        metadata: &Metadata,
    ) -> Result<Self, crate::SchemaError> {
        match name {
            "binary" => BinaryType::from_arrow_type(dtype, metadata).map(AnyType::Binary),
            "large_binary" => {
                LargeBinaryType::from_arrow_type(dtype, metadata).map(AnyType::LargeBinary)
            }
            "binary_view" => {
                BinaryViewType::from_arrow_type(dtype, metadata).map(AnyType::BinaryView)
            }
            "large_binary_view" => {
                LargeBinaryViewType::from_arrow_type(dtype, metadata).map(AnyType::LargeBinaryView)
            }
            _ => Err(crate::SchemaError::UnsupportedArrowType(dtype.clone())),
        }
    }

    /// Infers the variant from a bare Arrow type (no `yggdryl:type` metadata),
    /// choosing the non-`large` view variant where Arrow cannot tell them apart.
    #[cfg(feature = "arrow")]
    fn infer_from_arrow_type(
        dtype: &arrow_schema::DataType,
        metadata: &Metadata,
    ) -> Result<Self, crate::SchemaError> {
        use arrow_schema::DataType as ArrowType;
        let name = match dtype {
            ArrowType::Binary => "binary",
            ArrowType::LargeBinary => "large_binary",
            ArrowType::BinaryView => "binary_view",
            other => return Err(crate::SchemaError::UnsupportedArrowType(other.clone())),
        };
        Self::from_named_arrow_type(name, dtype, metadata)
    }
}

/// Generates the `From<ConcreteType>` conversion into the matching variant.
macro_rules! any_from {
    ($($variant:ident => $ty:ty),+ $(,)?) => {$(
        impl From<$ty> for AnyType {
            fn from(inner: $ty) -> Self {
                AnyType::$variant(inner)
            }
        }
    )+};
}

any_from! {
    Binary => BinaryType,
    LargeBinary => LargeBinaryType,
    BinaryView => BinaryViewType,
    LargeBinaryView => LargeBinaryViewType,
}
