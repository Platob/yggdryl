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
pub mod graph;
pub mod hashing;
pub mod holder;
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
pub mod types;
mod union_mode;
mod uri;

#[cfg(feature = "arrow")]
pub use arrow::{ArrowScalar, ArrowShape};
pub use bytestream::ByteStream;
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
    ASKCURRENCY_TAG_NAME, ASKUNIT_TAG_NAME, BIDCURRENCY_TAG_NAME, BIDUNIT_TAG_NAME,
    BLOOMBERGCODE_TAG_NAME, CRATE_TAG_MAX, CRATE_TAG_MIN, CREATUNIX_TAG_NAME, CROSSCODE_TAG_NAME,
    CROSSHASHCODE_TAG_NAME, CROSSUUID_TAG_NAME, CURRUUID_TAG_NAME, CUSIPCODE_TAG_NAME,
    DEFAULT_NULL_VALUES, DEFAULT_PAYLOAD_COLUMN, DEFAULT_REFUSED_MSGTYPES, EXPIRUNIX_TAG_NAME,
    FIXMSG_TAG_NAME, FixCapture, FixCode, FixCodeValue, FixCodec, FixCodes, FixDedup, FixDirection,
    FixDirectionEntry, FixDirections, FixEntry, FixFieldIter, FixHeader, FixId, FixKey,
    FixMessages, FixMsg, FixPatterns, FixRegistry, FixSpellings, HASHCODE_TAG_NAME,
    IDENTIFIERS_TAG_NAME, ISINCODE_TAG_NAME, METADATA_TAG_NAME, MICCODE_TAG_NAME,
    MSGCTXID_TAG_NAME, MSGDIRECTION_TAG_NAME, MSGSESSIONID_TAG_NAME, NOFIXENTRIES_TAG_NAME,
    PARENTUUIDS_TAG_NAME, PLUGINID_TAG_NAME, PREVPX_TAG_NAME, PREVQTY_TAG_NAME, PREVUNIX_TAG_NAME,
    PREVUUID_TAG_NAME, PX_TAG_NAME, QTY_TAG_NAME, RECORDEDAT_TAG_NAME, SEDOLCODE_TAG_NAME,
    SEQNUM_TAG_NAME, SNAPUNIX_TAG_NAME, SOH, SOURCEURL_TAG_NAME, STANDARD_HEADER_TAGS,
    STANDARD_TRAILER_TAGS, STATE_TAG_NAME, SYMBOLTICKER_TAG_NAME, TRADABLE_TAG_NAME,
    ULBRIDGE_ROWHEADER, UNIT_TAG_NAME, UNIX_TAG_NAME, Words, fix_column_of, fix_column_tags,
    fix_crate_fields, fix_schema, fix_schema_carrying, fix_schema_tags, from_fix_document,
    into_fix_document, is_crate_tag,
};
pub use hashing::xxhash::{DigestFieldNames, DigestFields};
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
pub use text::{Format, Limits, ScalarIter};
pub use time_unit::TimeUnit;
#[cfg(feature = "arrow")]
pub use types::cast::{
    ArrowCast, ArrowCastOptions, ArrowCastPlan, ArrowFieldType, Nullability, Representation,
};
pub use types::floating::{Float16, Float32, Float64};
pub use types::i256::{i256, u256};
pub use types::protocol::{
    ArrowPropertyField, ArrowPropertyFieldMut, AzField, AzFieldMut, DigestField, DigestFieldMut,
    FieldPropertiesField, FieldPropertiesFieldMut, FileField, FileFieldMut, FixField, FixFieldMut,
    GlueField, GlueFieldMut, GsField, GsFieldMut, HttpField, HttpFieldMut, IcebergField,
    IcebergFieldMut, IdentityField, IdentityFieldMut, MysqlField, MysqlFieldMut, PandasField,
    PandasFieldMut, PartitionField, PartitionFieldMut, PolarsField, PolarsFieldMut, PostgresField,
    PostgresFieldMut, PostgresqlField, PostgresqlFieldMut, ProtocolField, ProtocolFieldMut,
    PythonField, PythonFieldMut, PythonKind, PythonMetadata, S3Field, S3FieldMut, SparkField,
    SparkFieldMut, SqlField, SqlFieldMut, TransformField, TransformFieldMut, UrnField, UrnFieldMut,
};
pub use types::timezone::Timezone;
pub use types::{
    Bytes, Children, CodeValue, Decimal, DecimalValue, Differences, Field, FieldRecord, FieldRef,
    FieldScalar, FieldType, FloatingValue, GeospatialValue, IntegerValue, NestedValue,
    OwnedDifferences, PartitionFieldNames, PartitionFields, Pretty, Scalar, Str, TemporalFamily,
    TemporalValue, TypedField, TypedFieldRef, Value, Vocabulary,
};
pub use types::{
    BytesType, DataType, DecimalType, DictionaryType, Fields, FloatingType, GeospatialParameters,
    GeospatialType, IntegerType, MapField, MapType, MappingField, MappingType, MediaTypeField,
    MediaTypeType, MimeTypeField, MimeTypeType, RunEndEncodedType, SortedMapField,
    StringEnum, StringType, TemporalType, TimezoneField, TimezoneType, UnionFields, UrlField,
    StructType, StructureType, Struct2Type, UrlType, Version, VersionField,
    VersionType,
};
pub use union_mode::UnionMode;
pub use uri::{
    Authority, Extensions, Parameters, Parents, PathSegments, Uri, UriParents, UriPath, Url,
    UrlParents, Urn,
};

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
