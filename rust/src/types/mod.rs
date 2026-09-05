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
mod scalar;
pub(crate) mod serde;
pub mod temporal;
pub mod text;
mod typed;
pub mod uuid;
mod value;
mod vocabulary;

#[cfg(feature = "arrow")]
pub mod cast;

pub use crate::{TimeUnit, UnionMode};
pub(crate) use arithmetic::Arithmetic;
#[cfg(feature = "arrow")]
pub(crate) use arrow::{RecognizedExtension, recognized_arrow_extension};
pub(crate) use arrow::{arrow_dtype_to_ffi, arrow_extension_parts, is_variant_storage};
#[cfg(feature = "arrow")]
pub(crate) use ascii::ascii_padded;
pub use ascii::*;
pub(crate) use ascii::{
    ASCII_EXTENSION_NAME, ascii_bytes, ascii_free_text, ascii_text, code_cell_text,
    code_for_extension,
};
pub use ascii::{
    AsciiScalar, CfiScalar, CountryScalar, CurrencyScalar, FixedAsciiScalar, MicScalar,
};
#[cfg(feature = "arrow")]
pub(crate) use ascii::{
    CFI_WIDTH, COUNTRY_WIDTH, CURRENCY_WIDTH, MIC_WIDTH, code_refusal, code_text,
};
pub use boolean::*;
pub use boolean::{BooleanScalar, NullScalar};
pub use bytes::*;
pub use bytes::{BinaryScalar, BinaryViewScalar, FixedSizeBinaryScalar, LargeBinaryScalar};
pub use decimal::scalars::{Decimal32Scalar, Decimal64Scalar, Decimal128Scalar, Decimal256Scalar};
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
pub use floating::scalars::{Float16Scalar, Float32Scalar, Float64Scalar, Floating};
pub use floating::*;
#[cfg(feature = "parquet")]
pub(crate) use geospatial::DEFAULT_CRS;
pub use geospatial::*;
pub(crate) use geospatial::{GEOARROW_WKB_EXTENSION_NAME, VARIANT_EXTENSION_NAME};
pub use geospatial::{GeographyScalar, GeometryScalar};
pub use integer::scalars::{
    Int8Scalar, Int16Scalar, Int32Scalar, Int64Scalar, Integer, UInt8Scalar, UInt16Scalar,
    UInt32Scalar, UInt64Scalar,
};
pub use integer::*;
pub(crate) use merge::Recode;
pub use merge::Widening;
pub use nested::*;
pub use nested::{
    DictionaryScalar, FixedSizeListScalar, LargeListScalar, LargeListViewScalar, ListScalar,
    ListViewScalar, MapScalar, RunEndEncodedScalar, StructScalar, UnionScalar, VariantScalar,
};
pub use pretty::Pretty;
pub use scalar::{Scalar, ScalarFamily, ScalarValue};
pub use temporal::scalars::{
    Date32Scalar, Date64Scalar, DateTime64Scalar, Duration32Scalar, Duration64Scalar,
    IntervalScalar, TemporalFamily, Time32Scalar, Time64Scalar,
};
pub use temporal::*;
pub use text::*;
pub use text::{LargeUtf8Scalar, Utf8Scalar, Utf8ViewScalar};
pub use typed::{AnyType, FieldType, TypedField, TypedFieldRef, TypedScalar};
pub use uuid::UuidScalar;
pub use uuid::*;
pub(crate) use uuid::{UUID_EXTENSION_NAME, uuid_bytes, uuid_parse, uuid_text};
pub(crate) use value::{dtype_scalar, validate_dtype_value_for};

#[cfg(test)]
mod tests;
