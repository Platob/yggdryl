//! Geospatial values and typed scalar aliases.

use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::types::typed::define_scalar_type;
use crate::{DataType, DataTypeId, DataTypeKind, Result, Scalar, ScalarFamily, ScalarValue};

/// Borrowing access shared by geometry and geography values.
pub trait GeospatialValue: ScalarValue {
    /// Borrow the validated Well-Known Binary payload.
    fn as_bytes(&self) -> &[u8];
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

/// One exact geospatial interpretation over validated WKB.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[non_exhaustive]
pub enum Geospatial {
    /// Planar geometry.
    Geometry(Geometry),
    /// Geographic coordinates.
    Geography(Geography),
}

impl Geospatial {
    /// Borrow the WKB payload independently of its interpretation.
    pub fn as_bytes(&self) -> &[u8] {
        match self {
            Self::Geometry(value) => value.as_bytes(),
            Self::Geography(value) => value.as_bytes(),
        }
    }
}

impl fmt::Display for Geospatial {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.as_bytes() {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl PartialEq for Geospatial {
    fn eq(&self, other: &Self) -> bool {
        self.as_bytes() == other.as_bytes()
    }
}

impl Eq for Geospatial {}

impl PartialOrd for Geospatial {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Geospatial {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.as_bytes().cmp(other.as_bytes())
    }
}

impl Hash for Geospatial {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_bytes().hash(state);
    }
}

const _: () = assert!(std::mem::size_of::<Geospatial>() == 24);

macro_rules! geospatial_value {
    ($leaf:ident, $marker:ty, $variant:ident, $id:ident, $constructor:ident) => {
        impl ScalarValue for $leaf {
            type Family = Geospatial;
            type Type = $marker;

            const ID: DataTypeId = DataTypeId::$id;
            const KIND: DataTypeKind = DataTypeKind::Geospatial;

            fn dtype(&self) -> Result<DataType> {
                DataType::$constructor(None)
            }

            fn into_family(self) -> Self::Family {
                Geospatial::$variant(self)
            }

            fn from_family(family: &Self::Family) -> Option<&Self> {
                match family {
                    Geospatial::$variant(value) => Some(value),
                    _ => None,
                }
            }

            fn into_scalar(self) -> Scalar {
                Scalar::Geospatial(Geospatial::$variant(self))
            }

            fn from_scalar(value: &Scalar) -> Option<&Self> {
                match value {
                    Scalar::Geospatial(Geospatial::$variant(value)) => Some(value),
                    _ => None,
                }
            }
        }

        impl GeospatialValue for $leaf {
            fn as_bytes(&self) -> &[u8] {
                <$leaf>::as_bytes(self)
            }
        }
    };
}

geospatial_value!(Geometry, super::GeometryType, Geometry, Geometry, geometry);

impl ScalarValue for Geography {
    type Family = Geospatial;
    type Type = super::GeographyType;

    const ID: DataTypeId = DataTypeId::Geography;
    const KIND: DataTypeKind = DataTypeKind::Geospatial;

    fn dtype(&self) -> Result<DataType> {
        DataType::geography(None, None)
    }

    fn into_family(self) -> Self::Family {
        Geospatial::Geography(self)
    }

    fn from_family(family: &Self::Family) -> Option<&Self> {
        match family {
            Geospatial::Geography(value) => Some(value),
            _ => None,
        }
    }

    fn into_scalar(self) -> Scalar {
        Scalar::Geospatial(Geospatial::Geography(self))
    }

    fn from_scalar(value: &Scalar) -> Option<&Self> {
        match value {
            Scalar::Geospatial(Geospatial::Geography(value)) => Some(value),
            _ => None,
        }
    }
}

impl GeospatialValue for Geography {
    fn as_bytes(&self) -> &[u8] {
        Self::as_bytes(self)
    }
}

impl ScalarFamily for Geospatial {
    const KIND: DataTypeKind = DataTypeKind::Geospatial;

    fn id(&self) -> DataTypeId {
        match self {
            Self::Geometry(_) => DataTypeId::Geometry,
            Self::Geography(_) => DataTypeId::Geography,
        }
    }

    fn dtype(&self) -> Result<DataType> {
        match self {
            Self::Geometry(value) => ScalarValue::dtype(value),
            Self::Geography(value) => ScalarValue::dtype(value),
        }
    }

    fn into_scalar(self) -> Scalar {
        Scalar::Geospatial(self)
    }

    fn from_scalar(value: &Scalar) -> Option<&Self> {
        match value {
            Scalar::Geospatial(value) => Some(value),
            _ => None,
        }
    }
}

define_scalar_type!(GeometryScalar, super::GeometryType, "geometry");
define_scalar_type!(GeographyScalar, super::GeographyType, "geography");
