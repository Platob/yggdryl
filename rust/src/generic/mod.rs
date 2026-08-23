//! The enums that generalize over every implementation of one contract.
//!
//! A trait says what an implementation must do; the enum here says which
//! implementations exist. Holding one of these means holding "some handle" or
//! "some media" as a concrete value - no trait object, no generic parameter -
//! which is what lets a location, a listing, or a binding pass an
//! implementation around without knowing which one it is.
//!
//! - [`Holder`] names every [`crate::io::IOBase`] implementation.
//! - [`Codec`] names every transparent content coding applied to a handle.
//!
//! It also owns [`Value`], the one native value the whole project speaks: every
//! codec parses into it, every field validates it, and every binding converts
//! its own objects to it. Its scalar behavior is split by what it describes -
//! `value` for the shape and the ordering, `decimal` and `temporal` for the
//! kinds that carry a scale or a unit, `inference` for the datatype a value
//! already names, and `typed` for one value paired with the datatype it
//! belongs to.
//! - [`Media`] names every record encoding bound to a handle.
//! - [`RecordOptions`] names every encoding's read and write settings.
//! - [`wkb`] reads Well-Known Binary geometries: their bounds, their type
//!   codes, and their WKT spelling.
//!
//! Each one delegates the whole contract to the variant it holds, so code
//! written against the enum behaves exactly as code written against the
//! implementation would.

mod arithmetic;
mod codec;
pub(crate) mod decimal;
mod holder;
mod i256;
mod inference;
pub(crate) mod iso;
#[cfg(feature = "arrow")]
mod media;
#[cfg(feature = "arrow")]
mod options;
mod pairs;
mod temporal;
mod text;
mod typed;
pub mod value;
pub mod wkb;

pub(crate) use arithmetic::Arithmetic;
pub use codec::Codec;
pub use holder::Holder;
pub use i256::I256;
#[cfg(feature = "arrow")]
pub use media::Media;
#[cfg(feature = "arrow")]
pub(crate) use options::{CommitBuffer, WriteLimitState};
#[cfg(feature = "arrow")]
pub use options::{DEFAULT_RECORD_BATCH_SIZE, IORecordOptions, RecordOptions};
pub(crate) use pairs::sorted_pairs;
#[cfg(feature = "iceberg")]
pub(crate) use pairs::sorted_values;
pub use text::Text;
pub use typed::{
    BinaryValue, BinaryViewValue, BooleanValue, Date32Value, Date64Value, Decimal32Value,
    Decimal64Value, Decimal128Value, Decimal256Value, DictionaryValue, Duration32Value,
    Duration64Value, FixedSizeBinaryValue, FixedSizeListValue, Float16Value, Float32Value,
    Float64Value, GeographyValue, GeometryValue, Int8Value, Int16Value, Int32Value, Int64Value,
    IntervalValue, LargeBinaryValue, LargeListValue, LargeListViewValue, LargeUtf8Value, ListValue,
    ListViewValue, MapValue, NullValue, RunEndEncodedValue, StructValue, Time32Value, Time64Value,
    TimestampValue, TypedValue, UInt8Value, UInt16Value, UInt32Value, UInt64Value, UnionValue,
    Utf8Value, Utf8ViewValue, VariantValue,
};
pub use value::{Children, Float16, Float32, Float64, Value};
