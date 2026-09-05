//! Geospatial values and typed scalar aliases.

use std::fmt;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::Result;
use crate::types::typed::define_scalar_type;

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

define_scalar_type!(GeometryScalar, super::GeometryType, "geometry");
define_scalar_type!(GeographyScalar, super::GeographyType, "geography");
