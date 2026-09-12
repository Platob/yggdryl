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

#[cfg(feature = "arrow")]
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
pub mod expression;
pub mod fix;
pub mod holder;
mod i256;
mod iobase;
mod iocursor;
mod iofile;
mod iofolder;
mod iokind;
mod iomedia;
mod iomode;
mod iopath;
mod listing;
pub mod media;
mod media_type;
mod metadata;
mod mime_type;
mod path;
mod scheme;
pub mod text;
mod time_unit;
mod timezone;
// An instant coupled with a digest. The value and its protocol vocabulary
// need no Arrow, like the digest vocabulary they extend; only
// `txhash/arrow.rs` is gated.
pub mod txhash;
pub mod types;
mod union_mode;
mod uri;
// The digest vocabulary's implementation. The value codec has no Arrow
// dependency, so the module is unconditional like the Avro value codec; only
// `xxhash/arrow.rs` is gated.
pub mod xxhash;

#[cfg(feature = "arrow")]
pub use arrow::{ArrowShape, ArrowValue};
pub use bytestream::ByteStream;
pub use charset::Charset;
pub use codec::{Codec, Encoder, Level, RestartScan, Restarts};
pub use datatype_id::DataTypeId;
pub use datatype_kind::DataTypeKind;
pub use digest::{Digest, DigestAlgorithm, DigestBytes, Digester};
pub use edge_algorithm::EdgeAlgorithm;
pub use error::{Error, Result};
pub use expression::Expression;
pub use expression::{FieldPath, FieldSegment};
pub use fix::MsgType;
pub use fix::{
    CRATE_TAG_MAX, CRATE_TAG_MIN, DEFAULT_NULL_VALUES, DEFAULT_PARTITION_SECONDS,
    DEFAULT_PAYLOAD_COLUMN, ERROR_TAG_NAME, FixAnomalies, FixAnomaly, FixCode, FixCodeValue,
    FixCodec, FixCodes, FixDedup, FixDirection, FixDirectionEntry, FixDirections, FixEntry,
    FixFieldIter, FixId, FixKey, FixLifecycle, FixLift, FixLineage, FixLineageEntry, FixMessages,
    FixMsg, FixParty, FixPatterns, FixPedigree, FixRegistry, FixSpellings, ID_TAG_NAME,
    INSTID_TAG_NAME, ISINCODE_TAG_NAME, MBEAN_TAG_NAME, MICCODE_TAG_NAME, MSGCTXID_TAG_NAME,
    MSGDIRECTION_TAG_NAME, MSGHASH_TAG_NAME, OPERATION_TAG_NAME, PARENTCLORDID_TAG_NAME,
    PARENTORDERID_TAG_NAME, PERSISTENTID_TAG_NAME, PLUGINID_TAG_NAME, PREVPLUGINID_TAG_NAME,
    SENDERSESSIONID_TAG_NAME, SENDERSESSIONNAME_TAG_NAME, SOH, STANDARD_HEADER_TAGS,
    STANDARD_TRAILER_TAGS, STATE_TAG_NAME, STATUS_TAG_NAME, SYMBOLTICKER_TAG_NAME,
    TARGETSESSIONID_TAG_NAME, TARGETSESSIONNAME_TAG_NAME, TIMESTAMP_TAG_NAME, ULBRIDGE_DIALECT,
    ULBRIDGE_ROWHEADER, ULBRIDGE_TAG_MIN, UNIXPARTITION_TAG_NAME, UlPlugin, UlPlugins,
    VERSION_TAG_NAME, Words, fix_column_of, fix_column_tags, fix_crate_fields, fix_lift, fix_lifts,
    fix_schema, fix_schema_carrying, fix_schema_tags, fix_ulbridge_fields, is_crate_tag,
};
pub use i256::I256;
#[cfg(feature = "arrow")]
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
pub use scheme::Scheme;
pub use text::json::{from_json_scalar, from_json_scalar_with_field, into_json_scalar};
pub use text::toml::{from_toml_scalar, from_toml_scalar_with_field, into_toml_scalar};
pub use text::yaml::{from_yaml_scalar, from_yaml_scalar_with_field, into_yaml_scalar};
pub use text::{Format, Limits, ScalarIter, Structured};
pub(crate) use text::{stable_hash_display, stable_hash_of};
pub use time_unit::TimeUnit;
pub use timezone::Timezone;
#[cfg(feature = "arrow")]
pub use types::cast::{
    ArrowCast, ArrowCastOptions, ArrowCastPlan, ArrowFieldType, Nullability, Representation,
};
pub use types::floating::scalars::{Float16, Float32, Float64};
pub use types::protocol::{
    ArrowPropertyField, ArrowPropertyFieldMut, AzField, AzFieldMut, DigestField, DigestFieldMut,
    FieldPropertiesField, FieldPropertiesFieldMut, FileField, FileFieldMut, FixField, FixFieldMut,
    GlueField, GlueFieldMut, GsField, GsFieldMut, HttpField, HttpFieldMut, IcebergField,
    IcebergFieldMut, IdentityField, IdentityFieldMut, MysqlField, MysqlFieldMut, PandasField,
    PandasFieldMut, PartitionField, PartitionFieldMut, PolarsField, PolarsFieldMut, PostgresField,
    PostgresFieldMut, PostgresqlField, PostgresqlFieldMut, ProtocolField, ProtocolFieldMut,
    PythonField, PythonFieldMut, PythonKind, PythonMetadata, S3Field, S3FieldMut, SparkField,
    SparkFieldMut, SqlField, SqlFieldMut, UrnField, UrnFieldMut,
};
pub use types::{
    Bytes, Children, Code, CodeValue, DecimalValue, Differences, Enum, Field, FieldRecord,
    FieldRef, FieldScalar, FieldType, Floating, FloatingValue, GeospatialValue, Integer,
    IntegerValue, NestedValue, OwnedDifferences, PartitionFieldNames, PartitionFields, Pretty,
    Scalar, ScalarFamily, ScalarValue, Str, TemporalFamily, TemporalValue, TypedField,
    TypedFieldRef,
};
pub use types::{
    BytesType, DataType, DecimalType, DictionaryType, Fields, FloatingType, GeospatialParameters,
    GeospatialType, IntegerType, MapType, NestedType, RunEndEncodedType, StringEnum, StringType,
    TemporalType, UnionFields, UrlField, UrlType, Version, VersionField, VersionType,
};
pub use union_mode::UnionMode;
pub use uri::{
    Authority, Extensions, Parameters, Parents, PathSegments, Uri, UriParents, UriPath, Url,
    UrlParents, Urn,
};
pub use xxhash::{DigestFieldNames, DigestFields};

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
