//! Allocation-conscious Arrow schemas, arrays, tabular records, resource
//! identifiers, and byte codecs.
//!
//! `yggdryl` keeps schema, identifier, metadata, and structured codec values
//! validated and cheap to clone. Strings, nested values, byte payloads,
//! metadata, and Arrow projections share immutable storage across common
//! read/clone/project paths. The default-enabled [`arrow`] module owns native
//! arrays, RecordBatches, bounded IPC, and tabular casting; disabling default
//! features retains the schema and codec core without that runtime.

#![deny(unsafe_code)]

pub mod arrow;
mod bytestream;
pub mod charset;
mod codec;
pub mod coding;
mod datatype_id;
mod datatype_kind;
mod digest;
mod edge_algorithm;
mod error;
mod fix_category;

pub use fix_category::FixCategory;
mod arithmetic;
pub mod ascii;
pub mod avro;
pub mod bloomberg;
pub mod boolean;
pub(crate) mod budget;
pub mod bytes;
pub mod cast;
pub mod cfi;
pub mod code;
mod compatibility;
pub mod country;
pub mod cp1252;
pub mod currency;
pub mod cusip;
mod datatype;
pub mod date;
pub mod datetime;
pub mod decimal;
mod default;
mod diff;
pub mod duration;
mod enumeration;
pub mod enums;
pub mod expression;
mod family;
mod field;
pub mod fix;
pub mod floating;
pub mod fs;
pub mod geospatial;
pub mod graph;
pub mod gzip;
pub mod hashing;
pub mod holder;
#[cfg(feature = "iceberg")]
pub mod iceberg;
#[cfg(not(feature = "iceberg"))]
#[path = "iceberg/types.rs"]
pub mod iceberg;
pub(crate) mod int256;
pub mod integer;
pub mod interval;
mod iobase;
mod iocursor;
mod iofile;
mod iofolder;
mod iokind;
mod iomedia;
mod iomode;
mod iopath;
pub mod ipc;
pub mod isin;
pub mod json;
mod listing;
pub mod local;
pub mod mapping;
pub mod media;
mod media_type;
mod merge;
mod metadata;
pub mod mic;
mod mime_type;
#[cfg(feature = "object")]
pub mod object;
#[cfg(feature = "parquet")]
pub mod parquet;
mod parser;
mod path;
mod pretty;
pub mod protocol;
mod regex;
pub mod runend;
mod scalar;
mod scheme;
pub mod sedol;
pub mod sequence;
pub(crate) mod serde;
pub mod side;
pub mod state;
pub mod string;
pub mod structure;
pub mod temporal;
pub mod text;
pub mod time;
mod time_unit;
pub mod timeinforce;
pub mod timezone;
pub mod toml;
pub mod txhash;
mod typed;
pub mod union;
mod union_mode;
mod uri;
pub mod url;
pub mod utf8;
pub mod uuid;
mod value;
pub mod version;
mod vocabulary;
pub mod wkb;
pub mod xxhash;
pub mod yaml;
pub mod zip;
pub mod zlib;
pub mod zstd;

pub use crate::json::{from_json_scalar, from_json_scalar_with_field, into_json_scalar};
pub use crate::toml::{from_toml_scalar, from_toml_scalar_with_field, into_toml_scalar};
pub use crate::yaml::{from_yaml_scalar, from_yaml_scalar_with_field, into_yaml_scalar};
pub use arrow::{ArrowScalar, ArrowShape};
pub use bytestream::ByteStream;
pub use cast::{ArrowCastOptions, ArrowCastPlan, ArrowFieldType, Nullability, Representation};
pub use charset::Charset;
pub use codec::{Codec, Encoder, Level, RestartScan, Restarts};
pub use datatype_id::DataTypeId;
pub use datatype_kind::DataTypeKind;
pub use digest::{Digest, DigestAlgorithm, DigestBytes, Digester};
pub use edge_algorithm::EdgeAlgorithm;
pub use error::{Error, Result};
pub use expression::{Expression, Filter, Plan, Selector, Term};
pub use expression::{FieldPath, FieldSegment};
pub use fix::MsgType;
pub use fix::{
    CRATE_TAG_MAX, CRATE_TAG_MIN, CREATUNIX_TAG_NAME, CROSSCODE_TAG_NAME, CROSSHASHCODE_TAG_NAME,
    CROSSUUID_TAG_NAME, CURRHASHCODE_TAG_NAME, CURRUNIX_TAG_NAME, CURRUUID_TAG_NAME,
    DEFAULT_NULL_VALUES, DEFAULT_PAYLOAD_COLUMN, DEFAULT_REFUSED_MSGTYPES, FIX_TYPED_TAGS,
    FIXMSG_TAG_NAME, FixCapture, FixCode, FixCodeValue, FixCodec, FixCodes, FixDedup, FixDirection,
    FixDirectionEntry, FixDirections, FixEntry, FixFieldIter, FixHeader, FixId, FixKey, FixLifted,
    FixMessages, FixMsg, FixPatterns, FixRegistry, FixSpellings, IDENTIFIERS_TAG_NAME,
    METADATA_TAG_NAME, MSGCTXID_TAG_NAME, MSGDIRECTION_TAG_NAME, MSGSESSIONID_TAG_NAME,
    NOFIXENTRIES_TAG_NAME, PARENTUUIDS_TAG_NAME, PLUGINID_TAG_NAME, PREVUNIX_TAG_NAME,
    PREVUUID_TAG_NAME, RECORDEDAT_TAG_NAME, SEQNUM_TAG_NAME, SNAPUNIX_TAG_NAME, SOH,
    SOURCEURL_TAG_NAME, STANDARD_HEADER_TAGS, STANDARD_TRAILER_TAGS, ULBRIDGE_ROWHEADER, Words,
    fix_column_of, fix_column_tags, fix_crate_fields, fix_schema, fix_schema_carrying,
    fix_schema_tags, from_fix_document, into_fix_document, is_crate_tag,
};
pub use int256::{i256, u256};
pub use iobase::{ArrowWriteSession, overwrite_arrow_reader_default};
pub use iobase::{
    DEFAULT_FETCH_BYTE_SIZE, DEFAULT_STREAM_BATCH_SIZE, IOBase, Reader, Writer, not_empty,
    skip_absent,
};
pub use iocursor::{Cursor, IOCursor};
pub use iofile::IOFile;
pub use iofolder::IOFolder;
pub use iokind::IOKind;
pub use iomedia::IOMedia;
pub use iomode::IOMode;
pub use iopath::IOPath;
pub use listing::Listing;
pub use media_type::MediaType;
pub use metadata::{Metadata, MetadataIntoIter, MetadataIter, PropertyIter, ProtocolMetadata};
pub use mime_type::MimeType;
pub use protocol::{
    ArrowPropertyField, ArrowPropertyFieldMut, AzField, AzFieldMut, DigestField, DigestFieldMut,
    FieldPropertiesField, FieldPropertiesFieldMut, FileField, FileFieldMut, FixField, FixFieldMut,
    GlueField, GlueFieldMut, GsField, GsFieldMut, HttpField, HttpFieldMut, IcebergField,
    IcebergFieldMut, IdentityField, IdentityFieldMut, MysqlField, MysqlFieldMut, PandasField,
    PandasFieldMut, PartitionField, PartitionFieldMut, PolarsField, PolarsFieldMut, PostgresField,
    PostgresFieldMut, PostgresqlField, PostgresqlFieldMut, ProtocolField, ProtocolFieldMut,
    PythonField, PythonFieldMut, PythonKind, PythonMetadata, S3Field, S3FieldMut, SparkField,
    SparkFieldMut, SqlField, SqlFieldMut, TransformField, TransformFieldMut, UrnField, UrnFieldMut,
};
pub use scheme::Scheme;
pub use text::{Format, Limits, ScalarIter};
pub use time_unit::TimeUnit;
pub use union_mode::UnionMode;
pub use uri::{
    Authority, Extensions, Parameters, Parents, PathSegments, Uri, UriParents, UriPath, Url,
    UrlParents, Urn,
};
pub use xxhash::{DigestFieldNames, DigestFields};

pub(crate) use arithmetic::Arithmetic;
pub(crate) use ascii::{ascii_bytes, ascii_text, ascii_text_sized};
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
pub use datatype::{DataType, VariantType};
pub(crate) use datatype::{invalid, validate_non_negative};
pub use date::*;
pub use datetime::*;
pub use decimal::*;
pub(crate) use default::{
    default_value_for_field, preflight_schema, preflight_schema_shape, value_is_logically_null,
};
pub(crate) use diff::push_field_name_path;
pub use diff::{Differences, OwnedDifferences};
pub use duration::*;
pub use enumeration::Vocabulary;
pub use enums::*;
pub use family::{
    Children, DataTypeValue, DictionaryOptions, FieldSidecar, FieldValue, GeographyType,
    GeometryType, NestedValue, RunEndType, UnionType,
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
pub use interval::*;
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
pub(crate) use string::trim_padding;
pub use string::*;
pub use structure::*;
pub use temporal::*;
pub use time::*;
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

#[cfg(test)]
mod tests {
    use super::{
        DataType, Field, Fields, MediaType, Metadata, MimeType, OwnedDifferences, Scheme, Uri, Url,
        Urn,
    };

    fn assert_send_sync<T: Send + Sync>() {}

    #[test]
    fn core_schema_values_are_send_and_sync() {
        assert_send_sync::<DataType>();
        assert_send_sync::<Field>();
        assert_send_sync::<Fields>();
        assert_send_sync::<Metadata>();
        assert_send_sync::<MimeType>();
        assert_send_sync::<MediaType>();
        assert_send_sync::<OwnedDifferences>();
        assert_send_sync::<Scheme>();
        assert_send_sync::<Uri>();
        assert_send_sync::<Url>();
        assert_send_sync::<Urn>();
    }
}
