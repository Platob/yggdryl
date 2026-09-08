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
mod codec;
pub mod coding;
mod datatype_id;
mod datatype_kind;
mod digest;
mod edge_algorithm;
mod error;
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
pub use codec::{Codec, Encoder, Level, RestartScan, Restarts};
pub use datatype_id::DataTypeId;
pub use datatype_kind::DataTypeKind;
pub use digest::{Digest, DigestAlgorithm, DigestBytes, Digester};
pub use edge_algorithm::EdgeAlgorithm;
pub use error::{Error, Result};
pub use expression::Expression;
pub use fix::{
    CRATE_BRANCH, DEFAULT_NULL_VALUES, DEFAULT_PARTITION_SECONDS, ERROR_TAG, FixAnomalies,
    FixAnomaly, FixBranch, FixCode, FixCodeValue, FixCodec, FixCodes, FixDedup, FixEntry,
    FixFieldIter, FixId, FixKey, FixLift, FixLineage, FixLineageEntry, FixMsg, FixParty,
    FixPedigree, FixRegistry, FixSpellings, MBEAN_TAG, MSGCTXID_TAG, MSGDIRECTION_TAG, MSGHASH_TAG,
    OPERATION_TAG, PARENTCLORDID_TAG, PARENTORDERID_TAG, SENDERPLUGINID_TAG, SESSIONID_TAG,
    SESSIONINTERFACES_TAG, STANDARD_HEADER_TAGS, STANDARD_TRAILER_TAGS, STATUS_TAG,
    SYMBOLTICKER_TAG, TARGETPLUGINID_TAG, TIMESTAMP_NAME, TIMESTAMP_TAG, ULBRIDGE_BRANCH,
    ULBRIDGE_ROWHEADER, ULBRIDGE_TAG_MIN, UNIXPARTITION_TAG, UlPlugin, UlPlugins, VERSION_TAG,
    Words, fix_column_of, fix_column_tags, fix_crate_fields, fix_lift, fix_lifts, fix_schema,
    fix_schema_carrying, fix_schema_tags, fix_ulbridge_fields,
};
#[cfg(feature = "arrow")]
pub use fix::{
    DEFAULT_BATCH_BYTE_SIZE, DEFAULT_PAYLOAD_COLUMN, FixBatchReader, FixOptions, SOH,
    classify_arrow_array, write_fix,
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
    S3Field, S3FieldMut, SparkField, SparkFieldMut, SqlField, SqlFieldMut, UrnField, UrnFieldMut,
};
pub use types::{
    AnyType, AsciiValue, BytesValue, Children, DecimalValue, Differences, Enum, Field, FieldRef,
    FieldType, Floating, FloatingValue, GeospatialValue, Integer, IntegerValue, NestedValue,
    OwnedDifferences, PartitionFieldNames, PartitionFields, Pretty, Scalar, ScalarFamily,
    ScalarValue, TemporalFamily, TemporalValue, TextValue, TypedField, TypedFieldRef, TypedScalar,
};
pub use types::{
    AsciiEnum, AsciiType, BytesType, DataType, DecimalType, DictionaryType, Fields, FloatingType,
    GeospatialParameters, GeospatialType, IntegerType, MapType, NestedType, RunEndEncodedType,
    TemporalType, TextType, UnionFields, Version, VersionField, VersionScalar, VersionType,
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
