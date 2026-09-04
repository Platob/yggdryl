//! Geometry and geography datatypes.

#[cfg(feature = "arrow")]
pub(crate) mod casts;

mod dtypes;
mod fields;
mod parser;
mod scalars;
pub mod wkb;

#[cfg(feature = "parquet")]
pub(crate) use dtypes::DEFAULT_CRS;
pub use dtypes::GeospatialType;
pub(crate) use dtypes::{GEOARROW_WKB_EXTENSION_NAME, VARIANT_EXTENSION_NAME};
pub use fields::*;
pub use scalars::{GeographyScalar, GeometryScalar};
