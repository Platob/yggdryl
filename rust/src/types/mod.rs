//! Core datatypes, fields, scalars, and their family-owned behavior.

mod arithmetic;
mod arrow;
pub mod ascii;
pub mod boolean;
#[cfg(feature = "arrow")]
pub(crate) mod budget;
pub mod bytes;
mod compatibility;
pub mod decimal;
mod default;
mod diff;
mod dtype;
mod enumeration;
mod field;
pub mod floating;
pub mod geospatial;
pub mod integer;
mod merge;
pub mod nested;
mod parser;
mod pretty;
pub mod protocol;
mod regex;
mod scalar;
pub(crate) mod serde;
pub mod temporal;
pub mod text;
mod typed;
pub mod url;
pub mod uuid;
mod value;
pub mod version;
mod vocabulary;

#[cfg(feature = "arrow")]
pub mod cast;

pub use crate::{TimeUnit, UnionMode};
pub(crate) use arithmetic::Arithmetic;
#[cfg(feature = "arrow")]
pub(crate) use arrow::{RecognizedExtension, recognized_arrow_extension};
pub(crate) use arrow::{arrow_dtype_to_ffi, arrow_extension_parts, is_variant_storage};
pub use ascii::*;
pub(crate) use ascii::{
    ASCII_EXTENSION_NAME, ascii_bytes, ascii_document_width, ascii_free_text, ascii_text,
    code_cell_text, code_for_extension,
};
#[cfg(feature = "arrow")]
pub(crate) use ascii::{
    CFI_WIDTH, COUNTRY_WIDTH, CURRENCY_WIDTH, DIRECTION_WIDTH, MIC_WIDTH, SIDE_WIDTH, STATE_WIDTH,
    TIMEINFORCE_WIDTH, ascii_value_text, code_refusal, code_text, code_value_text,
};
pub use boolean::*;
pub use bytes::*;
pub use decimal::*;
pub(crate) use default::{
    default_value_for_field, preflight_schema, preflight_schema_shape, value_is_logically_null,
};
pub(crate) use diff::push_field_name_path;
pub use diff::{Differences, OwnedDifferences};
pub use dtype::DataType;
pub(crate) use dtype::{invalid, validate_non_negative};
pub use enumeration::Enum;
pub use field::*;
pub use floating::scalars::Floating;
pub use floating::*;
#[cfg(feature = "parquet")]
pub(crate) use geospatial::DEFAULT_CRS;
pub use geospatial::*;
pub(crate) use geospatial::{GEOARROW_WKB_EXTENSION_NAME, VARIANT_EXTENSION_NAME};
pub use integer::scalars::Integer;
pub use integer::*;
pub(crate) use merge::Recode;
pub use merge::Widening;
pub use nested::*;
pub(crate) use parser::{folds_equal, normalized};
pub use pretty::Pretty;
pub use scalar::{Scalar, ScalarFamily, ScalarValue};
pub use temporal::scalars::TemporalFamily;
pub use temporal::*;
pub use text::*;
pub use typed::{
    FieldRecord, FieldScalar, FieldType, TypedField, TypedFieldRef, UncheckedFieldScalar,
};
pub use url::*;
pub use uuid::*;
pub(crate) use uuid::{UUID_EXTENSION_NAME, uuid_bytes, uuid_parse, uuid_text};
pub(crate) use value::dtype_scalar;
pub(crate) use version::VERSION_EXTENSION_NAME;
pub use version::*;

#[cfg(test)]
mod tests;
