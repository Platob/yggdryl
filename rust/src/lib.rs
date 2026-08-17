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
mod datatype;
pub mod enums;
mod error;
pub mod field;
pub mod generic;
pub mod gzip;
#[cfg(feature = "iceberg")]
pub mod iceberg;
pub mod io;
#[cfg(feature = "arrow")]
pub mod ipc;
pub mod json;
pub mod local;
mod metadata;
#[cfg(feature = "parquet")]
pub mod parquet;
mod path;
pub mod text;
pub mod toml;
mod uri;
pub mod yaml;
pub mod zlib;
pub mod zstd;

pub use datatype::{DataType, DictionaryType, Fields, MapType, RunEndEncodedType, UnionFields};
pub use enums::{
    Codec, DataTypeId, DataTypeKind, Encoder, IOKind, Level, MediaType, MimeType, Scheme, TimeUnit,
    Timezone, UnionMode,
};
pub use error::{Error, Result};
#[cfg(feature = "arrow")]
pub use field::cast::{ArrowCast, ArrowFieldType};
pub use field::{
    AnyType, Differences, Field, FieldRef, FieldType, OwnedDifferences, PartitionFieldNames,
    PartitionFields, TypedField, TypedFieldRef,
};
pub use metadata::{
    Metadata, MetadataIntoIter, MetadataIter, PropertyIter, ProtocolMetadata, ProtocolMetadataMut,
};
pub(crate) use text::stable_hash_display;
pub use text::{Children, Float, Float32, Format, Limits, TypedValue, Value, ValueIter};
pub use uri::{
    Authority, Extensions, Parents, PathSegments, Uri, UriParents, UriPath, Url, UrlParents, Urn,
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
