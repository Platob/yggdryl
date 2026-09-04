//! Shared scalar, dispatch, I/O vocabulary, and runtime wrappers.

mod arithmetic;
pub(crate) mod decimal;
mod enum_scalar;
pub(crate) mod iso;
mod pairs;
pub mod scalar;
pub(crate) mod temporal;
mod text;
mod typed;
pub mod wkb;

pub(crate) use arithmetic::Arithmetic;
pub use enum_scalar::EnumScalar;
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
