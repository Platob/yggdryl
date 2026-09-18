//! Core datatypes, fields, scalars, and their family-owned behavior.

mod arithmetic;
pub mod bloomberg;
pub mod boolean;
pub(crate) mod budget;
pub mod bytes;
pub mod cfi;
pub mod code;
mod compatibility;
pub mod country;
pub mod currency;
pub mod cusip;
pub mod decimal;
mod default;
mod diff;
mod dtype;
mod enumeration;
pub mod enums;
mod family;
mod field;
pub mod floating;
pub mod geospatial;
pub(crate) mod i256;
pub mod integer;
pub mod isin;
pub mod mapping;
pub mod media_type;
mod merge;
pub mod mic;
pub mod mime_type;
mod parser;
mod pretty;
pub mod protocol;
mod regex;
pub mod runend;
mod scalar;
pub mod sedol;
pub mod sequence;
pub(crate) mod serde;
pub mod side;
pub mod state;
pub mod string;
pub mod structure;
pub mod temporal;
pub mod timeinforce;
pub mod timezone;
mod typed;
pub mod union;
pub mod url;
pub mod uuid;
mod value;
pub mod version;
mod vocabulary;
pub mod wkb;

pub mod cast;

pub use crate::{TimeUnit, UnionMode};
pub(crate) use arithmetic::Arithmetic;
pub use bloomberg::*;
pub use boolean::*;
pub use bytes::*;
pub use cfi::*;
pub use code::*;
pub(crate) use code::{code_cell_text, code_for_extension};
pub(crate) use code::{code_refusal, code_text};
pub use country::*;
pub use currency::*;
pub use cusip::*;
pub use decimal::*;
pub(crate) use default::{
    default_value_for_field, preflight_schema, preflight_schema_shape, value_is_logically_null,
};
pub(crate) use diff::push_field_name_path;
pub use diff::{Differences, OwnedDifferences};
pub use dtype::{DataType, VariantType};
pub(crate) use dtype::{invalid, validate_non_negative};
pub use enumeration::Vocabulary;
pub use enums::*;
pub use family::{
    Children, DataTypeValue, DateTime64Type, DictionaryOptions, Duration32Type, Duration64Type,
    FieldSidecar, FieldValue, GeographyType, GeometryType, IntervalType, NestedValue, RunEndType,
    Time32Type, Time64Type, UnionType,
};
pub use field::*;
pub use floating::*;
#[cfg(feature = "parquet")]
pub(crate) use geospatial::DEFAULT_CRS;
pub use geospatial::*;
pub(crate) use geospatial::{
    GEOARROW_WKB_EXTENSION_NAME, VARIANT_EXTENSION_NAME, is_variant_storage,
};
pub use integer::*;
pub use isin::*;
pub use mapping::*;
pub(crate) use media_type::MEDIATYPE_EXTENSION_NAME;
pub use media_type::MediaTypeType;
pub(crate) use merge::Recode;
pub use merge::Widening;
pub use mic::*;
pub(crate) use mime_type::MIMETYPE_EXTENSION_NAME;
pub use mime_type::MimeTypeType;
pub(crate) use parser::{folds_equal, normalized};
pub use pretty::Pretty;
pub use runend::*;
pub(crate) use scalar::code_scalars;
pub use scalar::{Scalar, Value};
pub use sedol::*;
pub use sequence::*;
pub use side::*;
pub use state::*;
pub use string::*;
pub(crate) use string::{ascii_bytes, ascii_text, ascii_text_sized, trim_padding};
pub use structure::*;
pub use temporal::TemporalFamily;
pub use temporal::*;
pub use timeinforce::*;
pub(crate) use timezone::TIMEZONE_EXTENSION_NAME;
pub use timezone::{Timezone, TimezoneType};
pub use typed::{FieldRecord, FieldScalar, UncheckedFieldScalar};
pub use union::*;
pub use url::*;
pub use uuid::*;
pub(crate) use uuid::{
    UUID_EXTENSION_NAME, UUID_TEXT_LEN, UUID_VERSION_EXTENSION_NAME, uuid_bytes, uuid_parse,
    uuid_rendered, uuid_text,
};
pub(crate) use value::dtype_scalar;
pub(crate) use version::VERSION_EXTENSION_NAME;
pub use version::*;
