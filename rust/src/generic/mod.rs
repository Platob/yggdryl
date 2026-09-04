//! Shared scalar, dispatch, I/O vocabulary, and runtime wrappers.

mod arithmetic;
pub(crate) mod codec;
mod coded;
mod datatype_id;
mod datatype_kind;
pub(crate) mod decimal;
mod digest;
mod edge_algorithm;
mod enum_scalar;
mod holder;
mod i256;
mod inference;
mod io_kind;
mod io_mode;
pub(crate) mod iso;
mod magic;
#[cfg(feature = "arrow")]
mod media;
mod media_type;
mod mime_type;
#[cfg(feature = "arrow")]
mod options;
mod pairs;
pub mod scalar;
mod scheme;
mod temporal;
mod text;
mod time_unit;
pub(crate) mod timezone;
mod typed;
mod union_mode;
pub mod wkb;

pub(crate) use arithmetic::Arithmetic;
pub use codec::{Codec, Encoder, Level};
pub use coded::Coded;
pub use datatype_id::DataTypeId;
pub use datatype_kind::DataTypeKind;
pub use digest::{Digest, DigestAlgorithm, DigestBytes, Digester};
pub use edge_algorithm::EdgeAlgorithm;
pub use enum_scalar::EnumScalar;
pub use holder::Holder;
pub use i256::I256;
pub use io_kind::IOKind;
pub use io_mode::IOMode;
pub use magic::MAGIC_PROBE_LEN;
#[cfg(feature = "arrow")]
pub use media::Media;
pub use media_type::MediaType;
pub use mime_type::MimeType;
#[cfg(feature = "arrow")]
pub(crate) use options::{CommitBuffer, WriteLimitState};
#[cfg(feature = "arrow")]
pub use options::{DEFAULT_RECORD_BATCH_ROW_SIZE, IORecordOptions, RecordOptions};

/// The root Field name a record surface uses when none is declared.
///
/// It names an inferred root and a declared datatype alike, so a stream read
/// without a schema and one read under a declared datatype answer the same
/// root name unless the options say otherwise.
pub const DEFAULT_ROOT_NAME: &str = "row";
pub(crate) use pairs::sorted_pairs;
#[cfg(feature = "iceberg")]
pub(crate) use pairs::sorted_values;
pub use scalar::{Children, Float, Float16, Float32, Float64, Integer, Scalar};
pub use scheme::Scheme;
pub use temporal::{TemporalFamily, TemporalRef};
pub use text::Text;
pub use time_unit::TimeUnit;
pub use timezone::Timezone;
pub use typed::{
    Ascii32Scalar, Ascii64Scalar, Ascii128Scalar, BinaryScalar, BinaryViewScalar, BooleanScalar,
    Date32Scalar, Date64Scalar, Decimal32Scalar, Decimal64Scalar, Decimal128Scalar,
    Decimal256Scalar, DictionaryScalar, Duration32Scalar, Duration64Scalar, FixedSizeBinaryScalar,
    FixedSizeListScalar, Float16Scalar, Float32Scalar, Float64Scalar, GeographyScalar,
    GeometryScalar, Int8Scalar, Int16Scalar, Int32Scalar, Int64Scalar, IntervalScalar,
    LargeBinaryScalar, LargeListScalar, LargeListViewScalar, LargeUtf8Scalar, ListScalar,
    ListViewScalar, MapScalar, NullScalar, RunEndEncodedScalar, StructScalar, Time32Scalar,
    Time64Scalar, TimestampScalar, TypedScalar, UInt8Scalar, UInt16Scalar, UInt32Scalar,
    UInt64Scalar, UnionScalar, Utf8Scalar, Utf8ViewScalar, VariantScalar,
};
pub use union_mode::UnionMode;
