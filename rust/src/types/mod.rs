//! Core datatypes, fields, scalars, and their family-owned behavior.

mod arrow;
pub mod ascii;
pub mod boolean;
pub mod bytes;
mod compatibility;
pub mod decimal;
mod default;
mod diff;
mod dtype;
pub mod floating;
pub mod geospatial;
pub mod guid;
pub mod integer;
mod merge;
pub mod nested;
mod parser;
pub(crate) mod serde;
pub mod temporal;
pub mod text;
mod vocabulary;

pub use crate::{TimeUnit, UnionMode};
pub(crate) use arrow::{arrow_dtype_to_ffi, arrow_extension_parts, is_variant_storage};
pub use ascii::AsciiEnum;
#[cfg(feature = "arrow")]
pub(crate) use ascii::ascii_padded;
pub(crate) use ascii::{
    ASCII_EXTENSION_NAME, CFI_WIDTH, COUNTRY_WIDTH, CURRENCY_WIDTH, MIC_WIDTH, ascii_bytes,
    ascii_free_text, ascii_text, code_cell_text, code_for_extension, code_refusal, code_text,
};
pub(crate) use default::{
    default_value_for_field, preflight_schema, preflight_schema_shape, value_is_logically_null,
};
pub use dtype::DataType;
pub(crate) use dtype::{invalid, validate_non_negative};
#[cfg(feature = "parquet")]
pub(crate) use geospatial::DEFAULT_CRS;
pub use geospatial::GeospatialType;
pub(crate) use geospatial::{GEOARROW_WKB_EXTENSION_NAME, VARIANT_EXTENSION_NAME};
pub(crate) use guid::{GUID_EXTENSION_NAME, guid_bytes, guid_parse, guid_text};
pub(crate) use merge::Recode;
pub use merge::Widening;
pub use nested::{DictionaryType, FieldKey, Fields, MapType, RunEndEncodedType, UnionFields};

#[cfg(test)]
mod tests;
