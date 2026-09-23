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
pub mod bloomberg_code;
pub mod boolean;
pub(crate) mod budget;
pub mod bytes;
pub mod cast;
pub mod cfi_code;
pub mod code;
mod compatibility;
pub mod country;
pub mod cp1252;
pub mod currency;
pub mod cusip_code;
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
mod field;
pub mod figi_code;
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
pub mod isin_code;
pub mod json;
mod listing;
pub mod local;
pub mod mapping;
pub mod media;
mod media_type;
mod merge;
mod metadata;
pub mod mic_code;
mod mime_type;
mod parallel;
#[cfg(feature = "parquet")]
pub mod parquet;
mod parser;
mod path;
mod pretty;
pub mod protocol;
mod regex;
pub mod runend;
#[cfg(feature = "s3")]
pub mod s3;
mod scalar;
mod scheme;
pub mod sedol_code;
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
pub mod uri;
pub mod utf8;
pub mod uuid;
mod value;
mod valuestream;
mod variant;
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
    BLOOMBERGCODE_TAG_NAME, CRATE_TAG_MAX, CRATE_TAG_MIN, CREAUNIX_TAG_NAME, CROSSCODE_TAG_NAME,
    CROSSHASHCODE_TAG_NAME, CROSSUUID_TAG_NAME, CURRHASHCODE_TAG_NAME, CURRUNIX_TAG_NAME,
    CURRUUID_TAG_NAME, CUSIPCODE_TAG_NAME, DEFAULT_NULL_VALUES, DEFAULT_PAYLOAD_COLUMN,
    DEFAULT_REFUSED_MSGTYPES, EXECUNIX_TAG_NAME, EXPRTIME_TAG_NAME, FIGICODE_TAG_NAME,
    FIX_TYPED_TAGS, FIXMSG_TAG_NAME, FixCapture, FixCode, FixCodeSet, FixCodeValue, FixCodec,
    FixCodes, FixDedup, FixDirection, FixDirectionEntry, FixDirections, FixEntry, FixFieldIter,
    FixHeader, FixId, FixKey, FixLifted, FixMessages, FixMsg, FixPatterns, FixRegistry,
    FixSpellings, IDENTIFIERS_TAG_NAME, ISINCODE_TAG_NAME, METADATA_TAG_NAME, MICCODE_TAG_NAME,
    MSGCAT_TAG_NAME, MSGCTXID_TAG_NAME, MSGDIRECTION_TAG_NAME, MSGPLUGINID_TAG_NAME,
    MSGSESSEVENTID_TAG_NAME, MSGSESSIONID_TAG_NAME, NOFIXENTRIES_TAG_NAME, PREVUNIX_TAG_NAME,
    PREVUUID_TAG_NAME, RECDUNIX_TAG_NAME, SEDOLCODE_TAG_NAME, SEQNUM_TAG_NAME, SNAPUNIX_TAG_NAME,
    SOH, SOURCEURL_TAG_NAME, SRCUUIDS_TAG_NAME, STANDARD_HEADER_TAGS, STANDARD_TRAILER_TAGS,
    STATE_TAG_NAME, ULBRIDGE_ROWHEADER, Words, fix_column_of, fix_column_tags, fix_crate_fields,
    fix_schema, fix_schema_carrying, fix_schema_tags, from_fix_document, into_fix_document,
    is_crate_tag,
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
    Arn, Authority, Extensions, Parameters, Parents, PathSegments, Uri, UriParents, UriPath,
    UriType, Url, UrlParents, Urn,
};
pub(crate) use uri::{URL_EXTENSION_NAME, URN_EXTENSION_NAME};
pub use xxhash::{DigestFieldNames, DigestFields};

pub(crate) use arithmetic::Arithmetic;
pub(crate) use ascii::{ascii_bytes, ascii_text, ascii_text_sized};
pub use bloomberg_code::*;
pub use boolean::*;
pub use bytes::*;
pub use cfi_code::*;
pub use code::*;
pub(crate) use code::{code_cell_text, code_for_extension};
pub(crate) use code::{code_refusal, code_text};
pub use country::*;
pub use currency::*;
pub use cusip_code::*;
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
pub use field::*;
pub use figi_code::*;
pub use floating::*;
#[cfg(feature = "parquet")]
pub(crate) use geospatial::DEFAULT_CRS;
pub(crate) use geospatial::GEOARROW_WKB_EXTENSION_NAME;
pub use geospatial::*;
pub use integer::*;
pub use interval::*;
pub use isin_code::*;
pub use mapping::*;
pub(crate) use media_type::MEDIATYPE_EXTENSION_NAME;
pub use media_type::MediaTypeType;
pub(crate) use merge::Recode;
pub use merge::Widening;
pub use mic_code::*;
pub(crate) use mime_type::MIMETYPE_EXTENSION_NAME;
pub use mime_type::MimeTypeType;
pub(crate) use parser::{folds_equal, normalized};
pub use pretty::Pretty;
pub use runend::*;
pub use scalar::Scalar;
pub(crate) use scalar::code_scalars;
pub use sedol_code::*;
pub use sequence::*;
pub use side::*;
pub use state::*;
pub(crate) use string::trim_padding;
pub use string::*;
pub use structure::*;
pub(crate) use temporal::TemporalKind;
pub use temporal::*;
pub use time::*;
pub use timeinforce::*;
pub(crate) use timezone::TIMEZONE_EXTENSION_NAME;
pub use timezone::{Timezone, TimezoneType};
pub use typed::{FieldRecord, FieldScalar, UncheckedFieldScalar};
pub use union::*;
pub use uuid::*;
pub(crate) use uuid::{
    UUID_EXTENSION_NAME, UUID_TEXT_LEN, uuid_bytes, uuid_parse, uuid_rendered, uuid_text,
};
pub(crate) use value::dtype_scalar;
pub use value::{
    Children, CodeValue, DataTypeValue, DecimalValue, DictionaryOptions, FamilyValue, FieldSidecar,
    FieldValue, FloatingValue, GeographyType, GeometryType, GeospatialValue, IntegerValue, Nested,
    NestedValue, RunEndType, TemporalValue, UnionType, Value,
};
pub use valuestream::{COMPRESS_FROM, VALUE_STREAM_VERSION, ValueStream};
pub use variant::{
    VARIANT_EXTENSION_NAME, VARIANT_METADATA_FIELD, VARIANT_VALUE_FIELD, VARIANT_VERSION, Variant,
};
pub(crate) use variant::{is_variant_storage, variant_fields};
pub(crate) use version::VERSION_EXTENSION_NAME;
pub use version::*;

// GENERATED by scripts/generate_internals.py - do not edit by hand.
/// What the test suite pins and a caller cannot reach.
///
/// Every test lives in `rust/tests/` and reaches the crate through
/// `yggdryl::`. The handful that pin something no caller can name reach it
/// here instead, under a feature no published build turns on. This is not
/// API: it carries no stability promise, and an item is `pub` only because
/// its own module is unreachable without the feature.
#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    pub use crate::arithmetic::internals as arithmetic;
    pub use crate::arrow::rows::internals as arrow_rows;
    pub use crate::avro::arrow::internals as avro_arrow;
    pub use crate::avro::batch::internals as avro_batch;
    pub use crate::avro::schema::internals as avro_schema;
    pub use crate::bytestream::internals as bytestream;
    pub use crate::charset::reader::internals as charset_reader;
    pub use crate::code::internals as code;
    pub use crate::decimal::internals as decimal;
    pub use crate::diff::internals as diff;
    pub use crate::error::internals as error;
    pub use crate::expression::eval::internals as expression_eval;
    pub use crate::fix::catalog::internals as fix_catalog;
    pub use crate::fix::codec::internals as fix_codec;
    pub use crate::fix::codes::internals as fix_codes;
    pub use crate::fix::component::internals as fix_component;
    pub use crate::fix::document::internals as fix_document;
    pub use crate::fix::enrich::internals as fix_enrich;
    pub use crate::fix::global::internals as fix_global;
    pub use crate::fix::group_plan::internals as fix_group_plan;
    pub use crate::fix::identity::internals as fix_identity;
    pub use crate::fix::memo::internals as fix_memo;
    pub use crate::fix::msgtype::internals as fix_msgtype;
    pub use crate::fix::registry::internals as fix_registry;
    pub use crate::fix::replacements::internals as fix_replacements;
    pub use crate::fix::retired::internals as fix_retired;
    pub use crate::fix::schema::internals as fix_schema;
    pub use crate::fix::store::internals as fix_store;
    pub use crate::fs::local::internals as fs_local;
    pub use crate::graph::instrument::internals as graph_instrument;
    pub use crate::graph::iterator::internals as graph_iterator;
    pub use crate::hashing::stable::internals as hashing_stable;
    pub use crate::holder::buffered::internals as holder_buffered;
    #[cfg(feature = "iceberg")]
    pub use crate::iceberg::manifest::internals as iceberg_manifest;
    #[cfg(feature = "iceberg")]
    pub use crate::iceberg::metadata::internals as iceberg_metadata;
    #[cfg(feature = "iceberg")]
    pub use crate::iceberg::partition::internals as iceberg_partition;
    #[cfg(feature = "iceberg")]
    pub use crate::iceberg::scan::internals as iceberg_scan;
    #[cfg(feature = "iceberg")]
    pub use crate::iceberg::snapshot::internals as iceberg_snapshot;
    #[cfg(feature = "iceberg")]
    pub use crate::iceberg::staging::internals as iceberg_staging;
    #[cfg(feature = "iceberg")]
    pub use crate::iceberg::statistics::internals as iceberg_statistics;
    #[cfg(feature = "iceberg")]
    pub use crate::iceberg::table::internals as iceberg_table;
    #[cfg(feature = "iceberg")]
    pub use crate::iceberg::value::internals as iceberg_value;
    pub use crate::ipc::internals as ipc;
    pub use crate::local::internals as local;
    pub use crate::media::merge::internals as media_merge;
    pub use crate::media::options::internals as media_options;
    pub use crate::media::partition::internals as media_partition;
    pub use crate::merge::internals as merge;
    pub use crate::metadata::internals as metadata;
    pub use crate::mime_type::line::internals as mime_type_line;
    pub use crate::parallel::internals as parallel;
    #[cfg(feature = "parquet")]
    pub use crate::parquet::geospatial::internals as parquet_geospatial;
    #[cfg(feature = "parquet")]
    pub use crate::parquet::internals as parquet;
    pub use crate::path::internals as path;
    pub use crate::protocol::internals as protocol;
    #[cfg(feature = "s3")]
    pub use crate::s3::answer::internals as s3_answer;
    #[cfg(feature = "s3")]
    pub use crate::s3::aws::credentials::internals as s3_aws_credentials;
    #[cfg(feature = "s3")]
    pub use crate::s3::aws::profile::internals as s3_aws_profile;
    #[cfg(feature = "s3")]
    pub use crate::s3::aws::xml::internals as s3_aws_xml;
    #[cfg(feature = "s3")]
    pub use crate::s3::azure::dialect::internals as s3_azure_dialect;
    #[cfg(feature = "s3")]
    pub use crate::s3::azure::sign::internals as s3_azure_sign;
    #[cfg(feature = "s3")]
    pub use crate::s3::azure::xml::internals as s3_azure_xml;
    #[cfg(feature = "s3")]
    pub use crate::s3::client::internals as s3_client;
    #[cfg(feature = "s3")]
    pub use crate::s3::file::internals as s3_file;
    #[cfg(feature = "s3")]
    pub use crate::s3::options::internals as s3_options;
    #[cfg(feature = "s3")]
    pub use crate::s3::sigv4::internals as s3_sigv4;
    #[cfg(feature = "s3")]
    pub use crate::s3::xml::internals as s3_xml;
    pub use crate::scalar::internals as scalar;
    pub use crate::temporal::internals as temporal;
    pub use crate::text::display::internals as text_display;
    pub use crate::text::line::internals as text_line;
    pub use crate::text::position::internals as text_position;
    pub use crate::text::reader::internals as text_reader;
    pub use crate::timezone::internals as timezone;
    pub use crate::toml::wire::internals as toml_wire;
    pub use crate::txhash::arrow::internals as txhash_arrow;
    pub use crate::txhash::internals as txhash;
    pub use crate::uri::pattern::internals as uri_pattern;
    pub use crate::uri::url::internals as uri_url;
    pub use crate::utf8::internals as utf8;
    pub use crate::valuestream::internals as valuestream;
    pub use crate::variant::internals as variant;
    pub use crate::version::internals as version;
    pub use crate::xxhash::internals as xxhash;
    pub use crate::zip::archive::internals as zip_archive;
    pub use crate::zip::entry::internals as zip_entry;
    pub use crate::zip::format::internals as zip_format;
    pub use crate::zip::name::internals as zip_name;
}
// END GENERATED
