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
// Foreign Arrow filesystems behind the one storage trait. The vtable adds no
// dependency, so the module is unconditional like the Avro value codec; the
// record surface its handles answer is inherited and already gated by `arrow`.
pub mod arrowfs;
pub mod avro;
// The page cache over any handle. It is a wrapping handle like the codings,
// so it lives beside them rather than inside `io`, and it needs no Arrow.
pub mod buffered;
mod bytestream;
mod codec;
mod datatype;
mod datatype_id;
mod datatype_kind;
mod digest;
mod edge_algorithm;
mod error;
pub mod expression;
pub mod field;
pub mod fix;
pub mod generic;
pub mod gzip;
mod i256;
#[cfg(feature = "iceberg")]
pub mod iceberg;
// The line handler consults the Iceberg type vocabulary even when the table
// format itself is not compiled in, so the schema it accepts never depends on
// the feature set - a schema answered from a pattern is the same schema in a
// schema-only build as in a full one. Without the feature the module is
// exactly the vocabulary - the one self-contained `types.rs`, never a
// duplicate, and it needs no dependency of its own - and enabling the feature
// only *adds* the rest of the format, so the features stay additive.
#[cfg(not(feature = "iceberg"))]
#[path = "iceberg/types.rs"]
pub mod iceberg;
pub mod io;
mod iobase;
mod iocursor;
mod iofile;
mod iofolder;
mod iokind;
mod iomedia;
mod iomode;
mod iopath;
#[cfg(feature = "arrow")]
pub mod ipc;
pub mod json;
mod listing;
pub mod local;
mod media_type;
mod metadata;
mod mime_type;
#[cfg(feature = "parquet")]
pub mod parquet;
mod path;
mod scheme;
pub mod text;
mod time_unit;
mod timezone;
pub mod toml;
mod union_mode;
mod uri;
// The digest vocabulary's implementation. The value codec has no Arrow
// dependency, so the module is unconditional like the Avro value codec; only
// `xxhash/arrow.rs` is gated.
pub mod xxhash;
pub mod yaml;
pub mod zlib;
pub mod zstd;

pub use bytestream::ByteStream;
pub use codec::{Codec, Encoder, Level};
pub use datatype::{
    AsciiEnum, DataType, DictionaryType, Fields, GeospatialType, MapType, RunEndEncodedType,
    UnionFields,
};
pub use datatype_id::DataTypeId;
pub use datatype_kind::DataTypeKind;
pub use digest::{Digest, DigestAlgorithm, DigestBytes, Digester};
pub use edge_algorithm::EdgeAlgorithm;
pub use error::{Error, Result};
pub use expression::Expression;
#[cfg(feature = "arrow")]
pub use field::cast::{ArrowCast, ArrowFieldType};
pub use field::protocol::{
    ArrowPropertyField, ArrowPropertyFieldMut, AzField, AzFieldMut, FieldPropertiesField,
    FieldPropertiesFieldMut, FileField, FileFieldMut, FixField, FixFieldMut, GlueField,
    GlueFieldMut, GsField, GsFieldMut, HttpField, HttpFieldMut, IcebergField, IcebergFieldMut,
    MysqlField, MysqlFieldMut, PandasField, PandasFieldMut, PolarsField, PolarsFieldMut,
    PostgresField, PostgresFieldMut, PostgresqlField, PostgresqlFieldMut, ProtocolField,
    ProtocolFieldMut, S3Field, S3FieldMut, SparkField, SparkFieldMut, SqlField, SqlFieldMut,
    UrnField, UrnFieldMut,
};
pub use field::{
    AnyType, Differences, Field, FieldRef, FieldType, OwnedDifferences, PartitionFieldNames,
    PartitionFields, Pretty, TypedField, TypedFieldRef,
};
pub use fix::{FixAliases, FixBranch, FixFieldIter, FixId, FixKey, FixMsg, FixRegistry};
pub use generic::{EnumScalar, Float, Integer, Scalar, TemporalFamily, TemporalRef};
pub use i256::I256;
#[cfg(feature = "arrow")]
pub use iobase::{ArrowWriteSession, overwrite_arrow_reader_default};
pub use iobase::{DEFAULT_STREAM_BATCH_SIZE, IOBase, Reader, Writer, not_empty, skip_absent};
pub use iocursor::{Cursor, IOCursor};
pub use iofile::IOFile;
pub use iofolder::IOFolder;
pub use iokind::IOKind;
pub use iomedia::IOMedia;
pub use iomode::IOMode;
pub use iopath::IOPath;
pub use json::{from_json_scalar, from_json_scalar_with_field, into_json_scalar};
pub use listing::Listing;
pub use media_type::MediaType;
pub use metadata::{Metadata, MetadataIntoIter, MetadataIter, PropertyIter, ProtocolMetadata};
pub use mime_type::MimeType;
pub use scheme::Scheme;
pub use text::{Children, Float16, Float32, Float64, Format, Limits, ScalarIter, TypedScalar};
pub(crate) use text::{stable_hash_display, stable_hash_of};
pub use time_unit::TimeUnit;
pub use timezone::Timezone;
pub use toml::{from_toml_scalar, from_toml_scalar_with_field, into_toml_scalar};
pub use union_mode::UnionMode;
pub use uri::{
    Authority, Extensions, Parents, PathSegments, Uri, UriParents, UriPath, Url, UrlParents, Urn,
};
pub use yaml::{from_yaml_scalar, from_yaml_scalar_with_field, into_yaml_scalar};

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
