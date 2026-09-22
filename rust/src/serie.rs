//! Serie: many values, on the fourth side of the value model.
//!
//! One type, one file - and a folder beside it, because a column is as wide
//! as Arrow's layouts are. [`DataType`] says what shape a value has,
//! [`Field`] says whose it is and whether a row may be absent, [`Scalar`] is
//! one value, and a [`Serie`] is many of them. It is what the sequence
//! family holds: `Scalar::Sequence(Serie)`, so there is one type for "many
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
//! The root is an enum over the families that hold them, each family an
//! enum over its leaves, spelled as [`DataType`]'s families are:
//!
//! | family | leaves | what a leaf lends |
//! | --- | --- | --- |
//! | [`IntegerSerie`] | [`Int8Serie`] .. [`UInt64Serie`] | `&[i32]` and its kind, straight off the buffer |
//! | [`FloatingSerie`] | [`Float16Serie`] .. [`Float64Serie`] | the same |
//! | [`DecimalSerie`] | [`Decimal32Serie`] .. [`Decimal256Serie`] | the coefficients, at the field's scale |
//! | [`TemporalSerie`] | [`Date32Serie`] .. [`IntervalMonthDayNanoSerie`] | the counts, at the field's unit |
//! | [`StringSerie`] | [`Utf8StringSerie`] .. [`FixedStringSerie`] | the offsets and the character bytes |
//! | [`BytesSerie`] | [`BinarySerie`] .. [`FixedBytesSerie`] | the offsets and the payload bytes |
//! | [`SequenceSerie`] | [`ListSerie`] .. [`FixedSizeListSerie`] | the offsets, and the item column under them |
//! | [`EnumSerie`] | [`DictionarySerie`] | the key column and the values column |
//! | [`StructSerie`] | - | one child [`Serie`] per child field |
//! | [`MappingSerie`] | - | the offsets, and the entries column under them |
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
//! use yggdryl::{DataType, Field, Int32Serie, Scalar, Serie};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let field = Field::new("price", DataType::Int32, false);
//! let array: ArrayRef = Arc::new(Int32Array::from(vec![125, 126, 127]));
//!
//! // The buffers cross in as they are.
//! let mut serie = Serie::from_arrow_array(field, array)?;
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
use crate::sequence::Run;
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

/// Emit one family's column enum over its leaves, and the dispatch every one
/// of them answers the same way.
macro_rules! serie_family {
    (
        $(#[$meta:meta])*
        $family:ident, $variant:ident, [$($leaf:ident => $held:ty),+ $(,)?]
    ) => {
        $(#[$meta])*
        #[derive(Clone)]
        #[non_exhaustive]
        pub enum $family {
            $(
                #[doc = concat!("A column of `", stringify!($leaf), "` rows.")]
                $leaf($held),
            )+
        }

        impl $family {
            /// Refuse what a write of `rows` over `range` could not do.
            pub(crate) fn check(
                &self,
                range: &::std::ops::Range<usize>,
                rows: &[$crate::Scalar],
            ) -> $crate::Result<()> {
                match self { $(Self::$leaf(column) => column.check(range, rows),)+ }
            }

            /// Write canonical `rows` over a checked `range`.
            pub(crate) fn write(
                &mut self,
                range: ::std::ops::Range<usize>,
                rows: Vec<$crate::Scalar>,
            ) {
                match self { $(Self::$leaf(column) => column.write(range, rows),)+ }
            }

            /// Append `other`'s buffers, answering whether the two hold one
            /// layout and the leaf's buffers reach the total; the root has
            /// already agreed the fields, and `false` leaves the column as
            /// it was.
            pub(crate) fn append(&mut self, other: &Self) -> bool {
                match (self, other) {
                    $((Self::$leaf(column), Self::$leaf(more)) => column.append(more),)+
                    // Unreachable for a family of one leaf.
                    #[allow(unreachable_patterns)]
                    _ => false,
                }
            }
        }

        impl $crate::value::SerieValue for $family {
            fn field(&self) -> &$crate::Field {
                match self { $(Self::$leaf(column) => column.field(),)+ }
            }

            fn field_ref(&self) -> &::std::sync::Arc<$crate::Field> {
                match self { $(Self::$leaf(column) => column.field_ref(),)+ }
            }

            fn len(&self) -> usize {
                match self { $(Self::$leaf(column) => column.len(),)+ }
            }

            fn null_count(&self) -> usize {
                match self { $(Self::$leaf(column) => column.null_count(),)+ }
            }

            fn is_null(&self, index: usize) -> $crate::Result<bool> {
                match self { $(Self::$leaf(column) => column.is_null(index),)+ }
            }

            fn scalar(&self, index: usize) -> $crate::Result<$crate::Scalar> {
                match self { $(Self::$leaf(column) => column.scalar(index),)+ }
            }

            fn slice(&self, offset: usize, length: usize) -> $crate::Result<Self> {
                match self {
                    $(Self::$leaf(column) => column.slice(offset, length).map(Self::$leaf),)+
                }
            }

            fn splice(
                &mut self,
                range: ::std::ops::Range<usize>,
                rows: Vec<$crate::Scalar>,
            ) -> $crate::Result<()> {
                match self { $(Self::$leaf(column) => column.splice(range, rows),)+ }
            }

            fn into_arrow_array(&self) -> ::arrow_array::ArrayRef {
                match self { $(Self::$leaf(column) => column.into_arrow_array(),)+ }
            }

            fn into_serie(self) -> Serie {
                Serie::$variant(::std::sync::Arc::new(self))
            }

            fn from_serie(value: &Serie) -> Option<&Self> {
                match value {
                    Serie::$variant(family) => Some(family.as_ref()),
                    _ => None,
                }
            }
        }

        impl ::std::fmt::Debug for $family {
            fn fmt(&self, formatter: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                match self { $(Self::$leaf(column) => ::std::fmt::Debug::fmt(column, formatter),)+ }
            }
        }

        serie_leaf!($family);

        impl From<$family> for Serie {
            fn from(value: $family) -> Self {
                Self::$variant(::std::sync::Arc::new(value))
            }
        }
    };
}

mod arrow;
mod boolean;
mod bytes;
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
mod variant;

pub use boolean::BooleanSerie;
pub use bytes::{
    BinarySerie, BinaryViewSerie, ByteKind, ByteLeaf, ByteSerie, ByteViewSerie, Chars,
    FixedBytesSerie, FixedLeaf, FixedSerie, LargeBinarySerie, Octets, ViewLeaf,
};
pub use enums::DictionarySerie;
pub use mapping::MappingSerie;
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
    FixedSizeListSerie, LargeListSerie, LargeListViewSerie, ListLeaf, ListSerie, ListViewSerie,
    OffsetListSerie, OffsetListViewSerie,
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
// The families.
// ------------------------------------------------------------------------

serie_family!(
    /// The integer family as one column: any signed or unsigned width.
    IntegerSerie,
    Integer,
    [
        Int8 => Int8Serie,
        Int16 => Int16Serie,
        Int32 => Int32Serie,
        Int64 => Int64Serie,
        UInt8 => UInt8Serie,
        UInt16 => UInt16Serie,
        UInt32 => UInt32Serie,
        UInt64 => UInt64Serie,
    ]
);

serie_family!(
    /// The floating family as one column: any IEEE width.
    FloatingSerie,
    Floating,
    [
        Float16 => Float16Serie,
        Float32 => Float32Serie,
        Float64 => Float64Serie,
    ]
);

serie_family!(
    /// The decimal family as one column: any coefficient width.
    DecimalSerie,
    Decimal,
    [
        Decimal32 => Decimal32Serie,
        Decimal64 => Decimal64Serie,
        Decimal128 => Decimal128Serie,
        Decimal256 => Decimal256Serie,
    ]
);

serie_family!(
    /// The temporal family as one column: a date, a time, a datetime, a
    /// duration or a calendar interval, at the unit its field declares.
    TemporalSerie,
    Temporal,
    [
        Date32 => Date32Serie,
        Date64 => Date64Serie,
        Time32Second => Time32SecondSerie,
        Time32Millisecond => Time32MillisecondSerie,
        Time64Microsecond => Time64MicrosecondSerie,
        Time64Nanosecond => Time64NanosecondSerie,
        DateTimeSecond => DateTimeSecondSerie,
        DateTimeMillisecond => DateTimeMillisecondSerie,
        DateTimeMicrosecond => DateTimeMicrosecondSerie,
        DateTimeNanosecond => DateTimeNanosecondSerie,
        DurationSecond => DurationSecondSerie,
        DurationMillisecond => DurationMillisecondSerie,
        DurationMicrosecond => DurationMicrosecondSerie,
        DurationNanosecond => DurationNanosecondSerie,
        IntervalYearMonth => IntervalYearMonthSerie,
        IntervalDayTime => IntervalDayTimeSerie,
        IntervalMonthDayNano => IntervalMonthDayNanoSerie,
    ]
);

serie_family!(
    /// The string family as one column, named by the buffers it holds.
    ///
    /// The field says which of the crate's eighteen string leaves - and which
    /// of its registered codes - the bytes under it are; the leaf here says
    /// how they are laid out, because that is what a reader of the offsets
    /// and the characters needs.
    StringSerie,
    String,
    [
        Utf8 => Utf8StringSerie,
        LargeUtf8 => LargeUtf8StringSerie,
        Utf8View => Utf8ViewStringSerie,
        Binary => BinaryStringSerie,
        LargeBinary => LargeBinaryStringSerie,
        BinaryView => BinaryViewStringSerie,
        Fixed => FixedStringSerie,
    ]
);

serie_family!(
    /// The byte family as one column, named by the buffers it holds.
    ///
    /// A UUID and a geospatial reading are bytes with an identity, so their
    /// columns are leaves here and their field is what names them.
    BytesSerie,
    Bytes,
    [
        Binary => BinarySerie,
        LargeBinary => LargeBinarySerie,
        BinaryView => BinaryViewSerie,
        Fixed => FixedBytesSerie,
    ]
);

serie_family!(
    /// The sequence family as one column: the five list layouts, each an
    /// item column under its own cut.
    SequenceSerie,
    Sequence,
    [
        List => ListSerie,
        LargeList => LargeListSerie,
        ListView => ListViewSerie,
        LargeListView => LargeListViewSerie,
        FixedSizeList => FixedSizeListSerie,
    ]
);

serie_family!(
    /// The enum family as one column: a key column over a values column.
    EnumSerie,
    Enum,
    [
        Dictionary => DictionarySerie,
    ]
);

// ------------------------------------------------------------------------
// The root.
// ------------------------------------------------------------------------

/// The sequence family's value: many values, under one field or under none.
///
/// One type answers "many values" everywhere in the crate. A [`Serie::Run`]
/// is a schema-free ordered run - what a row canonicalizes to, and what a
/// document parses as. Every other leaf is a column: the Arrow buffers of
/// one [`Field`], one variant per family, each family an enum over the
/// leaves that share a value reading. That is the shape [`DataType`] has,
/// for the same reason - a caller branching on the family never asks which
/// width it was stored at, and one that wants the buffers narrows to the
/// leaf and gets them typed.
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
    /// A column of integers, at any signed or unsigned width.
    Integer(Arc<IntegerSerie>),
    /// A column of floats, at any IEEE width.
    Floating(Arc<FloatingSerie>),
    /// A column of exact decimals, at any coefficient width.
    Decimal(Arc<DecimalSerie>),
    /// A column of temporals, at the unit its field declares.
    Temporal(Arc<TemporalSerie>),
    /// A column of text, or of one registered code.
    String(Arc<StringSerie>),
    /// A column of bytes, or of one identity stored as bytes.
    Bytes(Arc<BytesSerie>),
    /// A column of records, each child a serie of its own.
    Struct(Arc<StructSerie>),
    /// A column of sequences: a list, a large list, a list view, a large
    /// list view or a fixed-size list, the items a serie under its cut.
    Sequence(Arc<SequenceSerie>),
    /// A column of mappings, the entries a record column under the offsets.
    Mapping(Arc<MappingSerie>),
    /// A column of union rows, one child column per member.
    Union(Arc<UnionSerie>),
    /// A column of dictionary-encoded rows: a key column over its values.
    Enum(Arc<EnumSerie>),
    /// A column of run-end-encoded rows: the run ends over their values.
    RunEndEncoded(Arc<RunEndEncodedSerie>),
    /// A column of self-describing values, each one encoded run of bytes.
    Variant(Arc<VariantSerie>),
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
            Serie::Integer($column) => $answer,
            Serie::Floating($column) => $answer,
            Serie::Decimal($column) => $answer,
            Serie::Temporal($column) => $answer,
            Serie::String($column) => $answer,
            Serie::Bytes($column) => $answer,
            Serie::Struct($column) => $answer,
            Serie::Sequence($column) => $answer,
            Serie::Mapping($column) => $answer,
            Serie::Union($column) => $answer,
            Serie::Enum($column) => $answer,
            Serie::RunEndEncoded($column) => $answer,
            Serie::Variant($column) => $answer,
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
            Serie::Integer(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::Floating(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::Decimal(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::Temporal(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::String(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::Bytes(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::Struct(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::Sequence(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::Mapping(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::Union(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::Enum(held) => {
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
        }
    };
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
    /// A column carries its field, so this is a read: a list of the item
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
            Some(field) => Ok(DataType::list(field.clone().with_name("item"))),
            None => Scalar::Sequence(self.clone()).dtype(),
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
    /// names no Arrow layout; lay one out with
    /// [`crate::arrow::array_from_value`] under the field it should have.
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
            Self::Sequence(column) => Some(column.items()),
            Self::Mapping(column) => Some(column.entries()),
            Self::Enum(column) => Some(column.values()),
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
            Self::Mapping(column) => {
                let entries = column.entries();
                (entries.field()?.name() == name).then_some(entries)?
            }
            Self::Sequence(column) => {
                let items = column.items();
                if items.field()?.name() == name {
                    items
                } else {
                    // Reading sees through a list item, the way the schema
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
    fn append(&mut self, other: &Self) -> bool {
        match (self, other) {
            (Self::Null(mine), Self::Null(theirs)) => Arc::make_mut(mine).append(theirs),
            (Self::Boolean(mine), Self::Boolean(theirs)) => Arc::make_mut(mine).append(theirs),
            (Self::Integer(mine), Self::Integer(theirs)) => Arc::make_mut(mine).append(theirs),
            (Self::Floating(mine), Self::Floating(theirs)) => Arc::make_mut(mine).append(theirs),
            (Self::Decimal(mine), Self::Decimal(theirs)) => Arc::make_mut(mine).append(theirs),
            (Self::Temporal(mine), Self::Temporal(theirs)) => Arc::make_mut(mine).append(theirs),
            (Self::String(mine), Self::String(theirs)) => Arc::make_mut(mine).append(theirs),
            (Self::Bytes(mine), Self::Bytes(theirs)) => Arc::make_mut(mine).append(theirs),
            (Self::Struct(mine), Self::Struct(theirs)) => Arc::make_mut(mine).append(theirs),
            (Self::Sequence(mine), Self::Sequence(theirs)) => Arc::make_mut(mine).append(theirs),
            (Self::Mapping(mine), Self::Mapping(theirs)) => Arc::make_mut(mine).append(theirs),
            (Self::Union(mine), Self::Union(theirs)) => Arc::make_mut(mine).append(theirs),
            (Self::Enum(mine), Self::Enum(theirs)) => Arc::make_mut(mine).append(theirs),
            (Self::RunEndEncoded(mine), Self::RunEndEncoded(theirs)) => {
                Arc::make_mut(mine).append(theirs)
            }
            (Self::Variant(mine), Self::Variant(theirs)) => Arc::make_mut(mine).append(theirs),
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

/// Name the narrowing accessor for one leaf held inside a family.
macro_rules! narrow {
    ($(#[$meta:meta])* $name:ident / $mutable:ident, $leaf:ty, $family:ident, $held:ident, $variant:ident) => {
        $(#[$meta])*
        pub fn $name(&self) -> Option<&$leaf> {
            <$leaf as SerieValue>::from_serie(self)
        }

        narrow_mut!(
            #[doc = concat!("Return the `", stringify!($leaf), "` to write when this is that leaf.")]
            ///
            /// Through `Arc::make_mut`: the leaf struct is copied once when
            /// the column is shared, and its typed writers validate what
            /// remains - nullability - so no write bypasses the contract.
            $mutable, $leaf, $family, $held, $variant
        );
    };
}

/// Name the mutable narrowing accessor for one leaf held inside a family.
macro_rules! narrow_mut {
    ($(#[$meta:meta])* $name:ident, $leaf:ty, $family:ident, $held:ident, $variant:ident) => {
        $(#[$meta])*
        pub fn $name(&mut self) -> Option<&mut $leaf> {
            match self {
                Self::$family(family) => match Arc::make_mut(family) {
                    $held::$variant(column) => Some(column),
                    // Unreachable for a family of one leaf.
                    #[allow(unreachable_patterns)]
                    _ => None,
                },
                _ => None,
            }
        }
    };
}

/// Name both narrowing accessors for what a root variant holds directly.
macro_rules! narrow_root {
    ($(#[$meta:meta])* $name:ident / $mutable:ident, $held:ty, $variant:ident) => {
        $(#[$meta])*
        pub fn $name(&self) -> Option<&$held> {
            match self {
                Self::$variant(held) => Some(held.as_ref()),
                _ => None,
            }
        }

        #[doc = concat!("Return the `", stringify!($held), "` to write when this is that leaf.")]
        ///
        /// Through `Arc::make_mut`: copied once when the column is shared.
        pub fn $mutable(&mut self) -> Option<&mut $held> {
            match self {
                Self::$variant(held) => Some(Arc::make_mut(held)),
                _ => None,
            }
        }
    };
}

impl Serie {
    narrow_root!(
        /// Return the null column when this is that leaf.
        as_null / get_null_mut, NullSerie, Null
    );
    narrow_root!(
        /// Return the boolean column when this is that leaf.
        as_boolean / get_boolean_mut, BooleanSerie, Boolean
    );
    narrow_root!(
        /// Return the integer family when this column is one.
        as_integer / get_integer_mut, IntegerSerie, Integer
    );
    narrow_root!(
        /// Return the floating family when this column is one.
        as_floating / get_floating_mut, FloatingSerie, Floating
    );
    narrow_root!(
        /// Return the decimal family when this column is one.
        as_decimal / get_decimal_mut, DecimalSerie, Decimal
    );
    narrow_root!(
        /// Return the temporal family when this column is one.
        as_temporal / get_temporal_mut, TemporalSerie, Temporal
    );
    narrow_root!(
        /// Return the string family when this column is one.
        as_string / get_string_mut, StringSerie, String
    );
    narrow_root!(
        /// Return the byte family when this column is one.
        as_bytes / get_bytes_mut, BytesSerie, Bytes
    );
    narrow_root!(
        /// Return the record column when this is that leaf.
        as_struct / get_struct_mut, StructSerie, Struct
    );
    narrow_root!(
        /// Return the sequence family when this column is one.
        as_sequence / get_sequence_mut, SequenceSerie, Sequence
    );
    narrow_root!(
        /// Return the mapping column when this is that leaf.
        as_mapping / get_mapping_mut, MappingSerie, Mapping
    );
    narrow_root!(
        /// Return the union column when this is that leaf.
        as_union / get_union_mut, UnionSerie, Union
    );
    narrow_root!(
        /// Return the enum family when this column is one.
        as_enum / get_enum_mut, EnumSerie, Enum
    );
    narrow_root!(
        /// Return the run-end-encoded column when this is that leaf.
        as_run_end_encoded / get_run_end_encoded_mut, RunEndEncodedSerie, RunEndEncoded
    );
    narrow_root!(
        /// Return the variant column when this is that leaf.
        as_variant / get_variant_mut, VariantSerie, Variant
    );

    narrow!(
        /// Return the signed 8-bit column when this is that leaf.
        as_int8 / get_int8_mut, Int8Serie, Integer, IntegerSerie, Int8
    );
    narrow!(
        /// Return the signed 16-bit column when this is that leaf.
        as_int16 / get_int16_mut, Int16Serie, Integer, IntegerSerie, Int16
    );
    narrow!(
        /// Return the signed 32-bit column when this is that leaf.
        as_int32 / get_int32_mut, Int32Serie, Integer, IntegerSerie, Int32
    );
    narrow!(
        /// Return the signed 64-bit column when this is that leaf.
        as_int64 / get_int64_mut, Int64Serie, Integer, IntegerSerie, Int64
    );
    narrow!(
        /// Return the unsigned 8-bit column when this is that leaf.
        as_uint8 / get_uint8_mut, UInt8Serie, Integer, IntegerSerie, UInt8
    );
    narrow!(
        /// Return the unsigned 16-bit column when this is that leaf.
        as_uint16 / get_uint16_mut, UInt16Serie, Integer, IntegerSerie, UInt16
    );
    narrow!(
        /// Return the unsigned 32-bit column when this is that leaf.
        as_uint32 / get_uint32_mut, UInt32Serie, Integer, IntegerSerie, UInt32
    );
    narrow!(
        /// Return the unsigned 64-bit column when this is that leaf.
        as_uint64 / get_uint64_mut, UInt64Serie, Integer, IntegerSerie, UInt64
    );
    narrow!(
        /// Return the binary16 column when this is that leaf.
        as_float16 / get_float16_mut, Float16Serie, Floating, FloatingSerie, Float16
    );
    narrow!(
        /// Return the binary32 column when this is that leaf.
        as_float32 / get_float32_mut, Float32Serie, Floating, FloatingSerie, Float32
    );
    narrow!(
        /// Return the binary64 column when this is that leaf.
        as_float64 / get_float64_mut, Float64Serie, Floating, FloatingSerie, Float64
    );
    narrow!(
        /// Return the 32-bit decimal column when this is that leaf.
        as_decimal32 / get_decimal32_mut, Decimal32Serie, Decimal, DecimalSerie, Decimal32
    );
    narrow!(
        /// Return the 64-bit decimal column when this is that leaf.
        as_decimal64 / get_decimal64_mut, Decimal64Serie, Decimal, DecimalSerie, Decimal64
    );
    narrow!(
        /// Return the 128-bit decimal column when this is that leaf.
        as_decimal128 / get_decimal128_mut, Decimal128Serie, Decimal, DecimalSerie, Decimal128
    );
    narrow!(
        /// Return the 256-bit decimal column when this is that leaf.
        as_decimal256 / get_decimal256_mut, Decimal256Serie, Decimal, DecimalSerie, Decimal256
    );
    narrow!(
        /// Return the day-count date column when this is that leaf.
        as_date32 / get_date32_mut, Date32Serie, Temporal, TemporalSerie, Date32
    );
    narrow!(
        /// Return the millisecond-count date column when this is that leaf.
        as_date64 / get_date64_mut, Date64Serie, Temporal, TemporalSerie, Date64
    );
    narrow!(
        /// Return the second-count time column when this is that leaf.
        as_time32_second / get_time32_second_mut, Time32SecondSerie, Temporal, TemporalSerie, Time32Second
    );
    narrow!(
        /// Return the millisecond-count time column when this is that leaf.
        as_time32_millisecond / get_time32_millisecond_mut, Time32MillisecondSerie, Temporal, TemporalSerie, Time32Millisecond
    );
    narrow!(
        /// Return the microsecond-count time column when this is that leaf.
        as_time64_microsecond / get_time64_microsecond_mut, Time64MicrosecondSerie, Temporal, TemporalSerie, Time64Microsecond
    );
    narrow!(
        /// Return the nanosecond-count time column when this is that leaf.
        as_time64_nanosecond / get_time64_nanosecond_mut, Time64NanosecondSerie, Temporal, TemporalSerie, Time64Nanosecond
    );
    narrow!(
        /// Return the second-count datetime column when this is that leaf.
        as_datetime_second / get_datetime_second_mut, DateTimeSecondSerie, Temporal, TemporalSerie, DateTimeSecond
    );
    narrow!(
        /// Return the millisecond-count datetime column when this is that leaf.
        as_datetime_millisecond / get_datetime_millisecond_mut, DateTimeMillisecondSerie, Temporal, TemporalSerie, DateTimeMillisecond
    );
    narrow!(
        /// Return the microsecond-count datetime column when this is that leaf.
        as_datetime_microsecond / get_datetime_microsecond_mut, DateTimeMicrosecondSerie, Temporal, TemporalSerie, DateTimeMicrosecond
    );
    narrow!(
        /// Return the nanosecond-count datetime column when this is that leaf.
        as_datetime_nanosecond / get_datetime_nanosecond_mut, DateTimeNanosecondSerie, Temporal, TemporalSerie, DateTimeNanosecond
    );
    narrow!(
        /// Return the second-count duration column when this is that leaf.
        as_duration_second / get_duration_second_mut, DurationSecondSerie, Temporal, TemporalSerie, DurationSecond
    );
    narrow!(
        /// Return the millisecond-count duration column when this is that leaf.
        as_duration_millisecond / get_duration_millisecond_mut, DurationMillisecondSerie, Temporal, TemporalSerie, DurationMillisecond
    );
    narrow!(
        /// Return the microsecond-count duration column when this is that leaf.
        as_duration_microsecond / get_duration_microsecond_mut, DurationMicrosecondSerie, Temporal, TemporalSerie, DurationMicrosecond
    );
    narrow!(
        /// Return the nanosecond-count duration column when this is that leaf.
        as_duration_nanosecond / get_duration_nanosecond_mut, DurationNanosecondSerie, Temporal, TemporalSerie, DurationNanosecond
    );
    narrow!(
        /// Return the year-month interval column when this is that leaf.
        as_interval_year_month / get_interval_year_month_mut, IntervalYearMonthSerie, Temporal, TemporalSerie, IntervalYearMonth
    );
    narrow!(
        /// Return the day-time interval column when this is that leaf.
        as_interval_day_time / get_interval_day_time_mut, IntervalDayTimeSerie, Temporal, TemporalSerie, IntervalDayTime
    );
    narrow!(
        /// Return the month-day-nanosecond interval column when this is that leaf.
        as_interval_month_day_nano / get_interval_month_day_nano_mut, IntervalMonthDayNanoSerie, Temporal, TemporalSerie, IntervalMonthDayNano
    );
    narrow!(
        /// Return the UTF-8 column when this is that leaf.
        as_utf8 / get_utf8_mut, Utf8StringSerie, String, StringSerie, Utf8
    );
    narrow!(
        /// Return the 64-bit-offset UTF-8 column when this is that leaf.
        as_large_utf8 / get_large_utf8_mut, LargeUtf8StringSerie, String, StringSerie, LargeUtf8
    );
    narrow!(
        /// Return the viewed UTF-8 column when this is that leaf.
        as_utf8_view / get_utf8_view_mut, Utf8ViewStringSerie, String, StringSerie, Utf8View
    );
    narrow!(
        /// Return the binary-stored text column when this is that leaf.
        as_binary_string / get_binary_string_mut, BinaryStringSerie, String, StringSerie, Binary
    );
    narrow!(
        /// Return the 64-bit-offset binary-stored text column when this is that leaf.
        as_large_binary_string / get_large_binary_string_mut, LargeBinaryStringSerie, String, StringSerie, LargeBinary
    );
    narrow!(
        /// Return the viewed binary-stored text column when this is that leaf.
        as_binary_view_string / get_binary_view_string_mut, BinaryViewStringSerie, String, StringSerie, BinaryView
    );
    narrow!(
        /// Return the fixed-width text column when this is that leaf.
        as_fixed_string / get_fixed_string_mut, FixedStringSerie, String, StringSerie, Fixed
    );
    narrow!(
        /// Return the plain byte column when this is that leaf.
        as_binary / get_binary_mut, BinarySerie, Bytes, BytesSerie, Binary
    );
    narrow!(
        /// Return the 64-bit-offset byte column when this is that leaf.
        as_large_binary / get_large_binary_mut, LargeBinarySerie, Bytes, BytesSerie, LargeBinary
    );
    narrow!(
        /// Return the viewed byte column when this is that leaf.
        as_binary_view / get_binary_view_mut, BinaryViewSerie, Bytes, BytesSerie, BinaryView
    );
    narrow!(
        /// Return the fixed-width byte column when this is that leaf.
        as_fixed_bytes / get_fixed_bytes_mut, FixedBytesSerie, Bytes, BytesSerie, Fixed
    );
    narrow!(
        /// Return the 32-bit-offset list column when this is that leaf.
        as_list / get_list_mut, ListSerie, Sequence, SequenceSerie, List
    );
    narrow!(
        /// Return the 64-bit-offset list column when this is that leaf.
        as_large_list / get_large_list_mut, LargeListSerie, Sequence, SequenceSerie, LargeList
    );
    narrow!(
        /// Return the 32-bit list-view column when this is that leaf.
        as_list_view / get_list_view_mut, ListViewSerie, Sequence, SequenceSerie, ListView
    );
    narrow!(
        /// Return the 64-bit list-view column when this is that leaf.
        as_large_list_view / get_large_list_view_mut, LargeListViewSerie, Sequence, SequenceSerie, LargeListView
    );
    narrow!(
        /// Return the fixed-size list column when this is that leaf.
        as_fixed_size_list / get_fixed_size_list_mut, FixedSizeListSerie, Sequence, SequenceSerie, FixedSizeList
    );
    narrow!(
        /// Return the dictionary column when this is that leaf.
        as_dictionary / get_dictionary_mut, DictionarySerie, Enum, EnumSerie, Dictionary
    );
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
    /// The column wire alone: a run is never spelled through this type's own
    /// serde, so a list here is refused naming the wire.
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
                    "a `serie` is a column, its field beside its rows: a list of values is a run, spelled under `sequence`",
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
        Scalar::Sequence(self)
    }

    fn from_scalar(value: &Scalar) -> Option<&Self> {
        match value {
            Scalar::Sequence(value) => Some(value),
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
    fn from(value: Serie) -> Self {
        Self::Sequence(value)
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
