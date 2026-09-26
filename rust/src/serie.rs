//! Serie: many values, on the fourth side of the value model.
//!
//! One type, one file - and a folder beside it, because a column is as wide
//! as Arrow's layouts are. [`DataType`] says what shape a value has,
//! [`Field`] says whose it is and whether a row may be absent, [`Scalar`] is
//! one value, and a [`Serie`] is many of them. It is what the sequence
//! family holds: `Scalar::Serie(Serie)`, so there is one type for "many
//! values" and not two.
//!
//! | side | root | trait | widen | narrow |
//! | --- | --- | --- | --- | --- |
//! | datatype | [`DataType`] | [`DataTypeValue`] | `into_dtype` | `from_dtype` |
//! | field | [`Field`] | [`FieldValue`] | `into_field` | `from_field` |
//! | value | [`Scalar`] | [`Value`] | `into_scalar` | `from_scalar` |
//! | many | [`Serie`] | [`SerieValue`] | `into_serie` | `from_serie` |
//!
//! # Two kinds of many, and one type over both
//!
//! [`Serie::Run`] is a schema-free ordered run - what a row canonicalizes
//! to, what a document parses as, and what [`Scalar::from_sequence`] builds.
//! It holds its values and lends them. Every other leaf is a column: the
//! Arrow buffers of one [`Field`], holding no [`Scalar`] at all, building a
//! row only when one is asked for, writing its buffers in place when it
//! holds them alone, and holding nested children as [`Serie`] all the way
//! down.
//!
//! What separates them is the field. A run declares none, so
//! [`Serie::field`] answers `None` for it and its datatype is agreed back
//! out of its rows; a column carries one, so its datatype is read rather
//! than agreed and an empty column still names it.
//!
//! # The rows are buffers, and the leaf names them
//!
//! The root is flat, one variant per storage layout, spelled as the leaf that
//! holds it - `Int8`, `Decimal128`, `Time32Second`, `DurationMillisecond`,
//! `Utf8String`, `Serie`, `Map`, `SortedMap` - so a column's variant is the
//! buffers it holds and nothing wraps them. A layout serves every datatype
//! whose Arrow storage it is: `Utf8String` holds each string leaf laid out as
//! UTF-8, `DurationSecond` both duration widths, so which datatype a column
//! is is its field's [`id`](crate::SerieValue::id), never its variant. A code,
//! a version, a URI, a zone, a MIME or media type, a UUID and a geospatial
//! reading keep a variant of their own over the leaf they are stored in,
//! because a digest, a text body and a FIX payload each tell them apart:
//!
//! | root variants | held | what a leaf lends |
//! | --- | --- | --- |
//! | `Int8` .. `Float64` | [`Int8Serie`] .. [`Float64Serie`] | `&[i32]` and its kind, straight off the buffer |
//! | `Decimal32` .. `Decimal256` | [`Decimal32Serie`] .. [`Decimal256Serie`] | the coefficients, at the field's scale |
//! | `Date32`, `Date64` | [`Date32Serie`], [`Date64Serie`] | the counts |
//! | `Time32Second` .. `Time64Nanosecond`, `DateTimeSecond` .. `DateTimeNanosecond`, `DurationSecond` .. `DurationNanosecond`, `IntervalYearMonth` .. `IntervalMonthDayNano` | [`Time32SecondSerie`] .. [`IntervalMonthDayNanoSerie`] | the counts, at the unit the leaf is |
//! | `Utf8String` .. `FixedString` | [`Utf8StringSerie`] .. [`FixedStringSerie`] | the offsets and the character bytes |
//! | `Binary` .. `FixedBytes` | [`BinarySerie`] .. [`FixedBytesSerie`] | the offsets and the payload bytes |
//! | `Serie` .. `LargeSerieView`, `FixedSizeSerie` | [`SerieSerie`] .. [`FixedSizeSerieSerie`] | the offsets, and the item column under them |
//! | `Dictionary` | [`DictionarySerie`] | the key column and the values column |
//! | `Struct` | [`StructSerie`] | one child [`Serie`] per child field |
//! | `Map`, `SortedMap` | [`MapSerie`] | the offsets, and the entries column under them |
//! | [`UnionSerie`] | - | the type ids, the offsets, one child per member |
//! | [`RunEndEncodedSerie`] | - | the run ends and the values, each a column |
//! | [`VariantSerie`] | - | the encoded bytes of one row, decoded on demand |
//!
//! Every leaf is one [`ArrayRef`]'s buffers behind the field that types
//! them, so a column crosses into Arrow and back by sharing them -
//! [`Serie::from_arrow_array`] and [`Serie::into_arrow_array`] copy no row.
//! The column leaves are shared behind one pointer each, so a [`Scalar`]
//! carrying a serie is two words and cloning one is a pointer bump; the run
//! is held inline, because a row canonicalizes to one.
//!
//! # A column holds only rows its field accepts
//!
//! Every write proves its rows through the field's contract exactly once -
//! [`Serie::from_scalars`] and [`Serie::splice`] through [`Field::scalar`],
//! the Arrow door at import - so a stored row never refuses to be read, and a
//! refusal leaves the column exactly as it was. A run accepts any value,
//! because it declares none.
//!
//! # Identity is the rows
//!
//! Two series are one value when their rows are, whichever leaf holds them:
//! a run and a column of equal rows are equal and hash alike, and so are an
//! int32 column and an int64 column of equal numbers, exactly as their
//! [`Scalar`]s are. Nothing else counts - not the leaf, not the field.
//!
//! ```
//! use std::sync::Arc;
//!
//! use arrow_array::{ArrayRef, Int32Array};
//! use yggdryl::{ArrowCastOptions, DataType, Field, Int32Serie, Scalar, Serie};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let field = Field::new("price", DataType::Int32, false);
//! let array: ArrayRef = Arc::new(Int32Array::from(vec![125, 126, 127]));
//!
//! // The buffers cross in as they are: an exact layout is the identity cast.
//! let mut serie = Serie::from_arrow_array(Some(&field), array, ArrowCastOptions::new())?;
//! assert_eq!(serie.len(), 3);
//!
//! // And the leaf lends them: this is the values buffer itself.
//! let column: &Int32Serie = serie.as_int32().expect("an int32 column");
//! assert_eq!(column.values(), &[125, 126, 127]);
//! assert_eq!(column.value(1), Some(126));
//!
//! // One row becomes a value only when asked for one.
//! assert_eq!(serie.scalar(1)?, Scalar::from(126_i32));
//!
//! // It grows into the buffer it already holds, and a slot is one write.
//! serie.push(Scalar::from(128_i32))?;
//! serie.set(0, Scalar::from(124_i32))?;
//! assert_eq!(
//!     serie.as_int32().expect("an int32 column").values(),
//!     &[124, 126, 127, 128]
//! );
//! # Ok(())
//! # }
//! ```
//!
//! [`ArrayRef`]: arrow_array::ArrayRef
//! [`DataTypeValue`]: crate::DataTypeValue
//! [`FieldValue`]: crate::FieldValue
//! [`SerieValue`]: crate::SerieValue
//! [`Value`]: crate::Value

use std::borrow::Cow;
use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::sync::Arc;

use arrow_array::ArrayRef;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::expression::FieldSegment;
use crate::value::{Children, ColumnRows, NestedValue, SerieValue, Value};
use crate::{DataType, Field, FieldPath, Result, Scalar};

// ------------------------------------------------------------------------
// The machinery every leaf reads and writes through, declared before the
// modules that use it.
// ------------------------------------------------------------------------

/// Emit the identity a column has: its rows, and nothing else.
///
/// Ordering walks the rows, hashing writes what `<[Scalar]>::hash` writes,
/// so a column is one value with the run of its rows and with any other
/// column of equal rows, whichever field types them. Display renders the
/// rows behind the field's name. A parameter carries the bound its struct
/// declares, because an impl must restate what the struct requires.
macro_rules! serie_leaf {
    ($name:ident $(< $($param:ident $(: $bound:path)?),+ >)?) => {
        impl<$($($param $(: $bound)?),+)?> PartialEq for $name<$($($param),+)?>
        where
            Self: $crate::value::SerieValue,
        {
            fn eq(&self, other: &Self) -> bool {
                $crate::serie::compare_rows(self, other) == ::std::cmp::Ordering::Equal
            }
        }

        impl<$($($param $(: $bound)?),+)?> Eq for $name<$($($param),+)?> where
            Self: $crate::value::SerieValue
        {
        }

        impl<$($($param $(: $bound)?),+)?> PartialOrd for $name<$($($param),+)?>
        where
            Self: $crate::value::SerieValue,
        {
            fn partial_cmp(&self, other: &Self) -> Option<::std::cmp::Ordering> {
                Some(::std::cmp::Ord::cmp(self, other))
            }
        }

        impl<$($($param $(: $bound)?),+)?> Ord for $name<$($($param),+)?>
        where
            Self: $crate::value::SerieValue,
        {
            fn cmp(&self, other: &Self) -> ::std::cmp::Ordering {
                $crate::serie::compare_rows(self, other)
            }
        }

        impl<$($($param $(: $bound)?),+)?> ::std::hash::Hash for $name<$($($param),+)?>
        where
            Self: $crate::value::SerieValue,
        {
            fn hash<H: ::std::hash::Hasher>(&self, state: &mut H) {
                $crate::serie::hash_rows(self, state);
            }
        }

        impl<$($($param $(: $bound)?),+)?> ::std::fmt::Display for $name<$($($param),+)?>
        where
            Self: $crate::value::SerieValue,
        {
            fn fmt(&self, formatter: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                $crate::serie::display_column(self, formatter)
            }
        }
    };
}

pub(crate) mod arrow;
pub use arrow::SerieReader;
pub(crate) use arrow::{
    Proof, Resolved, canonical_rows, default_array, default_dtype_array, from_canonical_rows, land,
    land_batch, land_planned, land_resolved, land_under, proven_cell,
};
mod boolean;
mod bytes;
mod datatype;
mod enums;
pub(crate) mod layout;
mod mapping;
mod null;
mod primitive;
mod runend;
mod sequence;
mod string;
mod structure;
mod union;
pub(crate) mod value;
mod variant;

pub use boolean::BooleanSerie;
pub use bytes::{
    BinarySerie, BinaryViewSerie, ByteKind, ByteLeaf, ByteSerie, ByteViewSerie, Chars,
    FixedBytesSerie, FixedLeaf, FixedSerie, LargeBinarySerie, Octets, ViewLeaf,
};
pub use datatype::{Run, SerieType};
pub use enums::DictionarySerie;
pub use mapping::MapSerie;
pub use null::NullSerie;
pub use primitive::{
    Date32Serie, Date64Serie, DateTimeMicrosecondSerie, DateTimeMillisecondSerie,
    DateTimeNanosecondSerie, DateTimeSecondSerie, Decimal32Serie, Decimal64Serie, Decimal128Serie,
    Decimal256Serie, DurationMicrosecondSerie, DurationMillisecondSerie, DurationNanosecondSerie,
    DurationSecondSerie, Float16Serie, Float32Serie, Float64Serie, Int8Serie, Int16Serie,
    Int32Serie, Int64Serie, IntervalDayTimeSerie, IntervalMonthDayNanoSerie,
    IntervalYearMonthSerie, NativeLeaf, PrimitiveLeaf, PrimitiveSerie, Time32MillisecondSerie,
    Time32SecondSerie, Time64MicrosecondSerie, Time64NanosecondSerie, UInt8Serie, UInt16Serie,
    UInt32Serie, UInt64Serie,
};
pub use runend::RunEndEncodedSerie;
pub use sequence::{
    FixedSizeSerieSerie, LargeSerieSerie, LargeSerieViewSerie, OffsetLeaf, OffsetSerie,
    OffsetViewSerie, SerieSerie, SerieViewSerie,
};
pub use string::{
    BinaryStringSerie, BinaryViewStringSerie, FixedStringSerie, LargeBinaryStringSerie,
    LargeUtf8StringSerie, Utf8StringSerie, Utf8ViewStringSerie,
};
pub use structure::StructSerie;
pub use union::UnionSerie;
pub use variant::VariantSerie;

/// The invariant every stored row carries: the door proved it, so building
/// one cannot refuse.
pub(crate) const PROVEN: &str = "a stored row is one its field accepts: the door proved it";

/// What identity and rendering read: a length and one row at a time.
///
/// A run lends its rows; a column builds one at a time, and never a whole
/// `rows()` allocation.
pub(crate) trait Rows {
    /// The number of rows.
    fn rows_len(&self) -> usize;
    /// Row `index`, which the caller keeps below [`Self::rows_len`].
    fn row_at(&self, index: usize) -> Cow<'_, Scalar>;
}

impl<S: SerieValue> Rows for S {
    fn rows_len(&self) -> usize {
        self.len()
    }

    fn row_at(&self, index: usize) -> Cow<'_, Scalar> {
        Cow::Owned(self.scalar(index).expect(PROVEN))
    }
}

impl Rows for Serie {
    fn rows_len(&self) -> usize {
        self.len()
    }

    fn row_at(&self, index: usize) -> Cow<'_, Scalar> {
        match self {
            Self::Run(run) => Cow::Borrowed(&run.as_slice()[index]),
            column => Cow::Owned(proven_row(column, index)),
        }
    }
}

/// Build row `index` of a column, which the door proved and the caller keeps
/// below the length.
pub(crate) fn proven_row(serie: &Serie, index: usize) -> Scalar {
    serie.scalar(index).expect(PROVEN)
}

/// Order two series by their rows, then their length: what `<[Scalar]>::cmp`
/// answers for the runs of both.
pub(crate) fn compare_rows<L: Rows + ?Sized, R: Rows + ?Sized>(left: &L, right: &R) -> Ordering {
    let (left_len, right_len) = (left.rows_len(), right.rows_len());
    for index in 0..left_len.min(right_len) {
        let step = left.row_at(index).cmp(&right.row_at(index));
        if step != Ordering::Equal {
            return step;
        }
    }
    left_len.cmp(&right_len)
}

/// Hash the rows exactly as `<[Scalar]>::hash` does: the length, then each
/// row in order, so a column hashes byte-identically to the run of its rows.
pub(crate) fn hash_rows<S: Rows + ?Sized, H: Hasher>(rows: &S, state: &mut H) {
    let len = rows.rows_len();
    state.write_usize(len);
    for index in 0..len {
        rows.row_at(index).hash(state);
    }
}

/// Render a column as its field's name and the rows it holds.
pub(crate) fn display_column<S: SerieValue>(
    column: &S,
    formatter: &mut fmt::Formatter<'_>,
) -> fmt::Result {
    write!(formatter, "{}[", column.field().name())?;
    for index in 0..column.len() {
        if index != 0 {
            formatter.write_str(", ")?;
        }
        write!(formatter, "{:?}", column.row_at(index))?;
    }
    formatter.write_str("]")
}

/// Render a column's shape without reading a buffer.
pub(crate) fn debug_column<S: SerieValue>(
    column: &S,
    name: &'static str,
    formatter: &mut fmt::Formatter<'_>,
) -> fmt::Result {
    formatter
        .debug_struct(name)
        .field("field", column.field())
        .field("len", &column.len())
        .field("nulls", &column.null_count())
        .finish()
}

/// Refuse a row past the end, naming the column and both counts.
pub(crate) fn require_row(name: &str, index: usize, len: usize) -> Result<()> {
    if index < len {
        return Ok(());
    }
    Err(crate::Error::InvalidRecord {
        path: smol_str::SmolStr::new(name),
        reason: smol_str::format_smolstr!("row {index} is past the {len} rows {name} holds"),
    })
}

/// Refuse a reversed range or one reaching past the end, naming the column
/// and both counts.
pub(crate) fn require_range(name: &str, range: &Range<usize>, len: usize) -> Result<()> {
    if range.start > range.end {
        return Err(crate::Error::InvalidRecord {
            path: smol_str::SmolStr::new(name),
            reason: smol_str::format_smolstr!(
                "rows {}..{} run backwards in {name}",
                range.start,
                range.end
            ),
        });
    }
    if range.end <= len {
        return Ok(());
    }
    Err(crate::Error::InvalidRecord {
        path: smol_str::SmolStr::new(name),
        reason: smol_str::format_smolstr!(
            "rows {}..{} reach past the {len} rows {name} holds",
            range.start,
            range.end
        ),
    })
}

/// Refuse a window `offset..offset + length` reaching past the end, naming
/// the column and both counts; a sum that overflows reaches past any end.
pub(crate) fn require_window(name: &str, offset: usize, length: usize, len: usize) -> Result<()> {
    let end = offset
        .checked_add(length)
        .ok_or_else(|| crate::Error::InvalidRecord {
            path: smol_str::SmolStr::new(name),
            reason: smol_str::format_smolstr!(
                "a window of {length} rows from {offset} reaches past the {len} rows {name} holds"
            ),
        })?;
    require_range(name, &(offset..end), len)
}

// ------------------------------------------------------------------------
// The root.
// ------------------------------------------------------------------------

/// The serie family's value: many values, under one field or under none.
///
/// One type answers "many values" everywhere in the crate. A [`Serie::Run`]
/// is a schema-free ordered run - what a row canonicalizes to, and what a
/// document parses as. Every other leaf is a column: the Arrow buffers of
/// one [`Field`], one variant per storage layout, named as the leaf that
/// holds it. A variant names the layout, not the datatype: `Utf8String`
/// holds every string leaf laid out as UTF-8 - `ascii` and `sized_utf8`
/// among them - `Binary` a sized binary too, `BinaryView` a large view, and
/// `DurationSecond` both duration widths, so a column's datatype is its
/// field's, read through [`SerieValue::id`]. A code, a version, a URI, a
/// zone or a MIME or media type holds the UTF-8 column it is stored in, a
/// UUID its sixteen fixed bytes and a geospatial reading its Well-Known
/// Binary, each under a variant of its own - which is why
/// [`Self::as_utf8`], [`Self::as_fixed_bytes`] and [`Self::as_binary`]
/// reach those variants too.
///
/// The column leaves are shared behind one pointer each, so a [`Scalar`]
/// carrying a serie is two words and a clone of one is a pointer bump. A
/// write goes through `Arc::make_mut`: in place when this serie is the only
/// holder, and when it is not the leaf struct is copied (pointer bumps,
/// since its Arrow buffers are shared) and the first buffer edit copies the
/// rows once; every later edit is in place. The run is held inline, because
/// a row is one and paying an extra indirection per row is the one cost
/// this type cannot take.
#[derive(Clone)]
#[non_exhaustive]
pub enum Serie {
    /// A schema-free ordered run of values: what a row canonicalizes to.
    Run(Run),
    /// A column of nulls: a length, and no buffer at all.
    Null(Arc<NullSerie>),
    /// A column of booleans.
    Boolean(Arc<BooleanSerie>),
    /// A column of `int8` values.
    Int8(Arc<Int8Serie>),
    /// A column of `int16` values.
    Int16(Arc<Int16Serie>),
    /// A column of `int32` values.
    Int32(Arc<Int32Serie>),
    /// A column of `int64` values.
    Int64(Arc<Int64Serie>),
    /// A column of `uint8` values.
    UInt8(Arc<UInt8Serie>),
    /// A column of `uint16` values.
    UInt16(Arc<UInt16Serie>),
    /// A column of `uint32` values.
    UInt32(Arc<UInt32Serie>),
    /// A column of `uint64` values.
    UInt64(Arc<UInt64Serie>),
    /// A column of `float16` values.
    Float16(Arc<Float16Serie>),
    /// A column of `float32` values.
    Float32(Arc<Float32Serie>),
    /// A column of `float64` values.
    Float64(Arc<Float64Serie>),
    /// A column of datetimes counted in seconds.
    DateTimeSecond(Arc<DateTimeSecondSerie>),
    /// A column of datetimes counted in milliseconds.
    DateTimeMillisecond(Arc<DateTimeMillisecondSerie>),
    /// A column of datetimes counted in microseconds.
    DateTimeMicrosecond(Arc<DateTimeMicrosecondSerie>),
    /// A column of datetimes counted in nanoseconds.
    DateTimeNanosecond(Arc<DateTimeNanosecondSerie>),
    /// A column of day counts.
    Date32(Arc<Date32Serie>),
    /// A column of the milliseconds of midnights.
    Date64(Arc<Date64Serie>),
    /// A column of 32-bit times of day in seconds.
    Time32Second(Arc<Time32SecondSerie>),
    /// A column of 32-bit times of day in milliseconds.
    Time32Millisecond(Arc<Time32MillisecondSerie>),
    /// A column of 64-bit times of day in microseconds.
    Time64Microsecond(Arc<Time64MicrosecondSerie>),
    /// A column of 64-bit times of day in nanoseconds.
    Time64Nanosecond(Arc<Time64NanosecondSerie>),
    /// A column of durations counted in seconds, at the width its field declares.
    DurationSecond(Arc<DurationSecondSerie>),
    /// A column of durations counted in milliseconds, at the width its field declares.
    DurationMillisecond(Arc<DurationMillisecondSerie>),
    /// A column of durations counted in microseconds, at the width its field declares.
    DurationMicrosecond(Arc<DurationMicrosecondSerie>),
    /// A column of durations counted in nanoseconds, at the width its field declares.
    DurationNanosecond(Arc<DurationNanosecondSerie>),
    /// A column of calendar intervals in months.
    IntervalYearMonth(Arc<IntervalYearMonthSerie>),
    /// A column of calendar intervals in days and milliseconds.
    IntervalDayTime(Arc<IntervalDayTimeSerie>),
    /// A column of calendar intervals in months, days and nanoseconds.
    IntervalMonthDayNano(Arc<IntervalMonthDayNanoSerie>),
    /// A column of bytes behind 32-bit offsets.
    Binary(Arc<BinarySerie>),
    /// A column of bytes behind 64-bit offsets.
    LargeBinary(Arc<LargeBinarySerie>),
    /// A column of bytes behind views.
    BinaryView(Arc<BinaryViewSerie>),
    /// A column of bytes at one fixed width.
    FixedBytes(Arc<FixedBytesSerie>),
    /// A column of text as UTF-8 behind 32-bit offsets.
    Utf8String(Arc<Utf8StringSerie>),
    /// A column of text as UTF-8 behind 64-bit offsets.
    LargeUtf8String(Arc<LargeUtf8StringSerie>),
    /// A column of text as UTF-8 behind views.
    Utf8ViewString(Arc<Utf8ViewStringSerie>),
    /// A column of text in its field's charset behind 32-bit offsets.
    BinaryString(Arc<BinaryStringSerie>),
    /// A column of text in its field's charset behind 64-bit offsets.
    LargeBinaryString(Arc<LargeBinaryStringSerie>),
    /// A column of text in its field's charset behind views.
    BinaryViewString(Arc<BinaryViewStringSerie>),
    /// A column of text in its field's charset, padded to one fixed width.
    FixedString(Arc<FixedStringSerie>),
    /// A column of `Country` values, stored as their UTF-8 text.
    Country(Arc<Utf8StringSerie>),
    /// A column of `Ccy` values, stored as their UTF-8 text.
    Ccy(Arc<Utf8StringSerie>),
    /// A column of `Mic` values, stored as their UTF-8 text.
    Mic(Arc<Utf8StringSerie>),
    /// A column of `Cfi` values, stored as their UTF-8 text.
    Cfi(Arc<Utf8StringSerie>),
    /// A column of `Isin` values, stored as their UTF-8 text.
    Isin(Arc<Utf8StringSerie>),
    /// A column of `Side` values, stored as their UTF-8 text.
    Side(Arc<Utf8StringSerie>),
    /// A column of `State` values, stored as their UTF-8 text.
    State(Arc<Utf8StringSerie>),
    /// A column of `TimeInForce` values, stored as their UTF-8 text.
    TimeInForce(Arc<Utf8StringSerie>),
    /// A column of `Version` values, stored as their UTF-8 text.
    Version(Arc<Utf8StringSerie>),
    /// A column of `Url` values, stored as their UTF-8 text.
    Url(Arc<Utf8StringSerie>),
    /// A column of `Urn` values, stored as their UTF-8 text.
    Urn(Arc<Utf8StringSerie>),
    /// A column of `Timezone` values, stored as their UTF-8 text.
    Timezone(Arc<Utf8StringSerie>),
    /// A column of `MimeType` values, stored as their UTF-8 text.
    MimeType(Arc<Utf8StringSerie>),
    /// A column of `MediaType` values, stored as their UTF-8 text.
    MediaType(Arc<Utf8StringSerie>),
    /// A column of `Cusip` values, stored as their UTF-8 text.
    Cusip(Arc<Utf8StringSerie>),
    /// A column of `Sedol` values, stored as their UTF-8 text.
    Sedol(Arc<Utf8StringSerie>),
    /// A column of `Bbg` values, stored as their UTF-8 text.
    Bbg(Arc<Utf8StringSerie>),
    /// A column of `Ric` values, stored as their UTF-8 text.
    Ric(Arc<Utf8StringSerie>),
    /// A column of `Figi` values, stored as their UTF-8 text.
    Figi(Arc<Utf8StringSerie>),
    /// A column of `Unit` values, stored as their UTF-8 text.
    Unit(Arc<Utf8StringSerie>),
    /// A column of UUIDs, sixteen fixed bytes each.
    Uuid(Arc<FixedBytesSerie>),
    /// A column of series: 32-bit offsets over one item column.
    Serie(Arc<SerieSerie>),
    /// A column of serie views: 32-bit offsets and sizes over one item column.
    SerieView(Arc<SerieViewSerie>),
    /// A column of fixed-size series: `width` items per row.
    FixedSizeSerie(Arc<FixedSizeSerieSerie>),
    /// A column of large series: 64-bit offsets over one item column.
    LargeSerie(Arc<LargeSerieSerie>),
    /// A column of large serie views: 64-bit offsets and sizes over one item column.
    LargeSerieView(Arc<LargeSerieViewSerie>),
    /// A column of records, each child a serie of its own.
    Struct(Arc<StructSerie>),
    /// A column of union rows, one child column per member.
    Union(Arc<UnionSerie>),
    /// A column of dictionary-encoded rows: a key column over its values.
    Dictionary(Arc<DictionarySerie>),
    /// A column of 32-bit decimal coefficients, at the field's scale.
    Decimal32(Arc<Decimal32Serie>),
    /// A column of 64-bit decimal coefficients, at the field's scale.
    Decimal64(Arc<Decimal64Serie>),
    /// A column of 128-bit decimal coefficients, at the field's scale.
    Decimal128(Arc<Decimal128Serie>),
    /// A column of 256-bit decimal coefficients, at the field's scale.
    Decimal256(Arc<Decimal256Serie>),
    /// A column of maps: offsets over one record column of entries.
    Map(Arc<MapSerie>),
    /// A column of maps whose keys every row holds sorted.
    SortedMap(Arc<MapSerie>),
    /// A column of run-end-encoded rows: the run ends over their values.
    RunEndEncoded(Arc<RunEndEncodedSerie>),
    /// A column of self-describing values, the Parquet Variant pair per row.
    Variant(Arc<VariantSerie>),
    /// A column of planar geospatial features, as Well-Known Binary.
    Geometry(Arc<BinarySerie>),
    /// A column of geospatial features on a sphere, as Well-Known Binary.
    Geography(Arc<BinarySerie>),
}

// A 16-byte `Arc<[Scalar]>` inline beside a discriminant; every column leaf
// is one thin pointer.
const _: () = assert!(size_of::<Serie>() == 24);

/// Forward one verb to whichever column holds the rows, with the run's own
/// answer beside it.
///
/// The run is the one leaf that carries no field, so every verb that reads a
/// field states what a run answers instead rather than pretending it has one.
macro_rules! column {
    ($self:ident, $run:ident => $bare:expr, $column:ident => $answer:expr) => {
        match $self {
            Serie::Run($run) => $bare,
            Serie::Null($column) => $answer,
            Serie::Boolean($column) => $answer,
            Serie::Int8($column) => $answer,
            Serie::Int16($column) => $answer,
            Serie::Int32($column) => $answer,
            Serie::Int64($column) => $answer,
            Serie::UInt8($column) => $answer,
            Serie::UInt16($column) => $answer,
            Serie::UInt32($column) => $answer,
            Serie::UInt64($column) => $answer,
            Serie::Float16($column) => $answer,
            Serie::Float32($column) => $answer,
            Serie::Float64($column) => $answer,
            Serie::DateTimeSecond($column) => $answer,
            Serie::DateTimeMillisecond($column) => $answer,
            Serie::DateTimeMicrosecond($column) => $answer,
            Serie::DateTimeNanosecond($column) => $answer,
            Serie::Date32($column) => $answer,
            Serie::Date64($column) => $answer,
            Serie::Time32Second($column) => $answer,
            Serie::Time32Millisecond($column) => $answer,
            Serie::Time64Microsecond($column) => $answer,
            Serie::Time64Nanosecond($column) => $answer,
            Serie::DurationSecond($column) => $answer,
            Serie::DurationMillisecond($column) => $answer,
            Serie::DurationMicrosecond($column) => $answer,
            Serie::DurationNanosecond($column) => $answer,
            Serie::IntervalYearMonth($column) => $answer,
            Serie::IntervalDayTime($column) => $answer,
            Serie::IntervalMonthDayNano($column) => $answer,
            Serie::Binary($column) => $answer,
            Serie::LargeBinary($column) => $answer,
            Serie::BinaryView($column) => $answer,
            Serie::FixedBytes($column) => $answer,
            Serie::Utf8String($column) => $answer,
            Serie::LargeUtf8String($column) => $answer,
            Serie::Utf8ViewString($column) => $answer,
            Serie::BinaryString($column) => $answer,
            Serie::LargeBinaryString($column) => $answer,
            Serie::BinaryViewString($column) => $answer,
            Serie::FixedString($column) => $answer,
            Serie::Country($column) => $answer,
            Serie::Ccy($column) => $answer,
            Serie::Mic($column) => $answer,
            Serie::Cfi($column) => $answer,
            Serie::Isin($column) => $answer,
            Serie::Side($column) => $answer,
            Serie::State($column) => $answer,
            Serie::TimeInForce($column) => $answer,
            Serie::Version($column) => $answer,
            Serie::Url($column) => $answer,
            Serie::Urn($column) => $answer,
            Serie::Timezone($column) => $answer,
            Serie::MimeType($column) => $answer,
            Serie::MediaType($column) => $answer,
            Serie::Cusip($column) => $answer,
            Serie::Sedol($column) => $answer,
            Serie::Bbg($column) => $answer,
            Serie::Ric($column) => $answer,
            Serie::Figi($column) => $answer,
            Serie::Unit($column) => $answer,
            Serie::Uuid($column) => $answer,
            Serie::Serie($column) => $answer,
            Serie::SerieView($column) => $answer,
            Serie::FixedSizeSerie($column) => $answer,
            Serie::LargeSerie($column) => $answer,
            Serie::LargeSerieView($column) => $answer,
            Serie::Struct($column) => $answer,
            Serie::Union($column) => $answer,
            Serie::Dictionary($column) => $answer,
            Serie::Decimal32($column) => $answer,
            Serie::Decimal64($column) => $answer,
            Serie::Decimal128($column) => $answer,
            Serie::Decimal256($column) => $answer,
            Serie::Map($column) => $answer,
            Serie::SortedMap($column) => $answer,
            Serie::RunEndEncoded($column) => $answer,
            Serie::Variant($column) => $answer,
            Serie::Geometry($column) => $answer,
            Serie::Geography($column) => $answer,
        }
    };
}

/// The same, where the verb needs the column to write to.
///
/// `Arc::make_mut` copies the leaf struct once when the column is shared -
/// pointer bumps, since its Arrow buffers are - and the first buffer edit
/// then copies the rows once.
macro_rules! column_mut {
    ($self:ident, $run:ident => $bare:expr, $column:ident => $answer:expr) => {
        match $self {
            Serie::Run($run) => $bare,
            Serie::Null(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::Boolean(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::Int8(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::Int16(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::Int32(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::Int64(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::UInt8(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::UInt16(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::UInt32(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::UInt64(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::Float16(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::Float32(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::Float64(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::DateTimeSecond(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::DateTimeMillisecond(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::DateTimeMicrosecond(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::DateTimeNanosecond(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::Date32(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::Date64(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::Time32Second(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::Time32Millisecond(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::Time64Microsecond(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::Time64Nanosecond(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::DurationSecond(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::DurationMillisecond(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::DurationMicrosecond(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::DurationNanosecond(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::IntervalYearMonth(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::IntervalDayTime(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::IntervalMonthDayNano(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::Binary(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::LargeBinary(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::BinaryView(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::FixedBytes(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::Utf8String(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::LargeUtf8String(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::Utf8ViewString(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::BinaryString(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::LargeBinaryString(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::BinaryViewString(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::FixedString(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::Country(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::Ccy(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::Mic(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::Cfi(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::Isin(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::Side(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::State(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::TimeInForce(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::Version(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::Url(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::Urn(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::Timezone(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::MimeType(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::MediaType(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::Cusip(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::Sedol(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::Bbg(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::Ric(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::Figi(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::Unit(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::Uuid(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::Serie(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::SerieView(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::FixedSizeSerie(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::LargeSerie(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::LargeSerieView(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::Struct(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::Union(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::Dictionary(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::Decimal32(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::Decimal64(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::Decimal128(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::Decimal256(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::Map(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::SortedMap(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::RunEndEncoded(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::Variant(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::Geometry(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::Geography(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
        }
    };
}

/// Where one column type sits in the root: rooting a leaf and narrowing the
/// root back to it read the one table this is generated from, so a leaf
/// that more than one variant holds - the UTF-8 column behind every code,
/// the fixed bytes behind a UUID, the binary behind a geospatial reading -
/// roots by its field's datatype and narrows from any of them.
pub(crate) trait Leaf: Sized {
    /// Widen this column to the root variant its field's datatype names.
    fn root(self) -> Serie;
    /// Borrow this column out of the root, `None` for any other leaf.
    fn narrow(serie: &Serie) -> Option<&Self>;
    /// Borrow this column out of the root to write, copying the leaf struct
    /// once when it is shared; `None` for any other leaf.
    fn narrow_mut(serie: &mut Serie) -> Option<&mut Self>;
}

impl Leaf for NullSerie {
    fn root(self) -> Serie {
        Serie::Null(Arc::new(self))
    }

    fn narrow(serie: &Serie) -> Option<&Self> {
        match serie {
            Serie::Null(held) => Some(held.as_ref()),
            _ => None,
        }
    }

    fn narrow_mut(serie: &mut Serie) -> Option<&mut Self> {
        match serie {
            Serie::Null(held) => Some(Arc::make_mut(held)),
            _ => None,
        }
    }
}

impl Leaf for BooleanSerie {
    fn root(self) -> Serie {
        Serie::Boolean(Arc::new(self))
    }

    fn narrow(serie: &Serie) -> Option<&Self> {
        match serie {
            Serie::Boolean(held) => Some(held.as_ref()),
            _ => None,
        }
    }

    fn narrow_mut(serie: &mut Serie) -> Option<&mut Self> {
        match serie {
            Serie::Boolean(held) => Some(Arc::make_mut(held)),
            _ => None,
        }
    }
}

impl Leaf for Int8Serie {
    fn root(self) -> Serie {
        Serie::Int8(Arc::new(self))
    }

    fn narrow(serie: &Serie) -> Option<&Self> {
        match serie {
            Serie::Int8(held) => Some(held.as_ref()),
            _ => None,
        }
    }

    fn narrow_mut(serie: &mut Serie) -> Option<&mut Self> {
        match serie {
            Serie::Int8(held) => Some(Arc::make_mut(held)),
            _ => None,
        }
    }
}

impl Leaf for Int16Serie {
    fn root(self) -> Serie {
        Serie::Int16(Arc::new(self))
    }

    fn narrow(serie: &Serie) -> Option<&Self> {
        match serie {
            Serie::Int16(held) => Some(held.as_ref()),
            _ => None,
        }
    }

    fn narrow_mut(serie: &mut Serie) -> Option<&mut Self> {
        match serie {
            Serie::Int16(held) => Some(Arc::make_mut(held)),
            _ => None,
        }
    }
}

impl Leaf for Int32Serie {
    fn root(self) -> Serie {
        Serie::Int32(Arc::new(self))
    }

    fn narrow(serie: &Serie) -> Option<&Self> {
        match serie {
            Serie::Int32(held) => Some(held.as_ref()),
            _ => None,
        }
    }

    fn narrow_mut(serie: &mut Serie) -> Option<&mut Self> {
        match serie {
            Serie::Int32(held) => Some(Arc::make_mut(held)),
            _ => None,
        }
    }
}

impl Leaf for Int64Serie {
    fn root(self) -> Serie {
        Serie::Int64(Arc::new(self))
    }

    fn narrow(serie: &Serie) -> Option<&Self> {
        match serie {
            Serie::Int64(held) => Some(held.as_ref()),
            _ => None,
        }
    }

    fn narrow_mut(serie: &mut Serie) -> Option<&mut Self> {
        match serie {
            Serie::Int64(held) => Some(Arc::make_mut(held)),
            _ => None,
        }
    }
}

impl Leaf for UInt8Serie {
    fn root(self) -> Serie {
        Serie::UInt8(Arc::new(self))
    }

    fn narrow(serie: &Serie) -> Option<&Self> {
        match serie {
            Serie::UInt8(held) => Some(held.as_ref()),
            _ => None,
        }
    }

    fn narrow_mut(serie: &mut Serie) -> Option<&mut Self> {
        match serie {
            Serie::UInt8(held) => Some(Arc::make_mut(held)),
            _ => None,
        }
    }
}

impl Leaf for UInt16Serie {
    fn root(self) -> Serie {
        Serie::UInt16(Arc::new(self))
    }

    fn narrow(serie: &Serie) -> Option<&Self> {
        match serie {
            Serie::UInt16(held) => Some(held.as_ref()),
            _ => None,
        }
    }

    fn narrow_mut(serie: &mut Serie) -> Option<&mut Self> {
        match serie {
            Serie::UInt16(held) => Some(Arc::make_mut(held)),
            _ => None,
        }
    }
}

impl Leaf for UInt32Serie {
    fn root(self) -> Serie {
        Serie::UInt32(Arc::new(self))
    }

    fn narrow(serie: &Serie) -> Option<&Self> {
        match serie {
            Serie::UInt32(held) => Some(held.as_ref()),
            _ => None,
        }
    }

    fn narrow_mut(serie: &mut Serie) -> Option<&mut Self> {
        match serie {
            Serie::UInt32(held) => Some(Arc::make_mut(held)),
            _ => None,
        }
    }
}

impl Leaf for UInt64Serie {
    fn root(self) -> Serie {
        Serie::UInt64(Arc::new(self))
    }

    fn narrow(serie: &Serie) -> Option<&Self> {
        match serie {
            Serie::UInt64(held) => Some(held.as_ref()),
            _ => None,
        }
    }

    fn narrow_mut(serie: &mut Serie) -> Option<&mut Self> {
        match serie {
            Serie::UInt64(held) => Some(Arc::make_mut(held)),
            _ => None,
        }
    }
}

impl Leaf for Float16Serie {
    fn root(self) -> Serie {
        Serie::Float16(Arc::new(self))
    }

    fn narrow(serie: &Serie) -> Option<&Self> {
        match serie {
            Serie::Float16(held) => Some(held.as_ref()),
            _ => None,
        }
    }

    fn narrow_mut(serie: &mut Serie) -> Option<&mut Self> {
        match serie {
            Serie::Float16(held) => Some(Arc::make_mut(held)),
            _ => None,
        }
    }
}

impl Leaf for Float32Serie {
    fn root(self) -> Serie {
        Serie::Float32(Arc::new(self))
    }

    fn narrow(serie: &Serie) -> Option<&Self> {
        match serie {
            Serie::Float32(held) => Some(held.as_ref()),
            _ => None,
        }
    }

    fn narrow_mut(serie: &mut Serie) -> Option<&mut Self> {
        match serie {
            Serie::Float32(held) => Some(Arc::make_mut(held)),
            _ => None,
        }
    }
}

impl Leaf for Float64Serie {
    fn root(self) -> Serie {
        Serie::Float64(Arc::new(self))
    }

    fn narrow(serie: &Serie) -> Option<&Self> {
        match serie {
            Serie::Float64(held) => Some(held.as_ref()),
            _ => None,
        }
    }

    fn narrow_mut(serie: &mut Serie) -> Option<&mut Self> {
        match serie {
            Serie::Float64(held) => Some(Arc::make_mut(held)),
            _ => None,
        }
    }
}

impl Leaf for Date32Serie {
    fn root(self) -> Serie {
        Serie::Date32(Arc::new(self))
    }

    fn narrow(serie: &Serie) -> Option<&Self> {
        match serie {
            Serie::Date32(held) => Some(held.as_ref()),
            _ => None,
        }
    }

    fn narrow_mut(serie: &mut Serie) -> Option<&mut Self> {
        match serie {
            Serie::Date32(held) => Some(Arc::make_mut(held)),
            _ => None,
        }
    }
}

impl Leaf for Date64Serie {
    fn root(self) -> Serie {
        Serie::Date64(Arc::new(self))
    }

    fn narrow(serie: &Serie) -> Option<&Self> {
        match serie {
            Serie::Date64(held) => Some(held.as_ref()),
            _ => None,
        }
    }

    fn narrow_mut(serie: &mut Serie) -> Option<&mut Self> {
        match serie {
            Serie::Date64(held) => Some(Arc::make_mut(held)),
            _ => None,
        }
    }
}

impl Leaf for Utf8StringSerie {
    fn root(self) -> Serie {
        match SerieValue::field(&self).dtype() {
            DataType::Country => Serie::Country(Arc::new(self)),
            DataType::Ccy => Serie::Ccy(Arc::new(self)),
            DataType::Mic => Serie::Mic(Arc::new(self)),
            DataType::Cfi => Serie::Cfi(Arc::new(self)),
            DataType::Isin => Serie::Isin(Arc::new(self)),
            DataType::Side => Serie::Side(Arc::new(self)),
            DataType::State => Serie::State(Arc::new(self)),
            DataType::TimeInForce => Serie::TimeInForce(Arc::new(self)),
            DataType::Version => Serie::Version(Arc::new(self)),
            DataType::Url => Serie::Url(Arc::new(self)),
            DataType::Urn => Serie::Urn(Arc::new(self)),
            DataType::Timezone => Serie::Timezone(Arc::new(self)),
            DataType::MimeType => Serie::MimeType(Arc::new(self)),
            DataType::MediaType => Serie::MediaType(Arc::new(self)),
            DataType::Cusip => Serie::Cusip(Arc::new(self)),
            DataType::Sedol => Serie::Sedol(Arc::new(self)),
            DataType::Bbg => Serie::Bbg(Arc::new(self)),
            DataType::Ric => Serie::Ric(Arc::new(self)),
            DataType::Figi => Serie::Figi(Arc::new(self)),
            DataType::Unit => Serie::Unit(Arc::new(self)),
            _ => Serie::Utf8String(Arc::new(self)),
        }
    }

    fn narrow(serie: &Serie) -> Option<&Self> {
        match serie {
            Serie::Utf8String(held)
            | Serie::Country(held)
            | Serie::Ccy(held)
            | Serie::Mic(held)
            | Serie::Cfi(held)
            | Serie::Isin(held)
            | Serie::Side(held)
            | Serie::State(held)
            | Serie::TimeInForce(held)
            | Serie::Version(held)
            | Serie::Url(held)
            | Serie::Urn(held)
            | Serie::Timezone(held)
            | Serie::MimeType(held)
            | Serie::MediaType(held)
            | Serie::Cusip(held)
            | Serie::Sedol(held)
            | Serie::Bbg(held)
            | Serie::Ric(held)
            | Serie::Figi(held)
            | Serie::Unit(held) => Some(held.as_ref()),
            _ => None,
        }
    }

    fn narrow_mut(serie: &mut Serie) -> Option<&mut Self> {
        match serie {
            Serie::Utf8String(held)
            | Serie::Country(held)
            | Serie::Ccy(held)
            | Serie::Mic(held)
            | Serie::Cfi(held)
            | Serie::Isin(held)
            | Serie::Side(held)
            | Serie::State(held)
            | Serie::TimeInForce(held)
            | Serie::Version(held)
            | Serie::Url(held)
            | Serie::Urn(held)
            | Serie::Timezone(held)
            | Serie::MimeType(held)
            | Serie::MediaType(held)
            | Serie::Cusip(held)
            | Serie::Sedol(held)
            | Serie::Bbg(held)
            | Serie::Ric(held)
            | Serie::Figi(held)
            | Serie::Unit(held) => Some(Arc::make_mut(held)),
            _ => None,
        }
    }
}

impl Leaf for FixedBytesSerie {
    fn root(self) -> Serie {
        match SerieValue::field(&self).dtype() {
            DataType::Uuid => Serie::Uuid(Arc::new(self)),
            _ => Serie::FixedBytes(Arc::new(self)),
        }
    }

    fn narrow(serie: &Serie) -> Option<&Self> {
        match serie {
            Serie::FixedBytes(held) | Serie::Uuid(held) => Some(held.as_ref()),
            _ => None,
        }
    }

    fn narrow_mut(serie: &mut Serie) -> Option<&mut Self> {
        match serie {
            Serie::FixedBytes(held) | Serie::Uuid(held) => Some(Arc::make_mut(held)),
            _ => None,
        }
    }
}

impl Leaf for SerieSerie {
    fn root(self) -> Serie {
        Serie::Serie(Arc::new(self))
    }

    fn narrow(serie: &Serie) -> Option<&Self> {
        match serie {
            Serie::Serie(held) => Some(held.as_ref()),
            _ => None,
        }
    }

    fn narrow_mut(serie: &mut Serie) -> Option<&mut Self> {
        match serie {
            Serie::Serie(held) => Some(Arc::make_mut(held)),
            _ => None,
        }
    }
}

impl Leaf for SerieViewSerie {
    fn root(self) -> Serie {
        Serie::SerieView(Arc::new(self))
    }

    fn narrow(serie: &Serie) -> Option<&Self> {
        match serie {
            Serie::SerieView(held) => Some(held.as_ref()),
            _ => None,
        }
    }

    fn narrow_mut(serie: &mut Serie) -> Option<&mut Self> {
        match serie {
            Serie::SerieView(held) => Some(Arc::make_mut(held)),
            _ => None,
        }
    }
}

impl Leaf for FixedSizeSerieSerie {
    fn root(self) -> Serie {
        Serie::FixedSizeSerie(Arc::new(self))
    }

    fn narrow(serie: &Serie) -> Option<&Self> {
        match serie {
            Serie::FixedSizeSerie(held) => Some(held.as_ref()),
            _ => None,
        }
    }

    fn narrow_mut(serie: &mut Serie) -> Option<&mut Self> {
        match serie {
            Serie::FixedSizeSerie(held) => Some(Arc::make_mut(held)),
            _ => None,
        }
    }
}

impl Leaf for LargeSerieSerie {
    fn root(self) -> Serie {
        Serie::LargeSerie(Arc::new(self))
    }

    fn narrow(serie: &Serie) -> Option<&Self> {
        match serie {
            Serie::LargeSerie(held) => Some(held.as_ref()),
            _ => None,
        }
    }

    fn narrow_mut(serie: &mut Serie) -> Option<&mut Self> {
        match serie {
            Serie::LargeSerie(held) => Some(Arc::make_mut(held)),
            _ => None,
        }
    }
}

impl Leaf for LargeSerieViewSerie {
    fn root(self) -> Serie {
        Serie::LargeSerieView(Arc::new(self))
    }

    fn narrow(serie: &Serie) -> Option<&Self> {
        match serie {
            Serie::LargeSerieView(held) => Some(held.as_ref()),
            _ => None,
        }
    }

    fn narrow_mut(serie: &mut Serie) -> Option<&mut Self> {
        match serie {
            Serie::LargeSerieView(held) => Some(Arc::make_mut(held)),
            _ => None,
        }
    }
}

impl Leaf for StructSerie {
    fn root(self) -> Serie {
        Serie::Struct(Arc::new(self))
    }

    fn narrow(serie: &Serie) -> Option<&Self> {
        match serie {
            Serie::Struct(held) => Some(held.as_ref()),
            _ => None,
        }
    }

    fn narrow_mut(serie: &mut Serie) -> Option<&mut Self> {
        match serie {
            Serie::Struct(held) => Some(Arc::make_mut(held)),
            _ => None,
        }
    }
}

impl Leaf for UnionSerie {
    fn root(self) -> Serie {
        Serie::Union(Arc::new(self))
    }

    fn narrow(serie: &Serie) -> Option<&Self> {
        match serie {
            Serie::Union(held) => Some(held.as_ref()),
            _ => None,
        }
    }

    fn narrow_mut(serie: &mut Serie) -> Option<&mut Self> {
        match serie {
            Serie::Union(held) => Some(Arc::make_mut(held)),
            _ => None,
        }
    }
}

impl Leaf for DictionarySerie {
    fn root(self) -> Serie {
        Serie::Dictionary(Arc::new(self))
    }

    fn narrow(serie: &Serie) -> Option<&Self> {
        match serie {
            Serie::Dictionary(held) => Some(held.as_ref()),
            _ => None,
        }
    }

    fn narrow_mut(serie: &mut Serie) -> Option<&mut Self> {
        match serie {
            Serie::Dictionary(held) => Some(Arc::make_mut(held)),
            _ => None,
        }
    }
}

impl Leaf for Decimal32Serie {
    fn root(self) -> Serie {
        Serie::Decimal32(Arc::new(self))
    }

    fn narrow(serie: &Serie) -> Option<&Self> {
        match serie {
            Serie::Decimal32(held) => Some(held.as_ref()),
            _ => None,
        }
    }

    fn narrow_mut(serie: &mut Serie) -> Option<&mut Self> {
        match serie {
            Serie::Decimal32(held) => Some(Arc::make_mut(held)),
            _ => None,
        }
    }
}

impl Leaf for Decimal64Serie {
    fn root(self) -> Serie {
        Serie::Decimal64(Arc::new(self))
    }

    fn narrow(serie: &Serie) -> Option<&Self> {
        match serie {
            Serie::Decimal64(held) => Some(held.as_ref()),
            _ => None,
        }
    }

    fn narrow_mut(serie: &mut Serie) -> Option<&mut Self> {
        match serie {
            Serie::Decimal64(held) => Some(Arc::make_mut(held)),
            _ => None,
        }
    }
}

impl Leaf for Decimal128Serie {
    fn root(self) -> Serie {
        Serie::Decimal128(Arc::new(self))
    }

    fn narrow(serie: &Serie) -> Option<&Self> {
        match serie {
            Serie::Decimal128(held) => Some(held.as_ref()),
            _ => None,
        }
    }

    fn narrow_mut(serie: &mut Serie) -> Option<&mut Self> {
        match serie {
            Serie::Decimal128(held) => Some(Arc::make_mut(held)),
            _ => None,
        }
    }
}

impl Leaf for Decimal256Serie {
    fn root(self) -> Serie {
        Serie::Decimal256(Arc::new(self))
    }

    fn narrow(serie: &Serie) -> Option<&Self> {
        match serie {
            Serie::Decimal256(held) => Some(held.as_ref()),
            _ => None,
        }
    }

    fn narrow_mut(serie: &mut Serie) -> Option<&mut Self> {
        match serie {
            Serie::Decimal256(held) => Some(Arc::make_mut(held)),
            _ => None,
        }
    }
}

impl Leaf for MapSerie {
    fn root(self) -> Serie {
        match SerieValue::field(&self).dtype() {
            DataType::SortedMap { .. } => Serie::SortedMap(Arc::new(self)),
            _ => Serie::Map(Arc::new(self)),
        }
    }

    fn narrow(serie: &Serie) -> Option<&Self> {
        match serie {
            Serie::Map(held) | Serie::SortedMap(held) => Some(held.as_ref()),
            _ => None,
        }
    }

    fn narrow_mut(serie: &mut Serie) -> Option<&mut Self> {
        match serie {
            Serie::Map(held) | Serie::SortedMap(held) => Some(Arc::make_mut(held)),
            _ => None,
        }
    }
}

impl Leaf for RunEndEncodedSerie {
    fn root(self) -> Serie {
        Serie::RunEndEncoded(Arc::new(self))
    }

    fn narrow(serie: &Serie) -> Option<&Self> {
        match serie {
            Serie::RunEndEncoded(held) => Some(held.as_ref()),
            _ => None,
        }
    }

    fn narrow_mut(serie: &mut Serie) -> Option<&mut Self> {
        match serie {
            Serie::RunEndEncoded(held) => Some(Arc::make_mut(held)),
            _ => None,
        }
    }
}

impl Leaf for VariantSerie {
    fn root(self) -> Serie {
        Serie::Variant(Arc::new(self))
    }

    fn narrow(serie: &Serie) -> Option<&Self> {
        match serie {
            Serie::Variant(held) => Some(held.as_ref()),
            _ => None,
        }
    }

    fn narrow_mut(serie: &mut Serie) -> Option<&mut Self> {
        match serie {
            Serie::Variant(held) => Some(Arc::make_mut(held)),
            _ => None,
        }
    }
}

impl Leaf for BinarySerie {
    fn root(self) -> Serie {
        match SerieValue::field(&self).dtype() {
            DataType::Geometry { .. } => Serie::Geometry(Arc::new(self)),
            DataType::Geography { .. } => Serie::Geography(Arc::new(self)),
            _ => Serie::Binary(Arc::new(self)),
        }
    }

    fn narrow(serie: &Serie) -> Option<&Self> {
        match serie {
            Serie::Binary(held) | Serie::Geometry(held) | Serie::Geography(held) => {
                Some(held.as_ref())
            }
            _ => None,
        }
    }

    fn narrow_mut(serie: &mut Serie) -> Option<&mut Self> {
        match serie {
            Serie::Binary(held) | Serie::Geometry(held) | Serie::Geography(held) => {
                Some(Arc::make_mut(held))
            }
            _ => None,
        }
    }
}

impl Leaf for Time32SecondSerie {
    fn root(self) -> Serie {
        Serie::Time32Second(Arc::new(self))
    }

    fn narrow(serie: &Serie) -> Option<&Self> {
        match serie {
            Serie::Time32Second(held) => Some(held.as_ref()),
            _ => None,
        }
    }

    fn narrow_mut(serie: &mut Serie) -> Option<&mut Self> {
        match serie {
            Serie::Time32Second(held) => Some(Arc::make_mut(held)),
            _ => None,
        }
    }
}

impl Leaf for Time32MillisecondSerie {
    fn root(self) -> Serie {
        Serie::Time32Millisecond(Arc::new(self))
    }

    fn narrow(serie: &Serie) -> Option<&Self> {
        match serie {
            Serie::Time32Millisecond(held) => Some(held.as_ref()),
            _ => None,
        }
    }

    fn narrow_mut(serie: &mut Serie) -> Option<&mut Self> {
        match serie {
            Serie::Time32Millisecond(held) => Some(Arc::make_mut(held)),
            _ => None,
        }
    }
}

impl Leaf for Time64MicrosecondSerie {
    fn root(self) -> Serie {
        Serie::Time64Microsecond(Arc::new(self))
    }

    fn narrow(serie: &Serie) -> Option<&Self> {
        match serie {
            Serie::Time64Microsecond(held) => Some(held.as_ref()),
            _ => None,
        }
    }

    fn narrow_mut(serie: &mut Serie) -> Option<&mut Self> {
        match serie {
            Serie::Time64Microsecond(held) => Some(Arc::make_mut(held)),
            _ => None,
        }
    }
}

impl Leaf for Time64NanosecondSerie {
    fn root(self) -> Serie {
        Serie::Time64Nanosecond(Arc::new(self))
    }

    fn narrow(serie: &Serie) -> Option<&Self> {
        match serie {
            Serie::Time64Nanosecond(held) => Some(held.as_ref()),
            _ => None,
        }
    }

    fn narrow_mut(serie: &mut Serie) -> Option<&mut Self> {
        match serie {
            Serie::Time64Nanosecond(held) => Some(Arc::make_mut(held)),
            _ => None,
        }
    }
}

impl Leaf for DateTimeSecondSerie {
    fn root(self) -> Serie {
        Serie::DateTimeSecond(Arc::new(self))
    }

    fn narrow(serie: &Serie) -> Option<&Self> {
        match serie {
            Serie::DateTimeSecond(held) => Some(held.as_ref()),
            _ => None,
        }
    }

    fn narrow_mut(serie: &mut Serie) -> Option<&mut Self> {
        match serie {
            Serie::DateTimeSecond(held) => Some(Arc::make_mut(held)),
            _ => None,
        }
    }
}

impl Leaf for DateTimeMillisecondSerie {
    fn root(self) -> Serie {
        Serie::DateTimeMillisecond(Arc::new(self))
    }

    fn narrow(serie: &Serie) -> Option<&Self> {
        match serie {
            Serie::DateTimeMillisecond(held) => Some(held.as_ref()),
            _ => None,
        }
    }

    fn narrow_mut(serie: &mut Serie) -> Option<&mut Self> {
        match serie {
            Serie::DateTimeMillisecond(held) => Some(Arc::make_mut(held)),
            _ => None,
        }
    }
}

impl Leaf for DateTimeMicrosecondSerie {
    fn root(self) -> Serie {
        Serie::DateTimeMicrosecond(Arc::new(self))
    }

    fn narrow(serie: &Serie) -> Option<&Self> {
        match serie {
            Serie::DateTimeMicrosecond(held) => Some(held.as_ref()),
            _ => None,
        }
    }

    fn narrow_mut(serie: &mut Serie) -> Option<&mut Self> {
        match serie {
            Serie::DateTimeMicrosecond(held) => Some(Arc::make_mut(held)),
            _ => None,
        }
    }
}

impl Leaf for DateTimeNanosecondSerie {
    fn root(self) -> Serie {
        Serie::DateTimeNanosecond(Arc::new(self))
    }

    fn narrow(serie: &Serie) -> Option<&Self> {
        match serie {
            Serie::DateTimeNanosecond(held) => Some(held.as_ref()),
            _ => None,
        }
    }

    fn narrow_mut(serie: &mut Serie) -> Option<&mut Self> {
        match serie {
            Serie::DateTimeNanosecond(held) => Some(Arc::make_mut(held)),
            _ => None,
        }
    }
}

impl Leaf for DurationSecondSerie {
    fn root(self) -> Serie {
        Serie::DurationSecond(Arc::new(self))
    }

    fn narrow(serie: &Serie) -> Option<&Self> {
        match serie {
            Serie::DurationSecond(held) => Some(held.as_ref()),
            _ => None,
        }
    }

    fn narrow_mut(serie: &mut Serie) -> Option<&mut Self> {
        match serie {
            Serie::DurationSecond(held) => Some(Arc::make_mut(held)),
            _ => None,
        }
    }
}

impl Leaf for DurationMillisecondSerie {
    fn root(self) -> Serie {
        Serie::DurationMillisecond(Arc::new(self))
    }

    fn narrow(serie: &Serie) -> Option<&Self> {
        match serie {
            Serie::DurationMillisecond(held) => Some(held.as_ref()),
            _ => None,
        }
    }

    fn narrow_mut(serie: &mut Serie) -> Option<&mut Self> {
        match serie {
            Serie::DurationMillisecond(held) => Some(Arc::make_mut(held)),
            _ => None,
        }
    }
}

impl Leaf for DurationMicrosecondSerie {
    fn root(self) -> Serie {
        Serie::DurationMicrosecond(Arc::new(self))
    }

    fn narrow(serie: &Serie) -> Option<&Self> {
        match serie {
            Serie::DurationMicrosecond(held) => Some(held.as_ref()),
            _ => None,
        }
    }

    fn narrow_mut(serie: &mut Serie) -> Option<&mut Self> {
        match serie {
            Serie::DurationMicrosecond(held) => Some(Arc::make_mut(held)),
            _ => None,
        }
    }
}

impl Leaf for DurationNanosecondSerie {
    fn root(self) -> Serie {
        Serie::DurationNanosecond(Arc::new(self))
    }

    fn narrow(serie: &Serie) -> Option<&Self> {
        match serie {
            Serie::DurationNanosecond(held) => Some(held.as_ref()),
            _ => None,
        }
    }

    fn narrow_mut(serie: &mut Serie) -> Option<&mut Self> {
        match serie {
            Serie::DurationNanosecond(held) => Some(Arc::make_mut(held)),
            _ => None,
        }
    }
}

impl Leaf for IntervalYearMonthSerie {
    fn root(self) -> Serie {
        Serie::IntervalYearMonth(Arc::new(self))
    }

    fn narrow(serie: &Serie) -> Option<&Self> {
        match serie {
            Serie::IntervalYearMonth(held) => Some(held.as_ref()),
            _ => None,
        }
    }

    fn narrow_mut(serie: &mut Serie) -> Option<&mut Self> {
        match serie {
            Serie::IntervalYearMonth(held) => Some(Arc::make_mut(held)),
            _ => None,
        }
    }
}

impl Leaf for IntervalDayTimeSerie {
    fn root(self) -> Serie {
        Serie::IntervalDayTime(Arc::new(self))
    }

    fn narrow(serie: &Serie) -> Option<&Self> {
        match serie {
            Serie::IntervalDayTime(held) => Some(held.as_ref()),
            _ => None,
        }
    }

    fn narrow_mut(serie: &mut Serie) -> Option<&mut Self> {
        match serie {
            Serie::IntervalDayTime(held) => Some(Arc::make_mut(held)),
            _ => None,
        }
    }
}

impl Leaf for IntervalMonthDayNanoSerie {
    fn root(self) -> Serie {
        Serie::IntervalMonthDayNano(Arc::new(self))
    }

    fn narrow(serie: &Serie) -> Option<&Self> {
        match serie {
            Serie::IntervalMonthDayNano(held) => Some(held.as_ref()),
            _ => None,
        }
    }

    fn narrow_mut(serie: &mut Serie) -> Option<&mut Self> {
        match serie {
            Serie::IntervalMonthDayNano(held) => Some(Arc::make_mut(held)),
            _ => None,
        }
    }
}

impl Leaf for LargeUtf8StringSerie {
    fn root(self) -> Serie {
        Serie::LargeUtf8String(Arc::new(self))
    }

    fn narrow(serie: &Serie) -> Option<&Self> {
        match serie {
            Serie::LargeUtf8String(held) => Some(held.as_ref()),
            _ => None,
        }
    }

    fn narrow_mut(serie: &mut Serie) -> Option<&mut Self> {
        match serie {
            Serie::LargeUtf8String(held) => Some(Arc::make_mut(held)),
            _ => None,
        }
    }
}

impl Leaf for Utf8ViewStringSerie {
    fn root(self) -> Serie {
        Serie::Utf8ViewString(Arc::new(self))
    }

    fn narrow(serie: &Serie) -> Option<&Self> {
        match serie {
            Serie::Utf8ViewString(held) => Some(held.as_ref()),
            _ => None,
        }
    }

    fn narrow_mut(serie: &mut Serie) -> Option<&mut Self> {
        match serie {
            Serie::Utf8ViewString(held) => Some(Arc::make_mut(held)),
            _ => None,
        }
    }
}

impl Leaf for BinaryStringSerie {
    fn root(self) -> Serie {
        Serie::BinaryString(Arc::new(self))
    }

    fn narrow(serie: &Serie) -> Option<&Self> {
        match serie {
            Serie::BinaryString(held) => Some(held.as_ref()),
            _ => None,
        }
    }

    fn narrow_mut(serie: &mut Serie) -> Option<&mut Self> {
        match serie {
            Serie::BinaryString(held) => Some(Arc::make_mut(held)),
            _ => None,
        }
    }
}

impl Leaf for LargeBinaryStringSerie {
    fn root(self) -> Serie {
        Serie::LargeBinaryString(Arc::new(self))
    }

    fn narrow(serie: &Serie) -> Option<&Self> {
        match serie {
            Serie::LargeBinaryString(held) => Some(held.as_ref()),
            _ => None,
        }
    }

    fn narrow_mut(serie: &mut Serie) -> Option<&mut Self> {
        match serie {
            Serie::LargeBinaryString(held) => Some(Arc::make_mut(held)),
            _ => None,
        }
    }
}

impl Leaf for BinaryViewStringSerie {
    fn root(self) -> Serie {
        Serie::BinaryViewString(Arc::new(self))
    }

    fn narrow(serie: &Serie) -> Option<&Self> {
        match serie {
            Serie::BinaryViewString(held) => Some(held.as_ref()),
            _ => None,
        }
    }

    fn narrow_mut(serie: &mut Serie) -> Option<&mut Self> {
        match serie {
            Serie::BinaryViewString(held) => Some(Arc::make_mut(held)),
            _ => None,
        }
    }
}

impl Leaf for FixedStringSerie {
    fn root(self) -> Serie {
        Serie::FixedString(Arc::new(self))
    }

    fn narrow(serie: &Serie) -> Option<&Self> {
        match serie {
            Serie::FixedString(held) => Some(held.as_ref()),
            _ => None,
        }
    }

    fn narrow_mut(serie: &mut Serie) -> Option<&mut Self> {
        match serie {
            Serie::FixedString(held) => Some(Arc::make_mut(held)),
            _ => None,
        }
    }
}

impl Leaf for LargeBinarySerie {
    fn root(self) -> Serie {
        Serie::LargeBinary(Arc::new(self))
    }

    fn narrow(serie: &Serie) -> Option<&Self> {
        match serie {
            Serie::LargeBinary(held) => Some(held.as_ref()),
            _ => None,
        }
    }

    fn narrow_mut(serie: &mut Serie) -> Option<&mut Self> {
        match serie {
            Serie::LargeBinary(held) => Some(Arc::make_mut(held)),
            _ => None,
        }
    }
}

impl Leaf for BinaryViewSerie {
    fn root(self) -> Serie {
        Serie::BinaryView(Arc::new(self))
    }

    fn narrow(serie: &Serie) -> Option<&Self> {
        match serie {
            Serie::BinaryView(held) => Some(held.as_ref()),
            _ => None,
        }
    }

    fn narrow_mut(serie: &mut Serie) -> Option<&mut Self> {
        match serie {
            Serie::BinaryView(held) => Some(Arc::make_mut(held)),
            _ => None,
        }
    }
}

/// The path a refusal names for a run, which has no field to name.
const RUN_PATH: &str = "$";

/// Replace `range` of a run by `rows`: `Arc<[Scalar]>` cannot grow in
/// place, so the run is copied once.
fn splice_run(run: &mut Run, range: Range<usize>, rows: Vec<Scalar>) {
    let mut values = run.as_slice().to_vec();
    values.splice(range, rows);
    *run = Run::new(values);
}

impl fmt::Debug for Serie {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        column!(
            self,
            run => fmt::Debug::fmt(run, formatter),
            column => fmt::Debug::fmt(column.as_ref(), formatter)
        )
    }
}

impl fmt::Display for Serie {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        column!(
            self,
            run => fmt::Display::fmt(run, formatter),
            column => display_column(column.as_ref(), formatter)
        )
    }
}

impl Serie {
    /// Construct a schema-free ordered run.
    pub fn new(values: impl Into<Arc<[Scalar]>>) -> Self {
        Self::Run(Run::new(values))
    }

    /// Return the field every row is typed by, or `None` for a run.
    ///
    /// A run is schema free: its rows are whatever they are, and its
    /// datatype is agreed back out of them rather than read off a field.
    pub fn field(&self) -> Option<&Field> {
        column!(
            self,
            _run => None,
            column => Some(SerieValue::field(column.as_ref()))
        )
    }

    /// Return the shared field, or `None` for a run.
    pub fn field_ref(&self) -> Option<&Arc<Field>> {
        column!(
            self,
            _run => None,
            column => Some(SerieValue::field_ref(column.as_ref()))
        )
    }

    /// The field this serie's rows are typed by, refused by name for a run.
    ///
    /// A run declares no field, so it cannot stand where a column must -
    /// under a record's child, a sequence's item, a mapping's entry. The
    /// refusal says that rather than inventing a field for it.
    ///
    /// # Errors
    ///
    /// Returns an error naming the run where this is one.
    pub fn require_field(&self) -> Result<&Field> {
        self.field().ok_or_else(|| crate::Error::InvalidRecord {
            path: smol_str::SmolStr::new_static(RUN_PATH),
            reason: smol_str::SmolStr::new_static("a schema-free run declares no field"),
        })
    }

    /// The path a refusal names: the field's name, or `$` for a run.
    fn name(&self) -> &str {
        self.field().map_or(RUN_PATH, Field::name)
    }

    /// Return the number of rows.
    ///
    /// Constant for either leaf: a column knows its length without reading a
    /// row.
    pub fn len(&self) -> usize {
        column!(
            self,
            run => run.as_slice().len(),
            column => SerieValue::len(column.as_ref())
        )
    }

    /// Return whether this serie holds no rows.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Return how many rows hold no value.
    ///
    /// Constant for a column, which keeps the count on its validity bitmap
    /// (an encoding counts its logical nulls once). A run keeps no such
    /// bitmap, so it walks its values - the one ask where the two leaves
    /// differ in cost rather than in answer.
    pub fn null_count(&self) -> usize {
        column!(
            self,
            run => run.as_slice().iter().filter(|value| value.is_null()).count(),
            column => SerieValue::null_count(column.as_ref())
        )
    }

    /// Return whether row `index` holds no value.
    ///
    /// # Errors
    ///
    /// Returns an error naming the serie and both counts when `index` is
    /// past the end.
    pub fn is_null(&self, index: usize) -> Result<bool> {
        column!(
            self,
            run => {
                require_row(RUN_PATH, index, run.as_slice().len())?;
                Ok(run.as_slice()[index].is_null())
            },
            column => SerieValue::is_null(column.as_ref(), index)
        )
    }

    /// Return row `index` as a value.
    ///
    /// A run lends what it holds and clones it; a column builds the row from
    /// its buffers and keeps nothing. A stored row is one its field accepts,
    /// so the only refusal is an index past the end.
    ///
    /// # Errors
    ///
    /// Returns an error naming the serie and both counts when `index` is
    /// past the end.
    pub fn scalar(&self, index: usize) -> Result<Scalar> {
        column!(
            self,
            run => {
                require_row(RUN_PATH, index, run.as_slice().len())?;
                Ok(run.as_slice()[index].clone())
            },
            column => SerieValue::scalar(column.as_ref(), index)
        )
    }

    /// Look up row `index`, or `None` past the end.
    ///
    /// Borrowed for a run, built for a column: the one lookup that allocates
    /// on a column, by contract.
    pub fn get(&self, index: usize) -> Option<Cow<'_, Scalar>> {
        match self {
            Self::Run(run) => run.as_slice().get(index).map(Cow::Borrowed),
            column => (index < column.len()).then(|| Cow::Owned(proven_row(column, index))),
        }
    }

    /// Return the ordered values of whichever leaf this is.
    ///
    /// A run lends what it holds; a column builds its rows, one at a time,
    /// and hands them over owned. Nothing is kept either way, so a caller
    /// reading a column twice reads it twice - hold the answer rather than
    /// asking again.
    pub fn rows(&self) -> Cow<'_, [Scalar]> {
        match self {
            Self::Run(run) => Cow::Borrowed(run.as_slice()),
            column => Cow::Owned(
                (0..column.len())
                    .map(|index| proven_row(column, index))
                    .collect(),
            ),
        }
    }

    /// Walk the rows: lent for a run, built one at a time for a column.
    pub fn iter(&self) -> Children<'_> {
        NestedValue::children(self)
    }

    /// Return the datatype this serie materializes into.
    ///
    /// A column carries its field, so this is a read: a serie of the item
    /// field, named `item` exactly as a held array spells it, with the
    /// field's declared nullability. A run has its item agreed back out of
    /// its rows, and an empty run names a nullable null item.
    ///
    /// # Errors
    ///
    /// Returns an error where a run's rows do not agree on one item
    /// datatype.
    pub fn dtype(&self) -> Result<DataType> {
        match self.field() {
            Some(field) => Ok(DataType::serie(field.clone().with_name("item"))),
            None => Scalar::Serie(self.clone()).dtype(),
        }
    }

    /// Return the schema-free run when this is that leaf.
    pub fn as_run(&self) -> Option<&Run> {
        match self {
            Self::Run(run) => Some(run),
            _ => None,
        }
    }

    /// Borrow the ordered values where the leaf holds them.
    ///
    /// A run does and always will. A column holds Arrow buffers and no
    /// [`Scalar`], so it has none to lend and answers `None`; [`Self::rows`]
    /// is the door that reads it.
    pub fn as_slice(&self) -> Option<&[Scalar]> {
        self.as_run().map(Run::as_slice)
    }

    /// Return whether this serie is a column rather than a schema-free run.
    pub const fn is_column(&self) -> bool {
        !matches!(self, Self::Run(_))
    }

    /// Whether this is one of the seven text storage layouts, whichever
    /// string leaf its field declares - never a code, a version, a URI, a
    /// zone or a MIME or media type, which hold the same buffers under a
    /// variant of their own.
    pub(crate) const fn is_string_storage(&self) -> bool {
        matches!(
            self,
            Self::Utf8String(_)
                | Self::LargeUtf8String(_)
                | Self::Utf8ViewString(_)
                | Self::BinaryString(_)
                | Self::LargeBinaryString(_)
                | Self::BinaryViewString(_)
                | Self::FixedString(_)
        )
    }

    /// Whether this is one of the four byte storage layouts - never a UUID
    /// or a geospatial reading, which hold the same buffers under a variant
    /// of their own.
    pub(crate) const fn is_byte_storage(&self) -> bool {
        matches!(
            self,
            Self::Binary(_) | Self::LargeBinary(_) | Self::BinaryView(_) | Self::FixedBytes(_)
        )
    }

    /// Borrow row `index`'s bytes where a text or byte storage leaf holds
    /// them - in the payload an offset run points into, in a view's buffer,
    /// or in a fixed slot - with no value built: `None` where the row is
    /// absent or past the end, and for every other variant.
    ///
    /// The bytes are the storage as it stands: text in the charset its field
    /// declares, and a fixed slot with its padding - not what
    /// [`Scalar::as_value_bytes`] answers, which decodes them. A caller
    /// first knows the variant is a text or byte storage leaf - through
    /// [`Self::is_string_storage`] / [`Self::is_byte_storage`] or by matching
    /// those variants - because a `None` here does not say which it was.
    pub(crate) fn value_bytes(&self, index: usize) -> Option<&[u8]> {
        match self {
            Self::Utf8String(column) => column.value(index).map(str::as_bytes),
            Self::LargeUtf8String(column) => column.value(index).map(str::as_bytes),
            Self::Utf8ViewString(column) => column.value(index).map(str::as_bytes),
            Self::BinaryString(column) => column.value(index),
            Self::LargeBinaryString(column) => column.value(index),
            Self::BinaryViewString(column) => column.value(index),
            Self::FixedString(column) => column.value(index),
            Self::Binary(column) => column.value(index),
            Self::LargeBinary(column) => column.value(index),
            Self::BinaryView(column) => column.value(index),
            Self::FixedBytes(column) => column.value(index),
            _ => None,
        }
    }

    /// Return the rows as the schema-free run they are.
    ///
    /// The field is what a column has that a run has not, so this is the one
    /// direction that drops something and it is spelled rather than implied.
    /// A run answers itself; a column builds its rows once.
    pub fn into_run(self) -> Run {
        match self {
            Self::Run(run) => run,
            column => Run::new(column.rows().into_owned()),
        }
    }

    /// Return every row's buffers as one Arrow array, or `None` for a run.
    ///
    /// The buffers are shared, never copied. A run declares no field, so it
    /// names no Arrow layout; lay one out with [`Self::from_scalars`] under
    /// the field it should have.
    pub fn into_arrow_array(&self) -> Option<ArrayRef> {
        column!(
            self,
            _run => None,
            column => Some(SerieValue::into_arrow_array(column.as_ref()))
        )
    }

    /// This serie's buffers, refused by name for a run.
    ///
    /// # Errors
    ///
    /// [`Self::require_field`] carries the rule.
    pub fn require_arrow_array(&self) -> Result<ArrayRef> {
        self.require_field()?;
        self.into_arrow_array()
            .ok_or_else(|| crate::Error::InvalidRecord {
                path: smol_str::SmolStr::new_static(RUN_PATH),
                reason: smol_str::SmolStr::new_static("a schema-free run names no Arrow layout"),
            })
    }

    /// Return the window `offset..offset + length`.
    ///
    /// Zero copy for a column: an Arrow slice, a nested column slicing its
    /// validity and its children to the reached window with its offsets
    /// rebased. A run copies its window into a new run.
    ///
    /// # Errors
    ///
    /// Returns an error naming the serie and both counts when the window
    /// reaches past the end.
    pub fn slice(&self, offset: usize, length: usize) -> Result<Self> {
        column!(
            self,
            run => {
                require_window(RUN_PATH, offset, length, run.as_slice().len())?;
                Ok(Self::new(&run.as_slice()[offset..offset + length]))
            },
            column => SerieValue::slice(column.as_ref(), offset, length).map(SerieValue::into_serie)
        )
    }

    /// Borrow a record column's child by exact name; `None` elsewhere.
    pub fn child(&self, name: &str) -> Option<&Self> {
        match self {
            Self::Struct(column) => column.child(name),
            _ => None,
        }
    }

    /// Borrow child `index` of a record or a union column; `None` elsewhere.
    pub fn child_at(&self, index: usize) -> Option<&Self> {
        self.children().get(index)
    }

    /// Borrow a record column's children, or a union column's; empty
    /// elsewhere.
    pub fn children(&self) -> &[Self] {
        match self {
            Self::Struct(column) => column.children(),
            Self::Union(column) => column.children(),
            _ => &[],
        }
    }

    /// Borrow the column under this one: a sequence column's items, a
    /// mapping column's entries, an encoding's values; `None` elsewhere.
    pub fn items(&self) -> Option<&Self> {
        match self {
            Self::Serie(column) => Some(column.items()),
            Self::SerieView(column) => Some(column.items()),
            Self::FixedSizeSerie(column) => Some(column.items()),
            Self::LargeSerie(column) => Some(column.items()),
            Self::LargeSerieView(column) => Some(column.items()),
            Self::Map(column) | Self::SortedMap(column) => Some(column.entries()),
            Self::Dictionary(column) => Some(column.values()),
            Self::RunEndEncoded(column) => Some(column.values()),
            _ => None,
        }
    }

    /// Borrow the column a schema path reaches, with exactly the segment
    /// semantics of [`DataType::get_field_by_path`].
    ///
    /// A named segment is a record child, or a union member, by exact name;
    /// a sequence column is transparent to its item, so `orders.price`
    /// reaches the price of an `array<struct>` item the way
    /// `orders.item.price` does; a mapping column is addressed through its
    /// entries field. An index, key, range or predicate segment reaches no
    /// column.
    pub fn get_child_by_path(&self, path: &FieldPath) -> Option<&Self> {
        self.walk(path.segments())
    }

    /// The one borrowed walk a schema address takes over columns.
    fn walk(&self, segments: &[FieldSegment]) -> Option<&Self> {
        let (segment, rest) = segments.split_first()?;
        let FieldSegment::Field(name) = segment else {
            return None;
        };
        let child = match self {
            Self::Struct(column) => column.child(name)?,
            Self::Union(column) => {
                let DataType::Union(members, _) = column.field().dtype() else {
                    return None;
                };
                let (type_id, _) = members.get_by_name(name)?;
                column.child_of(type_id)?
            }
            Self::Map(column) | Self::SortedMap(column) => {
                let entries = column.entries();
                (entries.field()?.name() == name).then_some(entries)?
            }
            Self::Serie(_)
            | Self::SerieView(_)
            | Self::FixedSizeSerie(_)
            | Self::LargeSerie(_)
            | Self::LargeSerieView(_) => {
                let items = self.items()?;
                if items.field()?.name() == name {
                    items
                } else {
                    // Reading sees through a serie item, the way the schema
                    // walk does.
                    return items.walk(segments);
                }
            }
            _ => return None,
        };
        if rest.is_empty() {
            Some(child)
        } else {
            child.walk(rest)
        }
    }

    // --------------------------------------------------------------------
    // Writes: one mutation, `splice`, and the rest spelled over it. Every
    // write fails atomically - a column proves, checks, then writes without
    // failing; a run accepts every value because it declares none.
    // --------------------------------------------------------------------

    /// Replace `range` by `rows`: the one mutation every other write is
    /// spelled over.
    ///
    /// A column proves every row through its field's contract once, checks
    /// what a write could not do, and writes its buffers in place when it
    /// holds them alone (copied once when it does not). A run is one shared
    /// slice, so a write copies every value it holds.
    ///
    /// # Errors
    ///
    /// Returns an error naming the serie when `range` is reversed or reaches
    /// past the end, or [`SerieValue::splice`]'s refusal for a column.
    pub fn splice(&mut self, range: Range<usize>, rows: Vec<Scalar>) -> Result<()> {
        column_mut!(
            self,
            run => {
                require_range(RUN_PATH, &range, run.as_slice().len())?;
                splice_run(run, range, rows);
                Ok(())
            },
            column => SerieValue::splice(column, range, rows)
        )
    }

    /// Overwrite row `index`, through the field's contract where there is
    /// one.
    ///
    /// A primitive or boolean column overwrites one slot in place.
    ///
    /// # Errors
    ///
    /// Returns an error when `index` is past the end, or [`Self::splice`]'s
    /// refusal.
    pub fn set(&mut self, index: usize, value: Scalar) -> Result<()> {
        require_row(self.name(), index, self.len())?;
        self.splice(index..index + 1, vec![value])
    }

    /// Append one row, through the field's contract where there is one.
    ///
    /// A column writes its buffers, amortized. A run is one shared slice, so
    /// appending to it copies every value it holds - building a run one
    /// push at a time is quadratic, and [`Scalar::from_sequence`] or
    /// [`Self::new`] is what builds one from values already in hand.
    ///
    /// # Errors
    ///
    /// [`Self::splice`] carries the rule.
    pub fn push(&mut self, value: Scalar) -> Result<()> {
        let len = self.len();
        self.splice(len..len, vec![value])
    }

    /// Insert one row before `index`, which may be `len`.
    ///
    /// # Errors
    ///
    /// Returns an error when `index` is past `len`, or [`Self::splice`]'s
    /// refusal.
    pub fn insert(&mut self, index: usize, value: Scalar) -> Result<()> {
        require_range(self.name(), &(index..index), self.len())?;
        self.splice(index..index, vec![value])
    }

    /// Remove row `index` and answer it.
    ///
    /// # Errors
    ///
    /// Returns an error when `index` is past the end.
    pub fn remove(&mut self, index: usize) -> Result<Scalar> {
        let row = self.scalar(index)?;
        self.splice(index..index + 1, Vec::new())?;
        Ok(row)
    }

    /// Remove the last row and answer it, or `None` when empty.
    ///
    /// # Errors
    ///
    /// [`Self::splice`] carries the rule.
    pub fn pop(&mut self) -> Result<Option<Scalar>> {
        match self.len().checked_sub(1) {
            Some(last) => self.remove(last).map(Some),
            None => Ok(None),
        }
    }

    /// Drop every row from `len` on; a no-op when the serie is no longer.
    ///
    /// # Errors
    ///
    /// [`Self::splice`] carries the rule.
    pub fn truncate(&mut self, len: usize) -> Result<()> {
        let held = self.len();
        if len >= held {
            return Ok(());
        }
        self.splice(len..held, Vec::new())
    }

    /// Drop every row and keep the field.
    ///
    /// # Errors
    ///
    /// [`Self::splice`] carries the rule.
    pub fn clear(&mut self) -> Result<()> {
        self.truncate(0)
    }

    /// Append `rows`, each through the field's contract where there is one.
    ///
    /// # Errors
    ///
    /// [`Self::splice`] carries the rule.
    pub fn extend(&mut self, rows: Vec<Scalar>) -> Result<()> {
        let len = self.len();
        self.splice(len..len, rows)
    }

    /// Append every row of `other`.
    ///
    /// Two columns whose fields have one datatype, where this one is
    /// nullable or `other` holds no absent row, append buffer to buffer -
    /// the other column's rows are already proven - and no row is read.
    /// Anything else reads `other`'s rows and appends them through
    /// [`Self::extend`].
    ///
    /// # Errors
    ///
    /// [`Self::splice`] carries the rule.
    pub fn extend_from_serie(&mut self, other: &Self) -> Result<()> {
        let agreed = match (self.field(), other.field()) {
            (Some(mine), Some(theirs)) => {
                mine.dtype() == theirs.dtype() && (mine.is_nullable() || other.null_count() == 0)
            }
            _ => false,
        };
        if agreed && self.append(other) {
            return Ok(());
        }
        self.extend(other.rows().into_owned())
    }

    /// Append `other`'s buffers where the two hold one layout and the
    /// layout reaches the total, answering whether they did; the caller has
    /// agreed the fields, and `false` leaves this column as it was.
    ///
    /// A variant is a layout, not a datatype - both duration widths share
    /// one leaf per unit - so the agreed fields are what keep a `duration32`
    /// column from taking a `duration64` one's counts: this pairs variants
    /// only.
    pub(crate) fn append(&mut self, other: &Self) -> bool {
        match (self, other) {
            (Self::Null(mine), Self::Null(theirs)) => Arc::make_mut(mine).append(theirs),
            (Self::Boolean(mine), Self::Boolean(theirs)) => Arc::make_mut(mine).append(theirs),
            (Self::Int8(mine), Self::Int8(theirs)) => Arc::make_mut(mine).append(theirs),
            (Self::Int16(mine), Self::Int16(theirs)) => Arc::make_mut(mine).append(theirs),
            (Self::Int32(mine), Self::Int32(theirs)) => Arc::make_mut(mine).append(theirs),
            (Self::Int64(mine), Self::Int64(theirs)) => Arc::make_mut(mine).append(theirs),
            (Self::UInt8(mine), Self::UInt8(theirs)) => Arc::make_mut(mine).append(theirs),
            (Self::UInt16(mine), Self::UInt16(theirs)) => Arc::make_mut(mine).append(theirs),
            (Self::UInt32(mine), Self::UInt32(theirs)) => Arc::make_mut(mine).append(theirs),
            (Self::UInt64(mine), Self::UInt64(theirs)) => Arc::make_mut(mine).append(theirs),
            (Self::Float16(mine), Self::Float16(theirs)) => Arc::make_mut(mine).append(theirs),
            (Self::Float32(mine), Self::Float32(theirs)) => Arc::make_mut(mine).append(theirs),
            (Self::Float64(mine), Self::Float64(theirs)) => Arc::make_mut(mine).append(theirs),
            (Self::DateTimeSecond(mine), Self::DateTimeSecond(theirs)) => {
                Arc::make_mut(mine).append(theirs)
            }
            (Self::DateTimeMillisecond(mine), Self::DateTimeMillisecond(theirs)) => {
                Arc::make_mut(mine).append(theirs)
            }
            (Self::DateTimeMicrosecond(mine), Self::DateTimeMicrosecond(theirs)) => {
                Arc::make_mut(mine).append(theirs)
            }
            (Self::DateTimeNanosecond(mine), Self::DateTimeNanosecond(theirs)) => {
                Arc::make_mut(mine).append(theirs)
            }
            (Self::Date32(mine), Self::Date32(theirs)) => Arc::make_mut(mine).append(theirs),
            (Self::Date64(mine), Self::Date64(theirs)) => Arc::make_mut(mine).append(theirs),
            (Self::Time32Second(mine), Self::Time32Second(theirs)) => {
                Arc::make_mut(mine).append(theirs)
            }
            (Self::Time32Millisecond(mine), Self::Time32Millisecond(theirs)) => {
                Arc::make_mut(mine).append(theirs)
            }
            (Self::Time64Microsecond(mine), Self::Time64Microsecond(theirs)) => {
                Arc::make_mut(mine).append(theirs)
            }
            (Self::Time64Nanosecond(mine), Self::Time64Nanosecond(theirs)) => {
                Arc::make_mut(mine).append(theirs)
            }
            (Self::DurationSecond(mine), Self::DurationSecond(theirs)) => {
                Arc::make_mut(mine).append(theirs)
            }
            (Self::DurationMillisecond(mine), Self::DurationMillisecond(theirs)) => {
                Arc::make_mut(mine).append(theirs)
            }
            (Self::DurationMicrosecond(mine), Self::DurationMicrosecond(theirs)) => {
                Arc::make_mut(mine).append(theirs)
            }
            (Self::DurationNanosecond(mine), Self::DurationNanosecond(theirs)) => {
                Arc::make_mut(mine).append(theirs)
            }
            (Self::IntervalYearMonth(mine), Self::IntervalYearMonth(theirs)) => {
                Arc::make_mut(mine).append(theirs)
            }
            (Self::IntervalDayTime(mine), Self::IntervalDayTime(theirs)) => {
                Arc::make_mut(mine).append(theirs)
            }
            (Self::IntervalMonthDayNano(mine), Self::IntervalMonthDayNano(theirs)) => {
                Arc::make_mut(mine).append(theirs)
            }
            (Self::Binary(mine), Self::Binary(theirs)) => Arc::make_mut(mine).append(theirs),
            (Self::LargeBinary(mine), Self::LargeBinary(theirs)) => {
                Arc::make_mut(mine).append(theirs)
            }
            (Self::BinaryView(mine), Self::BinaryView(theirs)) => {
                Arc::make_mut(mine).append(theirs)
            }
            (Self::FixedBytes(mine), Self::FixedBytes(theirs)) => {
                Arc::make_mut(mine).append(theirs)
            }
            (Self::Utf8String(mine), Self::Utf8String(theirs)) => {
                Arc::make_mut(mine).append(theirs)
            }
            (Self::LargeUtf8String(mine), Self::LargeUtf8String(theirs)) => {
                Arc::make_mut(mine).append(theirs)
            }
            (Self::Utf8ViewString(mine), Self::Utf8ViewString(theirs)) => {
                Arc::make_mut(mine).append(theirs)
            }
            (Self::BinaryString(mine), Self::BinaryString(theirs)) => {
                Arc::make_mut(mine).append(theirs)
            }
            (Self::LargeBinaryString(mine), Self::LargeBinaryString(theirs)) => {
                Arc::make_mut(mine).append(theirs)
            }
            (Self::BinaryViewString(mine), Self::BinaryViewString(theirs)) => {
                Arc::make_mut(mine).append(theirs)
            }
            (Self::FixedString(mine), Self::FixedString(theirs)) => {
                Arc::make_mut(mine).append(theirs)
            }
            (Self::Country(mine), Self::Country(theirs)) => Arc::make_mut(mine).append(theirs),
            (Self::Ccy(mine), Self::Ccy(theirs)) => Arc::make_mut(mine).append(theirs),
            (Self::Mic(mine), Self::Mic(theirs)) => Arc::make_mut(mine).append(theirs),
            (Self::Cfi(mine), Self::Cfi(theirs)) => Arc::make_mut(mine).append(theirs),
            (Self::Isin(mine), Self::Isin(theirs)) => Arc::make_mut(mine).append(theirs),
            (Self::Side(mine), Self::Side(theirs)) => Arc::make_mut(mine).append(theirs),
            (Self::State(mine), Self::State(theirs)) => Arc::make_mut(mine).append(theirs),
            (Self::TimeInForce(mine), Self::TimeInForce(theirs)) => {
                Arc::make_mut(mine).append(theirs)
            }
            (Self::Version(mine), Self::Version(theirs)) => Arc::make_mut(mine).append(theirs),
            (Self::Url(mine), Self::Url(theirs)) => Arc::make_mut(mine).append(theirs),
            (Self::Urn(mine), Self::Urn(theirs)) => Arc::make_mut(mine).append(theirs),
            (Self::Timezone(mine), Self::Timezone(theirs)) => Arc::make_mut(mine).append(theirs),
            (Self::MimeType(mine), Self::MimeType(theirs)) => Arc::make_mut(mine).append(theirs),
            (Self::MediaType(mine), Self::MediaType(theirs)) => Arc::make_mut(mine).append(theirs),
            (Self::Cusip(mine), Self::Cusip(theirs)) => Arc::make_mut(mine).append(theirs),
            (Self::Sedol(mine), Self::Sedol(theirs)) => Arc::make_mut(mine).append(theirs),
            (Self::Bbg(mine), Self::Bbg(theirs)) => Arc::make_mut(mine).append(theirs),
            (Self::Ric(mine), Self::Ric(theirs)) => Arc::make_mut(mine).append(theirs),
            (Self::Figi(mine), Self::Figi(theirs)) => Arc::make_mut(mine).append(theirs),
            (Self::Unit(mine), Self::Unit(theirs)) => Arc::make_mut(mine).append(theirs),
            (Self::Uuid(mine), Self::Uuid(theirs)) => Arc::make_mut(mine).append(theirs),
            (Self::Serie(mine), Self::Serie(theirs)) => Arc::make_mut(mine).append(theirs),
            (Self::SerieView(mine), Self::SerieView(theirs)) => Arc::make_mut(mine).append(theirs),
            (Self::FixedSizeSerie(mine), Self::FixedSizeSerie(theirs)) => {
                Arc::make_mut(mine).append(theirs)
            }
            (Self::LargeSerie(mine), Self::LargeSerie(theirs)) => {
                Arc::make_mut(mine).append(theirs)
            }
            (Self::LargeSerieView(mine), Self::LargeSerieView(theirs)) => {
                Arc::make_mut(mine).append(theirs)
            }
            (Self::Struct(mine), Self::Struct(theirs)) => Arc::make_mut(mine).append(theirs),
            (Self::Union(mine), Self::Union(theirs)) => Arc::make_mut(mine).append(theirs),
            (Self::Dictionary(mine), Self::Dictionary(theirs)) => {
                Arc::make_mut(mine).append(theirs)
            }
            (Self::Decimal32(mine), Self::Decimal32(theirs)) => Arc::make_mut(mine).append(theirs),
            (Self::Decimal64(mine), Self::Decimal64(theirs)) => Arc::make_mut(mine).append(theirs),
            (Self::Decimal128(mine), Self::Decimal128(theirs)) => {
                Arc::make_mut(mine).append(theirs)
            }
            (Self::Decimal256(mine), Self::Decimal256(theirs)) => {
                Arc::make_mut(mine).append(theirs)
            }
            (Self::Map(mine), Self::Map(theirs)) => Arc::make_mut(mine).append(theirs),
            (Self::SortedMap(mine), Self::SortedMap(theirs)) => Arc::make_mut(mine).append(theirs),
            (Self::RunEndEncoded(mine), Self::RunEndEncoded(theirs)) => {
                Arc::make_mut(mine).append(theirs)
            }
            (Self::Variant(mine), Self::Variant(theirs)) => Arc::make_mut(mine).append(theirs),
            (Self::Geometry(mine), Self::Geometry(theirs)) => Arc::make_mut(mine).append(theirs),
            (Self::Geography(mine), Self::Geography(theirs)) => Arc::make_mut(mine).append(theirs),
            _ => false,
        }
    }

    /// Truncate to `len`, or grow to it with clones of `value`.
    ///
    /// `value` is proved once, and the clones are written without a second
    /// proof.
    ///
    /// # Errors
    ///
    /// Returns an error when `value` is not one the field accepts, or when
    /// the layout cannot hold the total the growth would reach.
    pub fn resize(&mut self, len: usize, value: Scalar) -> Result<()> {
        let held = self.len();
        if len <= held {
            return self.truncate(len);
        }
        let canonical = match self.field() {
            Some(field) => field.scalar(value)?,
            None => value,
        };
        let rows = vec![canonical; len - held];
        self.check(&(held..held), &rows)?;
        self.write(held..held, rows);
        Ok(())
    }

    /// Replace a record column's child of `child`'s name, or add it.
    ///
    /// # Errors
    ///
    /// Returns an error naming the serie when this is a run or a column that
    /// is not a record, when `child` is a run, or when it does not hold
    /// exactly `len` rows.
    pub fn set_child(&mut self, child: Self) -> Result<()> {
        match self {
            Self::Struct(held) => Arc::make_mut(held).set_child(child),
            other => Err(other.not_a_record("holds no child")),
        }
    }

    /// Write one cell of one row, `path` deep, in place.
    ///
    /// Every level is row-aligned, so `index` is the same row at every level.
    ///
    /// # Errors
    ///
    /// Returns an error naming the serie when this is a run or a column that
    /// is not a record, when a segment is not a record child's name or a
    /// level is not a record column, when row `index` of any record on the
    /// way is absent, or when `value` is not one the leaf's field accepts.
    pub fn set_cell(&mut self, path: &FieldPath, index: usize, value: Scalar) -> Result<()> {
        match self {
            Self::Struct(held) => Arc::make_mut(held).set_cell(path, index, value),
            other => Err(other.not_a_record("holds no cell")),
        }
    }

    /// The refusal a record-only write answers elsewhere.
    fn not_a_record(&self, what: &str) -> crate::Error {
        let reason = match self {
            Self::Run(_) => smol_str::format_smolstr!("a schema-free run {what}"),
            _ => smol_str::format_smolstr!("{} is not a record column, so it {what}", self.name()),
        };
        crate::Error::InvalidRecord {
            path: smol_str::SmolStr::new(self.name()),
            reason,
        }
    }

    /// Refuse what a write of `rows` over `range` could not do; a run can
    /// write anything.
    pub(crate) fn check(&self, range: &Range<usize>, rows: &[Scalar]) -> Result<()> {
        column!(
            self,
            _run => Ok(()),
            column => column.check(range, rows)
        )
    }

    /// Write canonical `rows` over a checked `range`, never re-proving.
    pub(crate) fn write(&mut self, range: Range<usize>, rows: Vec<Scalar>) {
        column_mut!(
            self,
            run => splice_run(run, range, rows),
            column => column.write(range, rows)
        );
    }
}

impl Default for Serie {
    fn default() -> Self {
        Self::Run(Run::default())
    }
}

impl PartialEq for Serie {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Serie {}

impl Ord for Serie {
    /// The rows, and nothing else: not the leaf, not the field. A run lends
    /// its rows and a column builds one at a time, so a run and a column of
    /// equal rows are one value.
    fn cmp(&self, other: &Self) -> Ordering {
        compare_rows(self, other)
    }
}

impl PartialOrd for Serie {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Hash for Serie {
    /// What `<[Scalar]>::hash` writes for the rows: a run hashes exactly what
    /// it hashed before the family had a second leaf, and a column hashes
    /// byte-identically to the run of its rows.
    fn hash<H: Hasher>(&self, state: &mut H) {
        column!(
            self,
            run => run.hash(state),
            column => hash_rows(column.as_ref(), state)
        );
    }
}

impl Serie {
    /// Borrow the [`NullSerie`] this is, `None` for any other leaf.
    #[must_use]
    pub fn as_null(&self) -> Option<&NullSerie> {
        Leaf::narrow(self)
    }

    /// Borrow the [`NullSerie`] this is to write, through `Arc::make_mut`: the
    /// leaf struct is copied once when the column is shared, and its typed
    /// writers validate what remains - nullability - so no write bypasses
    /// the contract.
    pub fn get_null_mut(&mut self) -> Option<&mut NullSerie> {
        Leaf::narrow_mut(self)
    }

    /// Borrow the [`BooleanSerie`] this is, `None` for any other leaf.
    #[must_use]
    pub fn as_boolean(&self) -> Option<&BooleanSerie> {
        Leaf::narrow(self)
    }

    /// Borrow the [`BooleanSerie`] this is to write, through `Arc::make_mut`: the
    /// leaf struct is copied once when the column is shared, and its typed
    /// writers validate what remains - nullability - so no write bypasses
    /// the contract.
    pub fn get_boolean_mut(&mut self) -> Option<&mut BooleanSerie> {
        Leaf::narrow_mut(self)
    }

    /// Borrow the [`Int8Serie`] this is, `None` for any other leaf.
    #[must_use]
    pub fn as_int8(&self) -> Option<&Int8Serie> {
        Leaf::narrow(self)
    }

    /// Borrow the [`Int8Serie`] this is to write, through `Arc::make_mut`: the
    /// leaf struct is copied once when the column is shared, and its typed
    /// writers validate what remains - nullability - so no write bypasses
    /// the contract.
    pub fn get_int8_mut(&mut self) -> Option<&mut Int8Serie> {
        Leaf::narrow_mut(self)
    }

    /// Borrow the [`Int16Serie`] this is, `None` for any other leaf.
    #[must_use]
    pub fn as_int16(&self) -> Option<&Int16Serie> {
        Leaf::narrow(self)
    }

    /// Borrow the [`Int16Serie`] this is to write, through `Arc::make_mut`: the
    /// leaf struct is copied once when the column is shared, and its typed
    /// writers validate what remains - nullability - so no write bypasses
    /// the contract.
    pub fn get_int16_mut(&mut self) -> Option<&mut Int16Serie> {
        Leaf::narrow_mut(self)
    }

    /// Borrow the [`Int32Serie`] this is, `None` for any other leaf.
    #[must_use]
    pub fn as_int32(&self) -> Option<&Int32Serie> {
        Leaf::narrow(self)
    }

    /// Borrow the [`Int32Serie`] this is to write, through `Arc::make_mut`: the
    /// leaf struct is copied once when the column is shared, and its typed
    /// writers validate what remains - nullability - so no write bypasses
    /// the contract.
    pub fn get_int32_mut(&mut self) -> Option<&mut Int32Serie> {
        Leaf::narrow_mut(self)
    }

    /// Borrow the [`Int64Serie`] this is, `None` for any other leaf.
    #[must_use]
    pub fn as_int64(&self) -> Option<&Int64Serie> {
        Leaf::narrow(self)
    }

    /// Borrow the [`Int64Serie`] this is to write, through `Arc::make_mut`: the
    /// leaf struct is copied once when the column is shared, and its typed
    /// writers validate what remains - nullability - so no write bypasses
    /// the contract.
    pub fn get_int64_mut(&mut self) -> Option<&mut Int64Serie> {
        Leaf::narrow_mut(self)
    }

    /// Borrow the [`UInt8Serie`] this is, `None` for any other leaf.
    #[must_use]
    pub fn as_uint8(&self) -> Option<&UInt8Serie> {
        Leaf::narrow(self)
    }

    /// Borrow the [`UInt8Serie`] this is to write, through `Arc::make_mut`: the
    /// leaf struct is copied once when the column is shared, and its typed
    /// writers validate what remains - nullability - so no write bypasses
    /// the contract.
    pub fn get_uint8_mut(&mut self) -> Option<&mut UInt8Serie> {
        Leaf::narrow_mut(self)
    }

    /// Borrow the [`UInt16Serie`] this is, `None` for any other leaf.
    #[must_use]
    pub fn as_uint16(&self) -> Option<&UInt16Serie> {
        Leaf::narrow(self)
    }

    /// Borrow the [`UInt16Serie`] this is to write, through `Arc::make_mut`: the
    /// leaf struct is copied once when the column is shared, and its typed
    /// writers validate what remains - nullability - so no write bypasses
    /// the contract.
    pub fn get_uint16_mut(&mut self) -> Option<&mut UInt16Serie> {
        Leaf::narrow_mut(self)
    }

    /// Borrow the [`UInt32Serie`] this is, `None` for any other leaf.
    #[must_use]
    pub fn as_uint32(&self) -> Option<&UInt32Serie> {
        Leaf::narrow(self)
    }

    /// Borrow the [`UInt32Serie`] this is to write, through `Arc::make_mut`: the
    /// leaf struct is copied once when the column is shared, and its typed
    /// writers validate what remains - nullability - so no write bypasses
    /// the contract.
    pub fn get_uint32_mut(&mut self) -> Option<&mut UInt32Serie> {
        Leaf::narrow_mut(self)
    }

    /// Borrow the [`UInt64Serie`] this is, `None` for any other leaf.
    #[must_use]
    pub fn as_uint64(&self) -> Option<&UInt64Serie> {
        Leaf::narrow(self)
    }

    /// Borrow the [`UInt64Serie`] this is to write, through `Arc::make_mut`: the
    /// leaf struct is copied once when the column is shared, and its typed
    /// writers validate what remains - nullability - so no write bypasses
    /// the contract.
    pub fn get_uint64_mut(&mut self) -> Option<&mut UInt64Serie> {
        Leaf::narrow_mut(self)
    }

    /// Borrow the [`Float16Serie`] this is, `None` for any other leaf.
    #[must_use]
    pub fn as_float16(&self) -> Option<&Float16Serie> {
        Leaf::narrow(self)
    }

    /// Borrow the [`Float16Serie`] this is to write, through `Arc::make_mut`: the
    /// leaf struct is copied once when the column is shared, and its typed
    /// writers validate what remains - nullability - so no write bypasses
    /// the contract.
    pub fn get_float16_mut(&mut self) -> Option<&mut Float16Serie> {
        Leaf::narrow_mut(self)
    }

    /// Borrow the [`Float32Serie`] this is, `None` for any other leaf.
    #[must_use]
    pub fn as_float32(&self) -> Option<&Float32Serie> {
        Leaf::narrow(self)
    }

    /// Borrow the [`Float32Serie`] this is to write, through `Arc::make_mut`: the
    /// leaf struct is copied once when the column is shared, and its typed
    /// writers validate what remains - nullability - so no write bypasses
    /// the contract.
    pub fn get_float32_mut(&mut self) -> Option<&mut Float32Serie> {
        Leaf::narrow_mut(self)
    }

    /// Borrow the [`Float64Serie`] this is, `None` for any other leaf.
    #[must_use]
    pub fn as_float64(&self) -> Option<&Float64Serie> {
        Leaf::narrow(self)
    }

    /// Borrow the [`Float64Serie`] this is to write, through `Arc::make_mut`: the
    /// leaf struct is copied once when the column is shared, and its typed
    /// writers validate what remains - nullability - so no write bypasses
    /// the contract.
    pub fn get_float64_mut(&mut self) -> Option<&mut Float64Serie> {
        Leaf::narrow_mut(self)
    }

    /// Borrow the [`Date32Serie`] this is, `None` for any other leaf.
    #[must_use]
    pub fn as_date32(&self) -> Option<&Date32Serie> {
        Leaf::narrow(self)
    }

    /// Borrow the [`Date32Serie`] this is to write, through `Arc::make_mut`: the
    /// leaf struct is copied once when the column is shared, and its typed
    /// writers validate what remains - nullability - so no write bypasses
    /// the contract.
    pub fn get_date32_mut(&mut self) -> Option<&mut Date32Serie> {
        Leaf::narrow_mut(self)
    }

    /// Borrow the [`Date64Serie`] this is, `None` for any other leaf.
    #[must_use]
    pub fn as_date64(&self) -> Option<&Date64Serie> {
        Leaf::narrow(self)
    }

    /// Borrow the [`Date64Serie`] this is to write, through `Arc::make_mut`: the
    /// leaf struct is copied once when the column is shared, and its typed
    /// writers validate what remains - nullability - so no write bypasses
    /// the contract.
    pub fn get_date64_mut(&mut self) -> Option<&mut Date64Serie> {
        Leaf::narrow_mut(self)
    }

    /// Borrow the [`Utf8StringSerie`] this is, `None` for any other leaf.
    #[must_use]
    pub fn as_utf8(&self) -> Option<&Utf8StringSerie> {
        Leaf::narrow(self)
    }

    /// Borrow the [`Utf8StringSerie`] this is to write, through `Arc::make_mut`: the
    /// leaf struct is copied once when the column is shared, and its typed
    /// writers validate what remains - nullability - so no write bypasses
    /// the contract.
    pub fn get_utf8_mut(&mut self) -> Option<&mut Utf8StringSerie> {
        Leaf::narrow_mut(self)
    }

    /// Borrow the [`FixedBytesSerie`] this is, `None` for any other leaf.
    #[must_use]
    pub fn as_fixed_bytes(&self) -> Option<&FixedBytesSerie> {
        Leaf::narrow(self)
    }

    /// Borrow the [`FixedBytesSerie`] this is to write, through `Arc::make_mut`: the
    /// leaf struct is copied once when the column is shared, and its typed
    /// writers validate what remains - nullability - so no write bypasses
    /// the contract.
    pub fn get_fixed_bytes_mut(&mut self) -> Option<&mut FixedBytesSerie> {
        Leaf::narrow_mut(self)
    }

    /// Borrow the [`SerieSerie`] this is, `None` for any other leaf.
    #[must_use]
    pub fn as_serie(&self) -> Option<&SerieSerie> {
        Leaf::narrow(self)
    }

    /// Borrow the [`SerieSerie`] this is to write, through `Arc::make_mut`: the
    /// leaf struct is copied once when the column is shared, and its typed
    /// writers validate what remains - nullability - so no write bypasses
    /// the contract.
    pub fn get_serie_mut(&mut self) -> Option<&mut SerieSerie> {
        Leaf::narrow_mut(self)
    }

    /// Borrow the [`SerieViewSerie`] this is, `None` for any other leaf.
    #[must_use]
    pub fn as_serie_view(&self) -> Option<&SerieViewSerie> {
        Leaf::narrow(self)
    }

    /// Borrow the [`SerieViewSerie`] this is to write, through `Arc::make_mut`: the
    /// leaf struct is copied once when the column is shared, and its typed
    /// writers validate what remains - nullability - so no write bypasses
    /// the contract.
    pub fn get_serie_view_mut(&mut self) -> Option<&mut SerieViewSerie> {
        Leaf::narrow_mut(self)
    }

    /// Borrow the [`FixedSizeSerieSerie`] this is, `None` for any other leaf.
    #[must_use]
    pub fn as_fixed_size_serie(&self) -> Option<&FixedSizeSerieSerie> {
        Leaf::narrow(self)
    }

    /// Borrow the [`FixedSizeSerieSerie`] this is to write, through `Arc::make_mut`: the
    /// leaf struct is copied once when the column is shared, and its typed
    /// writers validate what remains - nullability - so no write bypasses
    /// the contract.
    pub fn get_fixed_size_serie_mut(&mut self) -> Option<&mut FixedSizeSerieSerie> {
        Leaf::narrow_mut(self)
    }

    /// Borrow the [`LargeSerieSerie`] this is, `None` for any other leaf.
    #[must_use]
    pub fn as_large_serie(&self) -> Option<&LargeSerieSerie> {
        Leaf::narrow(self)
    }

    /// Borrow the [`LargeSerieSerie`] this is to write, through `Arc::make_mut`: the
    /// leaf struct is copied once when the column is shared, and its typed
    /// writers validate what remains - nullability - so no write bypasses
    /// the contract.
    pub fn get_large_serie_mut(&mut self) -> Option<&mut LargeSerieSerie> {
        Leaf::narrow_mut(self)
    }

    /// Borrow the [`LargeSerieViewSerie`] this is, `None` for any other leaf.
    #[must_use]
    pub fn as_large_serie_view(&self) -> Option<&LargeSerieViewSerie> {
        Leaf::narrow(self)
    }

    /// Borrow the [`LargeSerieViewSerie`] this is to write, through `Arc::make_mut`: the
    /// leaf struct is copied once when the column is shared, and its typed
    /// writers validate what remains - nullability - so no write bypasses
    /// the contract.
    pub fn get_large_serie_view_mut(&mut self) -> Option<&mut LargeSerieViewSerie> {
        Leaf::narrow_mut(self)
    }

    /// Borrow the [`StructSerie`] this is, `None` for any other leaf.
    #[must_use]
    pub fn as_struct(&self) -> Option<&StructSerie> {
        Leaf::narrow(self)
    }

    /// Borrow the [`StructSerie`] this is to write, through `Arc::make_mut`: the
    /// leaf struct is copied once when the column is shared, and its typed
    /// writers validate what remains - nullability - so no write bypasses
    /// the contract.
    pub fn get_struct_mut(&mut self) -> Option<&mut StructSerie> {
        Leaf::narrow_mut(self)
    }

    /// Borrow the [`UnionSerie`] this is, `None` for any other leaf.
    #[must_use]
    pub fn as_union(&self) -> Option<&UnionSerie> {
        Leaf::narrow(self)
    }

    /// Borrow the [`UnionSerie`] this is to write, through `Arc::make_mut`: the
    /// leaf struct is copied once when the column is shared, and its typed
    /// writers validate what remains - nullability - so no write bypasses
    /// the contract.
    pub fn get_union_mut(&mut self) -> Option<&mut UnionSerie> {
        Leaf::narrow_mut(self)
    }

    /// Borrow the [`DictionarySerie`] this is, `None` for any other leaf.
    #[must_use]
    pub fn as_dictionary(&self) -> Option<&DictionarySerie> {
        Leaf::narrow(self)
    }

    /// Borrow the [`DictionarySerie`] this is to write, through `Arc::make_mut`: the
    /// leaf struct is copied once when the column is shared, and its typed
    /// writers validate what remains - nullability - so no write bypasses
    /// the contract.
    pub fn get_dictionary_mut(&mut self) -> Option<&mut DictionarySerie> {
        Leaf::narrow_mut(self)
    }

    /// Borrow the [`Decimal32Serie`] this is, `None` for any other leaf.
    #[must_use]
    pub fn as_decimal32(&self) -> Option<&Decimal32Serie> {
        Leaf::narrow(self)
    }

    /// Borrow the [`Decimal32Serie`] this is to write, through `Arc::make_mut`: the
    /// leaf struct is copied once when the column is shared, and its typed
    /// writers validate what remains - nullability - so no write bypasses
    /// the contract.
    pub fn get_decimal32_mut(&mut self) -> Option<&mut Decimal32Serie> {
        Leaf::narrow_mut(self)
    }

    /// Borrow the [`Decimal64Serie`] this is, `None` for any other leaf.
    #[must_use]
    pub fn as_decimal64(&self) -> Option<&Decimal64Serie> {
        Leaf::narrow(self)
    }

    /// Borrow the [`Decimal64Serie`] this is to write, through `Arc::make_mut`: the
    /// leaf struct is copied once when the column is shared, and its typed
    /// writers validate what remains - nullability - so no write bypasses
    /// the contract.
    pub fn get_decimal64_mut(&mut self) -> Option<&mut Decimal64Serie> {
        Leaf::narrow_mut(self)
    }

    /// Borrow the [`Decimal128Serie`] this is, `None` for any other leaf.
    #[must_use]
    pub fn as_decimal128(&self) -> Option<&Decimal128Serie> {
        Leaf::narrow(self)
    }

    /// Borrow the [`Decimal128Serie`] this is to write, through `Arc::make_mut`: the
    /// leaf struct is copied once when the column is shared, and its typed
    /// writers validate what remains - nullability - so no write bypasses
    /// the contract.
    pub fn get_decimal128_mut(&mut self) -> Option<&mut Decimal128Serie> {
        Leaf::narrow_mut(self)
    }

    /// Borrow the [`Decimal256Serie`] this is, `None` for any other leaf.
    #[must_use]
    pub fn as_decimal256(&self) -> Option<&Decimal256Serie> {
        Leaf::narrow(self)
    }

    /// Borrow the [`Decimal256Serie`] this is to write, through `Arc::make_mut`: the
    /// leaf struct is copied once when the column is shared, and its typed
    /// writers validate what remains - nullability - so no write bypasses
    /// the contract.
    pub fn get_decimal256_mut(&mut self) -> Option<&mut Decimal256Serie> {
        Leaf::narrow_mut(self)
    }

    /// Borrow the [`MapSerie`] this is, `None` for any other leaf.
    #[must_use]
    pub fn as_map(&self) -> Option<&MapSerie> {
        Leaf::narrow(self)
    }

    /// Borrow the [`MapSerie`] this is to write, through `Arc::make_mut`: the
    /// leaf struct is copied once when the column is shared, and its typed
    /// writers validate what remains - nullability - so no write bypasses
    /// the contract.
    pub fn get_map_mut(&mut self) -> Option<&mut MapSerie> {
        Leaf::narrow_mut(self)
    }

    /// Borrow the [`RunEndEncodedSerie`] this is, `None` for any other leaf.
    #[must_use]
    pub fn as_run_end_encoded(&self) -> Option<&RunEndEncodedSerie> {
        Leaf::narrow(self)
    }

    /// Borrow the [`RunEndEncodedSerie`] this is to write, through `Arc::make_mut`: the
    /// leaf struct is copied once when the column is shared, and its typed
    /// writers validate what remains - nullability - so no write bypasses
    /// the contract.
    pub fn get_run_end_encoded_mut(&mut self) -> Option<&mut RunEndEncodedSerie> {
        Leaf::narrow_mut(self)
    }

    /// Borrow the [`VariantSerie`] this is, `None` for any other leaf.
    #[must_use]
    pub fn as_variant(&self) -> Option<&VariantSerie> {
        Leaf::narrow(self)
    }

    /// Borrow the [`VariantSerie`] this is to write, through `Arc::make_mut`: the
    /// leaf struct is copied once when the column is shared, and its typed
    /// writers validate what remains - nullability - so no write bypasses
    /// the contract.
    pub fn get_variant_mut(&mut self) -> Option<&mut VariantSerie> {
        Leaf::narrow_mut(self)
    }

    /// Borrow the [`BinarySerie`] this is, `None` for any other leaf.
    #[must_use]
    pub fn as_binary(&self) -> Option<&BinarySerie> {
        Leaf::narrow(self)
    }

    /// Borrow the [`BinarySerie`] this is to write, through `Arc::make_mut`: the
    /// leaf struct is copied once when the column is shared, and its typed
    /// writers validate what remains - nullability - so no write bypasses
    /// the contract.
    pub fn get_binary_mut(&mut self) -> Option<&mut BinarySerie> {
        Leaf::narrow_mut(self)
    }

    /// Borrow the [`Time32SecondSerie`] this is, `None` for any other leaf.
    #[must_use]
    pub fn as_time32_second(&self) -> Option<&Time32SecondSerie> {
        Leaf::narrow(self)
    }

    /// Borrow the [`Time32SecondSerie`] this is to write, through `Arc::make_mut`: the
    /// leaf struct is copied once when the column is shared, and its typed
    /// writers validate what remains - nullability - so no write bypasses
    /// the contract.
    pub fn get_time32_second_mut(&mut self) -> Option<&mut Time32SecondSerie> {
        Leaf::narrow_mut(self)
    }

    /// Borrow the [`Time32MillisecondSerie`] this is, `None` for any other leaf.
    #[must_use]
    pub fn as_time32_millisecond(&self) -> Option<&Time32MillisecondSerie> {
        Leaf::narrow(self)
    }

    /// Borrow the [`Time32MillisecondSerie`] this is to write, through `Arc::make_mut`: the
    /// leaf struct is copied once when the column is shared, and its typed
    /// writers validate what remains - nullability - so no write bypasses
    /// the contract.
    pub fn get_time32_millisecond_mut(&mut self) -> Option<&mut Time32MillisecondSerie> {
        Leaf::narrow_mut(self)
    }

    /// Borrow the [`Time64MicrosecondSerie`] this is, `None` for any other leaf.
    #[must_use]
    pub fn as_time64_microsecond(&self) -> Option<&Time64MicrosecondSerie> {
        Leaf::narrow(self)
    }

    /// Borrow the [`Time64MicrosecondSerie`] this is to write, through `Arc::make_mut`: the
    /// leaf struct is copied once when the column is shared, and its typed
    /// writers validate what remains - nullability - so no write bypasses
    /// the contract.
    pub fn get_time64_microsecond_mut(&mut self) -> Option<&mut Time64MicrosecondSerie> {
        Leaf::narrow_mut(self)
    }

    /// Borrow the [`Time64NanosecondSerie`] this is, `None` for any other leaf.
    #[must_use]
    pub fn as_time64_nanosecond(&self) -> Option<&Time64NanosecondSerie> {
        Leaf::narrow(self)
    }

    /// Borrow the [`Time64NanosecondSerie`] this is to write, through `Arc::make_mut`: the
    /// leaf struct is copied once when the column is shared, and its typed
    /// writers validate what remains - nullability - so no write bypasses
    /// the contract.
    pub fn get_time64_nanosecond_mut(&mut self) -> Option<&mut Time64NanosecondSerie> {
        Leaf::narrow_mut(self)
    }

    /// Borrow the [`DateTimeSecondSerie`] this is, `None` for any other leaf.
    #[must_use]
    pub fn as_datetime_second(&self) -> Option<&DateTimeSecondSerie> {
        Leaf::narrow(self)
    }

    /// Borrow the [`DateTimeSecondSerie`] this is to write, through `Arc::make_mut`: the
    /// leaf struct is copied once when the column is shared, and its typed
    /// writers validate what remains - nullability - so no write bypasses
    /// the contract.
    pub fn get_datetime_second_mut(&mut self) -> Option<&mut DateTimeSecondSerie> {
        Leaf::narrow_mut(self)
    }

    /// Borrow the [`DateTimeMillisecondSerie`] this is, `None` for any other leaf.
    #[must_use]
    pub fn as_datetime_millisecond(&self) -> Option<&DateTimeMillisecondSerie> {
        Leaf::narrow(self)
    }

    /// Borrow the [`DateTimeMillisecondSerie`] this is to write, through `Arc::make_mut`: the
    /// leaf struct is copied once when the column is shared, and its typed
    /// writers validate what remains - nullability - so no write bypasses
    /// the contract.
    pub fn get_datetime_millisecond_mut(&mut self) -> Option<&mut DateTimeMillisecondSerie> {
        Leaf::narrow_mut(self)
    }

    /// Borrow the [`DateTimeMicrosecondSerie`] this is, `None` for any other leaf.
    #[must_use]
    pub fn as_datetime_microsecond(&self) -> Option<&DateTimeMicrosecondSerie> {
        Leaf::narrow(self)
    }

    /// Borrow the [`DateTimeMicrosecondSerie`] this is to write, through `Arc::make_mut`: the
    /// leaf struct is copied once when the column is shared, and its typed
    /// writers validate what remains - nullability - so no write bypasses
    /// the contract.
    pub fn get_datetime_microsecond_mut(&mut self) -> Option<&mut DateTimeMicrosecondSerie> {
        Leaf::narrow_mut(self)
    }

    /// Borrow the [`DateTimeNanosecondSerie`] this is, `None` for any other leaf.
    #[must_use]
    pub fn as_datetime_nanosecond(&self) -> Option<&DateTimeNanosecondSerie> {
        Leaf::narrow(self)
    }

    /// Borrow the [`DateTimeNanosecondSerie`] this is to write, through `Arc::make_mut`: the
    /// leaf struct is copied once when the column is shared, and its typed
    /// writers validate what remains - nullability - so no write bypasses
    /// the contract.
    pub fn get_datetime_nanosecond_mut(&mut self) -> Option<&mut DateTimeNanosecondSerie> {
        Leaf::narrow_mut(self)
    }

    /// Borrow the [`DurationSecondSerie`] this is, `None` for any other leaf.
    #[must_use]
    pub fn as_duration_second(&self) -> Option<&DurationSecondSerie> {
        Leaf::narrow(self)
    }

    /// Borrow the [`DurationSecondSerie`] this is to write, through `Arc::make_mut`: the
    /// leaf struct is copied once when the column is shared, and its typed
    /// writers validate what remains - nullability - so no write bypasses
    /// the contract.
    pub fn get_duration_second_mut(&mut self) -> Option<&mut DurationSecondSerie> {
        Leaf::narrow_mut(self)
    }

    /// Borrow the [`DurationMillisecondSerie`] this is, `None` for any other leaf.
    #[must_use]
    pub fn as_duration_millisecond(&self) -> Option<&DurationMillisecondSerie> {
        Leaf::narrow(self)
    }

    /// Borrow the [`DurationMillisecondSerie`] this is to write, through `Arc::make_mut`: the
    /// leaf struct is copied once when the column is shared, and its typed
    /// writers validate what remains - nullability - so no write bypasses
    /// the contract.
    pub fn get_duration_millisecond_mut(&mut self) -> Option<&mut DurationMillisecondSerie> {
        Leaf::narrow_mut(self)
    }

    /// Borrow the [`DurationMicrosecondSerie`] this is, `None` for any other leaf.
    #[must_use]
    pub fn as_duration_microsecond(&self) -> Option<&DurationMicrosecondSerie> {
        Leaf::narrow(self)
    }

    /// Borrow the [`DurationMicrosecondSerie`] this is to write, through `Arc::make_mut`: the
    /// leaf struct is copied once when the column is shared, and its typed
    /// writers validate what remains - nullability - so no write bypasses
    /// the contract.
    pub fn get_duration_microsecond_mut(&mut self) -> Option<&mut DurationMicrosecondSerie> {
        Leaf::narrow_mut(self)
    }

    /// Borrow the [`DurationNanosecondSerie`] this is, `None` for any other leaf.
    #[must_use]
    pub fn as_duration_nanosecond(&self) -> Option<&DurationNanosecondSerie> {
        Leaf::narrow(self)
    }

    /// Borrow the [`DurationNanosecondSerie`] this is to write, through `Arc::make_mut`: the
    /// leaf struct is copied once when the column is shared, and its typed
    /// writers validate what remains - nullability - so no write bypasses
    /// the contract.
    pub fn get_duration_nanosecond_mut(&mut self) -> Option<&mut DurationNanosecondSerie> {
        Leaf::narrow_mut(self)
    }

    /// Borrow the [`IntervalYearMonthSerie`] this is, `None` for any other leaf.
    #[must_use]
    pub fn as_interval_year_month(&self) -> Option<&IntervalYearMonthSerie> {
        Leaf::narrow(self)
    }

    /// Borrow the [`IntervalYearMonthSerie`] this is to write, through `Arc::make_mut`: the
    /// leaf struct is copied once when the column is shared, and its typed
    /// writers validate what remains - nullability - so no write bypasses
    /// the contract.
    pub fn get_interval_year_month_mut(&mut self) -> Option<&mut IntervalYearMonthSerie> {
        Leaf::narrow_mut(self)
    }

    /// Borrow the [`IntervalDayTimeSerie`] this is, `None` for any other leaf.
    #[must_use]
    pub fn as_interval_day_time(&self) -> Option<&IntervalDayTimeSerie> {
        Leaf::narrow(self)
    }

    /// Borrow the [`IntervalDayTimeSerie`] this is to write, through `Arc::make_mut`: the
    /// leaf struct is copied once when the column is shared, and its typed
    /// writers validate what remains - nullability - so no write bypasses
    /// the contract.
    pub fn get_interval_day_time_mut(&mut self) -> Option<&mut IntervalDayTimeSerie> {
        Leaf::narrow_mut(self)
    }

    /// Borrow the [`IntervalMonthDayNanoSerie`] this is, `None` for any other leaf.
    #[must_use]
    pub fn as_interval_month_day_nano(&self) -> Option<&IntervalMonthDayNanoSerie> {
        Leaf::narrow(self)
    }

    /// Borrow the [`IntervalMonthDayNanoSerie`] this is to write, through `Arc::make_mut`: the
    /// leaf struct is copied once when the column is shared, and its typed
    /// writers validate what remains - nullability - so no write bypasses
    /// the contract.
    pub fn get_interval_month_day_nano_mut(&mut self) -> Option<&mut IntervalMonthDayNanoSerie> {
        Leaf::narrow_mut(self)
    }

    /// Borrow the [`LargeUtf8StringSerie`] this is, `None` for any other leaf.
    #[must_use]
    pub fn as_large_utf8(&self) -> Option<&LargeUtf8StringSerie> {
        Leaf::narrow(self)
    }

    /// Borrow the [`LargeUtf8StringSerie`] this is to write, through `Arc::make_mut`: the
    /// leaf struct is copied once when the column is shared, and its typed
    /// writers validate what remains - nullability - so no write bypasses
    /// the contract.
    pub fn get_large_utf8_mut(&mut self) -> Option<&mut LargeUtf8StringSerie> {
        Leaf::narrow_mut(self)
    }

    /// Borrow the [`Utf8ViewStringSerie`] this is, `None` for any other leaf.
    #[must_use]
    pub fn as_utf8_view(&self) -> Option<&Utf8ViewStringSerie> {
        Leaf::narrow(self)
    }

    /// Borrow the [`Utf8ViewStringSerie`] this is to write, through `Arc::make_mut`: the
    /// leaf struct is copied once when the column is shared, and its typed
    /// writers validate what remains - nullability - so no write bypasses
    /// the contract.
    pub fn get_utf8_view_mut(&mut self) -> Option<&mut Utf8ViewStringSerie> {
        Leaf::narrow_mut(self)
    }

    /// Borrow the [`BinaryStringSerie`] this is, `None` for any other leaf.
    #[must_use]
    pub fn as_binary_string(&self) -> Option<&BinaryStringSerie> {
        Leaf::narrow(self)
    }

    /// Borrow the [`BinaryStringSerie`] this is to write, through `Arc::make_mut`: the
    /// leaf struct is copied once when the column is shared, and its typed
    /// writers validate what remains - nullability - so no write bypasses
    /// the contract.
    pub fn get_binary_string_mut(&mut self) -> Option<&mut BinaryStringSerie> {
        Leaf::narrow_mut(self)
    }

    /// Borrow the [`LargeBinaryStringSerie`] this is, `None` for any other leaf.
    #[must_use]
    pub fn as_large_binary_string(&self) -> Option<&LargeBinaryStringSerie> {
        Leaf::narrow(self)
    }

    /// Borrow the [`LargeBinaryStringSerie`] this is to write, through `Arc::make_mut`: the
    /// leaf struct is copied once when the column is shared, and its typed
    /// writers validate what remains - nullability - so no write bypasses
    /// the contract.
    pub fn get_large_binary_string_mut(&mut self) -> Option<&mut LargeBinaryStringSerie> {
        Leaf::narrow_mut(self)
    }

    /// Borrow the [`BinaryViewStringSerie`] this is, `None` for any other leaf.
    #[must_use]
    pub fn as_binary_view_string(&self) -> Option<&BinaryViewStringSerie> {
        Leaf::narrow(self)
    }

    /// Borrow the [`BinaryViewStringSerie`] this is to write, through `Arc::make_mut`: the
    /// leaf struct is copied once when the column is shared, and its typed
    /// writers validate what remains - nullability - so no write bypasses
    /// the contract.
    pub fn get_binary_view_string_mut(&mut self) -> Option<&mut BinaryViewStringSerie> {
        Leaf::narrow_mut(self)
    }

    /// Borrow the [`FixedStringSerie`] this is, `None` for any other leaf.
    #[must_use]
    pub fn as_fixed_string(&self) -> Option<&FixedStringSerie> {
        Leaf::narrow(self)
    }

    /// Borrow the [`FixedStringSerie`] this is to write, through `Arc::make_mut`: the
    /// leaf struct is copied once when the column is shared, and its typed
    /// writers validate what remains - nullability - so no write bypasses
    /// the contract.
    pub fn get_fixed_string_mut(&mut self) -> Option<&mut FixedStringSerie> {
        Leaf::narrow_mut(self)
    }

    /// Borrow the [`LargeBinarySerie`] this is, `None` for any other leaf.
    #[must_use]
    pub fn as_large_binary(&self) -> Option<&LargeBinarySerie> {
        Leaf::narrow(self)
    }

    /// Borrow the [`LargeBinarySerie`] this is to write, through `Arc::make_mut`: the
    /// leaf struct is copied once when the column is shared, and its typed
    /// writers validate what remains - nullability - so no write bypasses
    /// the contract.
    pub fn get_large_binary_mut(&mut self) -> Option<&mut LargeBinarySerie> {
        Leaf::narrow_mut(self)
    }

    /// Borrow the [`BinaryViewSerie`] this is, `None` for any other leaf.
    #[must_use]
    pub fn as_binary_view(&self) -> Option<&BinaryViewSerie> {
        Leaf::narrow(self)
    }

    /// Borrow the [`BinaryViewSerie`] this is to write, through `Arc::make_mut`: the
    /// leaf struct is copied once when the column is shared, and its typed
    /// writers validate what remains - nullability - so no write bypasses
    /// the contract.
    pub fn get_binary_view_mut(&mut self) -> Option<&mut BinaryViewSerie> {
        Leaf::narrow_mut(self)
    }
}

// ------------------------------------------------------------------------
// The serde shadow: a column is its field and its rows, which is the one
// thing the sequence wire cannot write for it.
// ------------------------------------------------------------------------

/// The private serde shadow of a column: the field beside the rows.
#[derive(Deserialize, Serialize)]
#[serde(rename = "serie")]
struct SerieWire {
    field: Field,
    rows: Vec<Scalar>,
}

impl Serialize for Serie {
    /// A schema-free run writes its values; a column writes the field that
    /// types them beside those values, because that is the one thing the
    /// values cannot say for it.
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        column!(
            self,
            run => run.serialize(serializer),
            column => SerieWire {
                field: SerieValue::field(column.as_ref()).clone(),
                rows: self.rows().into_owned(),
            }
            .serialize(serializer)
        )
    }
}

impl<'de> Deserialize<'de> for Serie {
    /// The column wire alone: a run is never read through this type's own
    /// serde, so an array here is refused naming the wire.
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        use serde::de::{Error as _, MapAccess, SeqAccess, Visitor};

        struct ColumnVisitor;

        impl<'de> Visitor<'de> for ColumnVisitor {
            type Value = Serie;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a `serie` column: its field beside its rows")
            }

            fn visit_map<A: MapAccess<'de>>(self, map: A) -> std::result::Result<Serie, A::Error> {
                let wire =
                    SerieWire::deserialize(serde::de::value::MapAccessDeserializer::new(map))?;
                Serie::from_scalars(wire.field, wire.rows).map_err(A::Error::custom)
            }

            fn visit_seq<A: SeqAccess<'de>>(self, _: A) -> std::result::Result<Serie, A::Error> {
                Err(A::Error::custom(
                    "a `serie` column is its field beside its rows: bare rows are a run, read through a scalar's `serie` tag",
                ))
            }
        }

        deserializer.deserialize_struct("serie", &["field", "rows"], ColumnVisitor)
    }
}

impl Value for Serie {
    fn dtype(&self) -> Result<DataType> {
        Self::dtype(self)
    }

    fn into_scalar(self) -> Scalar {
        Scalar::Serie(self)
    }

    fn from_scalar(value: &Scalar) -> Option<&Self> {
        match value {
            Scalar::Serie(value)
            | Scalar::SerieView(value)
            | Scalar::FixedSizeSerie(value)
            | Scalar::LargeSerie(value)
            | Scalar::LargeSerieView(value) => Some(value),
            _ => None,
        }
    }
}

impl NestedValue for Serie {
    fn len(&self) -> usize {
        Self::len(self)
    }

    fn children(&self) -> Children<'_> {
        match self {
            Self::Run(run) => Children::Sequence(run.as_slice().iter()),
            column => Children::Column(ColumnRows::new(column)),
        }
    }
}

impl From<Serie> for Scalar {
    /// A serie is the value a `serie` holds: a run, or a column of the item
    /// its field names. The other four layouts are declared by their own
    /// variant, never inferred from the rows.
    fn from(value: Serie) -> Self {
        Self::Serie(value)
    }
}

impl From<Run> for Serie {
    fn from(value: Run) -> Self {
        Self::Run(value)
    }
}

impl FromIterator<Scalar> for Serie {
    /// A run, built in one allocation.
    fn from_iter<I: IntoIterator<Item = Scalar>>(iter: I) -> Self {
        Self::Run(Run::new(iter.into_iter().collect::<Arc<[Scalar]>>()))
    }
}

impl<'a> IntoIterator for &'a Serie {
    type Item = Cow<'a, Scalar>;
    type IntoIter = Children<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}
