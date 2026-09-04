//! Geometry and geography datatypes.

mod dtypes;
mod parser;

#[cfg(feature = "parquet")]
pub(crate) use dtypes::DEFAULT_CRS;
pub use dtypes::GeospatialType;
pub(crate) use dtypes::{GEOARROW_WKB_EXTENSION_NAME, VARIANT_EXTENSION_NAME};
