//! Shared scalar, dispatch, I/O vocabulary, and runtime wrappers.

mod arithmetic;
mod coded;
pub(crate) mod decimal;
mod enum_scalar;
mod holder;
mod inference;
pub(crate) mod iso;
mod magic;
#[cfg(feature = "arrow")]
mod media;
#[cfg(feature = "arrow")]
mod options;
mod pairs;
pub mod scalar;
pub(crate) mod temporal;
mod text;
mod typed;
pub mod wkb;

pub(crate) use arithmetic::Arithmetic;
pub use coded::Coded;
pub use enum_scalar::EnumScalar;
pub use holder::Holder;
pub use magic::MAGIC_PROBE_LEN;
#[cfg(feature = "arrow")]
pub use media::Media;
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
pub use temporal::{TemporalFamily, TemporalRef};
pub use text::Text;
pub use typed::{
    AsciiScalar, BinaryScalar, BinaryViewScalar, BooleanScalar, CfiScalar, CountryScalar,
    CurrencyScalar, Date32Scalar, Date64Scalar, Decimal32Scalar, Decimal64Scalar, Decimal128Scalar,
    Decimal256Scalar, DictionaryScalar, Duration32Scalar, Duration64Scalar, FixedAsciiScalar,
    FixedSizeBinaryScalar, FixedSizeListScalar, Float16Scalar, Float32Scalar, Float64Scalar,
    GeographyScalar, GeometryScalar, GuidScalar, Int8Scalar, Int16Scalar, Int32Scalar, Int64Scalar,
    IntervalScalar, LargeBinaryScalar, LargeListScalar, LargeListViewScalar, LargeUtf8Scalar,
    ListScalar, ListViewScalar, MapScalar, MicScalar, NullScalar, RunEndEncodedScalar,
    StructScalar, Time32Scalar, Time64Scalar, TimestampScalar, TypedScalar, UInt8Scalar,
    UInt16Scalar, UInt32Scalar, UInt64Scalar, UnionScalar, Utf8Scalar, Utf8ViewScalar,
    VariantScalar,
};
