//! Geospatial values and typed scalar aliases.

use std::fmt;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::{DataType, DataTypeId, DataTypeKind, Result, Scalar, ScalarValue};

/// Borrowing access shared by geometry and geography values.
pub trait GeospatialValue: ScalarValue {
    /// Borrow the validated Well-Known Binary payload.
    fn as_bytes(&self) -> &[u8];
    /// Borrow the shared storage behind the payload.
    ///
    /// The payload is already validated WKB, so reinterpreting a geometry as
    /// a geography clones this handle rather than copying and re-reading it.
    fn storage(&self) -> &Arc<[u8]>;
}

macro_rules! geospatial_leaf {
    ($name:ident) => {
        #[doc = concat!("One validated `", stringify!($name), "` WKB value.")]
        #[repr(transparent)]
        #[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
        #[serde(transparent)]
        pub struct $name(Arc<[u8]>);

        impl $name {
            /// Validate and construct a WKB value.
            pub fn new(value: impl Into<Arc<[u8]>>) -> Result<Self> {
                let value = value.into();
                super::wkb::Geometry::from_slice(value.as_ref())?;
                Ok(Self(value))
            }

            /// Borrow the canonical WKB bytes.
            pub fn as_bytes(&self) -> &[u8] {
                self.0.as_ref()
            }

            /// Borrow the shared storage without copying the payload.
            pub fn storage(&self) -> &Arc<[u8]> {
                &self.0
            }

            /// Consume this value and return its shared WKB bytes.
            pub fn into_inner(self) -> Arc<[u8]> {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                for byte in self.as_bytes() {
                    write!(formatter, "{byte:02x}")?;
                }
                Ok(())
            }
        }
    };
}

geospatial_leaf!(Geometry);
geospatial_leaf!(Geography);

const _: () = assert!(std::mem::size_of::<Geometry>() == 16);
const _: () = assert!(std::mem::size_of::<Geography>() == 16);

// Each interpretation is its own family, as every width leaf is: the `Scalar`
// variant holds the leaf directly, so there is no grouping enum to widen into.
// Geometry and geography differ in the coordinate reference they name, not in
// the bytes, so rewriting one as the other shares the storage handle.
macro_rules! geospatial_value {
    ($leaf:ident, $dtype:expr) => {
        impl ScalarValue for $leaf {

            const ID: DataTypeId = DataTypeId::$leaf;
            const KIND: DataTypeKind = DataTypeKind::Geospatial;

            fn dtype(&self) -> Result<DataType> {
                $dtype
            }

            fn into_scalar(self) -> Scalar {
                Scalar::$leaf(self)
            }

            fn from_scalar(value: &Scalar) -> Option<&Self> {
                match value {
                    Scalar::$leaf(value) => Some(value),
                    _ => None,
                }
            }
        }

        impl GeospatialValue for $leaf {
            fn as_bytes(&self) -> &[u8] {
                <$leaf>::as_bytes(self)
            }

            fn storage(&self) -> &Arc<[u8]> {
                <$leaf>::storage(self)
            }
        }
    };
}

geospatial_value!(Geometry, DataType::geometry(None));
geospatial_value!(Geography, DataType::geography(None, None));
