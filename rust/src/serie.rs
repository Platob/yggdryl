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
//! [`Serie::List`] is a schema-free ordered run - what a row canonicalizes
//! to, what a document parses as, and what [`Scalar::from_sequence`] builds.
//! It holds its values and lends them. Every other leaf is a column: the
//! Arrow buffers of one [`Field`], which holds no [`Scalar`] at all and
//! builds a row only when one is asked for.
//!
//! What separates them is the field. A run declares none, so
//! [`Serie::field`] answers `None` for it and its datatype is agreed back
//! out of its rows; a column carries one, so its datatype is read rather
//! than agreed and an empty column still names it.
//!
//! # The nesting points back here
//!
//! A record's child, a sequence's items and a mapping's entries are each a
//! [`Serie`] - the same type, all the way down. So a column of records is
//! columns of columns, and reaching a leaf three levels deep is three
//! borrows and no copy.
//!
//! # The rows are buffers, and the leaf names them
//!
//! A column stores no [`Scalar`]. It stores what Arrow stores - a values
//! buffer, offsets where the layout has them, a validity bitmap - and the
//! root is an enum over the families that hold them, each family an enum over
//! its leaves:
//!
//! | family | leaves | what a leaf lends |
//! | --- | --- | --- |
//! | [`IntegerSerie`] | [`Int8Serie`] .. [`UInt64Serie`] | `&[i32]` and its kind, straight off the buffer |
//! | [`FloatingSerie`] | [`Float16Serie`] .. [`Float64Serie`] | the same |
//! | [`DecimalSerie`] | [`Decimal32Serie`] .. [`Decimal256Serie`] | the coefficients, at the field's scale |
//! | [`TemporalSerie`] | [`Date32Serie`] .. [`IntervalMonthDayNanoSerie`] | the counts, at the field's unit |
//! | [`StringSerie`] | [`Utf8StringSerie`], [`LargeUtf8StringSerie`], and the rest | the offsets and the character bytes |
//! | [`BytesSerie`] | [`BinarySerie`] .. [`FixedBytesSerie`] | the offsets and the payload bytes |
//! | [`StructSerie`] | - | one child [`Serie`] per child field |
//! | [`SequenceSerie`] | - | the offsets, and the item column under them |
//! | [`MappingSerie`] | - | the offsets, and the entry column under them |
//! | [`VariantSerie`] | - | the encoded bytes of one row, decoded on demand |
//!
//! Every leaf is one [`ArrayRef`]'s buffers behind the field that types them,
//! so a column crosses into Arrow and back by sharing them -
//! [`Serie::from_arrow_array`] and [`Serie::into_arrow_array`] copy no row -
//! and a nested column crosses child by child, which is why
//! [`StructSerie::child`] answers a [`Serie`] rather than a projection.
//!
//! The column leaves are shared behind one pointer each, so a [`Scalar`]
//! carrying a serie is two words and cloning one is a pointer bump. The run
//! is held inline, because a row canonicalizes to one and paying an extra
//! indirection per row is the one cost this type cannot take.
//!
//! # The value side is lazy
//!
//! Nothing is decoded until something asks for one row.
//! [`SerieValue::scalar`] builds that row and no other, and the typed
//! accessors do not build one at all: [`Int32Serie::value`] is a bounds check
//! and a buffer read, [`Utf8StringSerie::value`] borrows the characters where
//! they lie. The mutators are the same bargain in reverse -
//! [`Int32Serie::push_value`] and [`Int32Serie::set_value`] write the buffer,
//! and [`SerieValue::push`] and [`SerieValue::set`] read a value through the
//! field's one contract and then write that buffer. Arrow hands its buffers
//! back as a builder when nothing else holds them, so an append and a slot
//! write are in place; when something does, the rows are copied once.
//!
//! # What a column proves, and when
//!
//! - [`Serie::from_scalars`] takes values and proves every one through
//!   [`Field::scalar`], the crate's one value contract.
//! - [`Serie::from_arrow_array`] takes buffers and proves the layout, which is
//!   what buffers can be asked in constant time. A value those buffers hold
//!   that the field's own contract would refuse - text no
//!   [`Currency`](crate::Currency) registers, bytes that are not well-known
//!   binary - is refused where it is read, by [`SerieValue::scalar`], and
//!   never silently.
//!
//! Many values of one field is a list of that field, which is a datatype the
//! sequence family already owns, so a serie *is* that family's value rather
//! than a root of its own: `Scalar::Sequence(Serie)`. It adds no
//! [`DataTypeId`](crate::DataTypeId), no [`DataType`] variant and no
//! [`Field`] variant.
//!
//! A column stores no [`Scalar`] anywhere, so it lends none: reading one
//! builds the rows asked for and keeps nothing. [`Serie::as_slice`]
//! therefore answers only for the schema-free run, [`Serie::rows`] reads
//! either - borrowing the run's values, building the column's - and a walk
//! over a value yields `Cow`, borrowed where the value was already there.
//!
//! ```
//! use std::sync::Arc;
//!
//! use arrow_array::{ArrayRef, Int32Array};
//! use yggdryl::{DataType, Field, Int32Serie, Scalar, Serie, SerieValue};
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
//! // And it grows into the buffer it already holds.
//! serie.push(Scalar::from(128_i32))?;
//! assert_eq!(serie.as_int32().expect("an int32 column").values().len(), 4);
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
use std::sync::Arc;

use arrow_array::{Array, ArrayRef};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::sequence::List;
use crate::value::SerieValue;
use crate::{DataType, Field, Result, Scalar};

// ------------------------------------------------------------------------
// The machinery every leaf reads and writes through, declared before the
// modules that use it.
// ------------------------------------------------------------------------

/// Emit the identity a column has: the field it is typed by and the rows it
/// holds, never how its buffers happen to be cut.
///
/// Ordering reads the rows, because that is what a column *is*; hashing reads
/// the field and the length, because those are what two equal columns share
/// without touching a buffer, and a hash that decoded one would make a map
/// lookup a scan.
macro_rules! serie_leaf {
    ($name:ident $(, $param:ident : $bound:path)*) => {
        impl<$($param: $bound),*> PartialEq for $name<$($param),*> {
            fn eq(&self, other: &Self) -> bool {
                $crate::serie::compare_leaves(self, other) == ::std::cmp::Ordering::Equal
            }
        }

        impl<$($param: $bound),*> Eq for $name<$($param),*> {}

        impl<$($param: $bound),*> PartialOrd for $name<$($param),*> {
            fn partial_cmp(&self, other: &Self) -> Option<::std::cmp::Ordering> {
                Some(::std::cmp::Ord::cmp(self, other))
            }
        }

        impl<$($param: $bound),*> Ord for $name<$($param),*> {
            fn cmp(&self, other: &Self) -> ::std::cmp::Ordering {
                $crate::serie::compare_leaves(self, other)
            }
        }

        impl<$($param: $bound),*> ::std::hash::Hash for $name<$($param),*> {
            fn hash<H: ::std::hash::Hasher>(&self, state: &mut H) {
                $crate::serie::hash_leaf(self, state);
            }
        }

        impl<$($param: $bound),*> ::std::fmt::Display for $name<$($param),*> {
            fn fmt(&self, formatter: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                $crate::serie::display_leaf(self, formatter)
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

        impl $crate::value::SerieValue for $family {
            fn field(&self) -> &$crate::Field {
                match self { $(Self::$leaf(column) => column.field(),)+ }
            }

            fn len(&self) -> usize {
                match self { $(Self::$leaf(column) => column.len(),)+ }
            }

            fn null_count(&self) -> usize {
                match self { $(Self::$leaf(column) => column.null_count(),)+ }
            }

            fn is_null(&self, index: usize) -> bool {
                match self { $(Self::$leaf(column) => column.is_null(index),)+ }
            }

            fn scalar(&self, index: usize) -> $crate::Result<$crate::Scalar> {
                match self { $(Self::$leaf(column) => column.scalar(index),)+ }
            }

            fn set(&mut self, index: usize, value: $crate::Scalar) -> $crate::Result<()> {
                match self { $(Self::$leaf(column) => column.set(index, value),)+ }
            }

            fn push(&mut self, value: $crate::Scalar) -> $crate::Result<()> {
                match self { $(Self::$leaf(column) => column.push(value),)+ }
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

mod nested;
mod primitive;
mod text;
mod variant;

pub use nested::{
    Entries, GenericSequenceSerie, Items, LargeSequenceSerie, MappingSerie, SequenceKind,
    SequenceSerie, StructSerie,
};
pub use primitive::{
    BooleanSerie, Date32Serie, Date64Serie, DateTimeMicrosecondSerie, DateTimeMillisecondSerie,
    DateTimeNanosecondSerie, DateTimeSecondSerie, Decimal32Serie, Decimal64Serie, Decimal128Serie,
    Decimal256Serie, DurationMicrosecondSerie, DurationMillisecondSerie, DurationNanosecondSerie,
    DurationSecondSerie, Float16Serie, Float32Serie, Float64Serie, Int8Serie, Int16Serie,
    Int32Serie, Int64Serie, IntervalDayTimeSerie, IntervalMonthDayNanoSerie,
    IntervalYearMonthSerie, NullSerie, PrimitiveLeaf, PrimitiveSerie, Time32MillisecondSerie,
    Time32SecondSerie, Time64MicrosecondSerie, Time64NanosecondSerie, UInt8Serie, UInt16Serie,
    UInt32Serie, UInt64Serie,
};
pub use text::{
    BinarySerie, BinaryStringSerie, BinaryViewSerie, BinaryViewStringSerie, ByteKind, ByteLeaf,
    ByteSerie, ByteViewSerie, FixedBytesSerie, FixedLeaf, FixedSerie, FixedStringSerie,
    LargeBinarySerie, LargeBinaryStringSerie, LargeUtf8StringSerie, RawRun, TextRun,
    Utf8StringSerie, Utf8ViewStringSerie, ViewLeaf,
};
pub use variant::VariantSerie;

/// Order two columns by their field, then their length, then their rows.
///
/// Rows that cannot be read order after rows that can, and two columns whose
/// rows both refuse order equal: a refusal is not a value, and an order has
/// to be total.
pub(crate) fn compare_leaves<L: SerieValue, R: SerieValue>(left: &L, right: &R) -> Ordering {
    let order = left
        .field()
        .cmp(right.field())
        .then_with(|| left.len().cmp(&right.len()));
    if order != Ordering::Equal {
        return order;
    }
    for index in 0..left.len() {
        let step = match (left.scalar(index), right.scalar(index)) {
            (Ok(one), Ok(other)) => one.cmp(&other),
            (Ok(_), Err(_)) => Ordering::Less,
            (Err(_), Ok(_)) => Ordering::Greater,
            (Err(_), Err(_)) => Ordering::Equal,
        };
        if step != Ordering::Equal {
            return step;
        }
    }
    Ordering::Equal
}

/// Hash what two equal columns share without reading a buffer.
pub(crate) fn hash_leaf<S: SerieValue, H: std::hash::Hasher>(column: &S, state: &mut H) {
    use std::hash::Hash as _;

    column.field().hash(state);
    column.len().hash(state);
}

/// Render a column as its field's name and the rows it holds.
pub(crate) fn display_leaf<S: SerieValue>(
    column: &S,
    formatter: &mut fmt::Formatter<'_>,
) -> fmt::Result {
    write!(formatter, "{}[", column.field().name())?;
    for index in 0..column.len() {
        if index != 0 {
            formatter.write_str(", ")?;
        }
        match column.scalar(index) {
            Ok(value) => write!(formatter, "{value:?}")?,
            Err(error) => write!(formatter, "<{error}>")?,
        }
    }
    formatter.write_str("]")
}

/// Render a column's shape without reading a buffer.
pub(crate) fn debug_leaf<S: SerieValue>(
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
        reason: smol_str::format_smolstr!("row {index} is past the {len} this column holds"),
    })
}

/// Build one row as the buffers of `field`'s layout, and hand back the
/// concrete array it is.
///
/// The value crosses at the crate's one scalar-array boundary, so the field
/// decides what it may be exactly as it does for a stored column.
pub(crate) fn one_row<A: Array + Clone + 'static>(field: &Field, value: Scalar) -> Result<A> {
    let array = crate::arrow::scalar_array(field, &value)?;
    array
        .as_any()
        .downcast_ref::<A>()
        .cloned()
        .ok_or_else(|| crate::Error::InvalidRecord {
            path: smol_str::SmolStr::new(field.name()),
            reason: smol_str::SmolStr::new_static("the value does not lay out as this column does"),
        })
}

/// Read row `index` of one array as the value `field` types.
///
/// [`crate::arrow::value::value_from_array`] is the crate's one
/// schema-directed decode, so a column never grows a second reader.
pub(crate) fn scalar_at(field: &Field, array: &dyn Array, index: usize) -> Result<Scalar> {
    if index >= array.len() {
        return Ok(Scalar::Null);
    }
    Ok(crate::arrow::value::value_from_array(
        field.dtype(),
        array,
        index,
    )?)
}

// ------------------------------------------------------------------------
// The families of fixed-width leaves.
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
    /// of its eleven registered codes - the bytes under it are; the leaf here
    /// says how they are laid out, because that is what a reader of the
    /// offsets and the characters needs.
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

// ------------------------------------------------------------------------
// The root.
// ------------------------------------------------------------------------

/// The sequence family's value: many values, under one field or under none.
///
/// One type answers "many values" everywhere in the crate. A [`Serie::List`]
/// is a schema-free ordered run - what a row canonicalizes to, and what a
/// document parses as. Every other leaf is a column: the Arrow buffers of
/// one [`Field`], one variant per family, each family an enum over the
/// leaves that share a value reading. That is the shape [`DataType`] has,
/// for the same reason - a caller branching on the family never asks which
/// width it was stored at, and one that wants the buffers narrows to the
/// leaf and gets them typed.
///
/// The column leaves are shared, so a [`Scalar`] carrying a serie is two
/// words and a clone of one is a pointer bump. The run is held inline,
/// because a row is one and paying an extra indirection per row is the one
/// cost this type cannot take.
#[derive(Clone)]
#[non_exhaustive]
pub enum Serie {
    /// A schema-free ordered run of values: what a row canonicalizes to.
    List(List),
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
    /// A column of sequences, the items a serie under 32-bit offsets.
    Sequence(Arc<SequenceSerie>),
    /// A column of sequences, the items a serie under 64-bit offsets.
    LargeSequence(Arc<LargeSequenceSerie>),
    /// A column of mappings, the entries a serie under the offsets.
    Mapping(Arc<MappingSerie>),
    /// A column of self-describing values, each one encoded run of bytes.
    Variant(Arc<VariantSerie>),
}

/// Forward one verb to whichever column holds the rows, with the run's own
/// answer beside it.
///
/// The run is the one leaf that carries no field, so every verb that reads a
/// field states what a run answers instead rather than pretending it has one.
macro_rules! column {
    ($self:ident, $run:ident => $bare:expr, $column:ident => $answer:expr) => {
        match $self {
            Serie::List($run) => $bare,
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
            Serie::LargeSequence($column) => $answer,
            Serie::Mapping($column) => $answer,
            Serie::Variant($column) => $answer,
        }
    };
}

/// The same, where the verb needs the column to write to.
macro_rules! column_mut {
    ($self:ident, $run:ident => $bare:expr, $column:ident => $answer:expr) => {
        match $self {
            Serie::List($run) => $bare,
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
            Serie::LargeSequence(held) => {
                let $column = Arc::make_mut(held);
                $answer
            }
            Serie::Mapping(held) => {
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
            column => display_leaf(column.as_ref(), formatter)
        )
    }
}

impl Serie {
    /// Construct a schema-free ordered run.
    pub fn new(values: impl Into<Arc<[Scalar]>>) -> Self {
        Self::List(List::new(values))
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
    /// Constant for a column, which keeps the count on its validity bitmap.
    /// A run keeps no such bitmap, so it walks its values - the one ask
    /// where the two leaves differ in cost rather than in answer.
    pub fn null_count(&self) -> usize {
        column!(
            self,
            run => run.as_slice().iter().filter(|value| value.is_null()).count(),
            column => SerieValue::null_count(column.as_ref())
        )
    }

    /// Return whether row `index` holds no value.
    pub fn is_null(&self, index: usize) -> bool {
        column!(
            self,
            run => run.as_slice().get(index).is_none_or(Scalar::is_null),
            column => SerieValue::is_null(column.as_ref(), index)
        )
    }

    /// Return row `index` as a value, or [`Scalar::Null`] past the end.
    ///
    /// A run lends what it holds and clones it; a column builds the row from
    /// its buffers and keeps nothing.
    ///
    /// # Errors
    ///
    /// [`SerieValue::scalar`] carries the rule for a column. A run never
    /// refuses.
    pub fn scalar(&self, index: usize) -> Result<Scalar> {
        column!(
            self,
            run => Ok(run.as_slice().get(index).cloned().unwrap_or(Scalar::Null)),
            column => SerieValue::scalar(column.as_ref(), index)
        )
    }

    /// Overwrite row `index`, through the field's contract where there is one.
    ///
    /// A column's buffers are written in place when nothing else holds them
    /// and copied once when something does, which is what sharing a column
    /// between two values costs the first write. A run is one shared slice,
    /// so a write copies every value it holds.
    ///
    /// # Errors
    ///
    /// [`SerieValue::set`] carries the rule, and a row past the end is
    /// refused by name.
    pub fn set(&mut self, index: usize, value: Scalar) -> Result<()> {
        column_mut!(
            self,
            run => {
                let mut values = run.as_slice().to_vec();
                let slot = values.get_mut(index).ok_or_else(|| crate::Error::InvalidRecord {
                    path: smol_str::SmolStr::new_static("$"),
                    reason: smol_str::format_smolstr!(
                        "row {index} is past the {} this run holds",
                        run.as_slice().len()
                    ),
                })?;
                *slot = value;
                *run = List::new(values);
                Ok(())
            },
            column => SerieValue::set(column, index, value)
        )
    }

    /// Append one row, through the field's contract where there is one.
    ///
    /// A column writes its buffers, amortized. A run is one shared slice, so
    /// appending to it copies every value it holds - building a run one
    /// [`Self::push`] at a time is quadratic, and
    /// [`Scalar::from_sequence`] is what builds one from values already in
    /// hand.
    ///
    /// # Errors
    ///
    /// [`SerieValue::push`] carries the rule. A run accepts every value,
    /// because it declares none.
    pub fn push(&mut self, value: Scalar) -> Result<()> {
        column_mut!(
            self,
            run => {
                let mut values = run.as_slice().to_vec();
                values.push(value);
                *run = List::new(values);
                Ok(())
            },
            column => SerieValue::push(column, value)
        )
    }

    /// Borrow the ordered values where the leaf holds them.
    ///
    /// A run does and always will. A column holds Arrow buffers and no
    /// [`Scalar`], so it has none to lend and answers `None`; [`Self::rows`]
    /// is the door that reads it.
    pub fn as_slice(&self) -> Option<&[Scalar]> {
        match self {
            Self::List(run) => Some(run.as_slice()),
            _ => None,
        }
    }

    /// Return the ordered values of whichever leaf this is.
    ///
    /// A run lends what it holds; a column builds its rows, one at a time,
    /// and hands them over owned. Nothing is kept either way, so a caller
    /// reading a column twice reads it twice - hold the answer rather than
    /// asking again.
    ///
    /// # Errors
    ///
    /// Returns the column's field's own refusal where its buffers hold a
    /// value that field does not accept. A run never refuses.
    pub fn rows(&self) -> Result<Cow<'_, [Scalar]>> {
        match self {
            Self::List(run) => Ok(Cow::Borrowed(run.as_slice())),
            column => Ok(Cow::Owned(column.scalars()?)),
        }
    }

    /// Build every row as a value.
    ///
    /// One row at a time, and nothing is kept: a column holds buffers, so
    /// this is an allocation a caller asked for rather than one it inherits.
    ///
    /// # Errors
    ///
    /// [`SerieValue::scalar`] carries the rule, and names the row.
    pub fn scalars(&self) -> Result<Vec<Scalar>> {
        if let Self::List(run) = self {
            return Ok(run.as_slice().to_vec());
        }
        (0..self.len()).map(|index| self.scalar(index)).collect()
    }

    /// Return the datatype this serie materializes into.
    ///
    /// A column carries its field, so this is a read: an empty column names
    /// its datatype where an empty run cannot. A run has its item agreed
    /// back out of its rows, which is what makes the two different values.
    ///
    /// # Errors
    ///
    /// Returns an error where a run's rows do not agree on one item
    /// datatype.
    pub fn dtype(&self) -> Result<DataType> {
        match self.field() {
            Some(field) => Ok(DataType::list(field.clone())),
            None => Scalar::Sequence(self.clone()).dtype(),
        }
    }

    /// Return the rows as the schema-free run they are.
    ///
    /// The field is what a column has that a run has not, so this is the one
    /// direction that drops something and it is spelled rather than implied.
    /// A run answers itself.
    ///
    /// # Errors
    ///
    /// [`Self::scalars`] carries the rule.
    pub fn into_sequence(self) -> Result<Scalar> {
        if matches!(self, Self::List(_)) {
            return Ok(Scalar::Sequence(self));
        }
        Ok(Scalar::from_sequence(self.scalars()?))
    }

    /// Return the schema-free run when this is that leaf.
    pub fn as_run(&self) -> Option<&List> {
        match self {
            Self::List(run) => Some(run),
            _ => None,
        }
    }

    /// Return whether this serie is a column rather than a schema-free run.
    pub const fn is_column(&self) -> bool {
        !matches!(self, Self::List(_))
    }

    /// Return every row's buffers as one Arrow array, or `None` for a run.
    ///
    /// A run declares no field, so it names no Arrow layout; lay one out
    /// with [`crate::arrow::array_from_value`] under the field it should
    /// have.
    pub fn into_arrow_array(&self) -> Option<ArrayRef> {
        column!(
            self,
            _run => None,
            column => Some(SerieValue::into_arrow_array(column.as_ref()))
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
            path: smol_str::SmolStr::new_static("$"),
            reason: smol_str::SmolStr::new_static(
                "a schema-free run declares no field, so it cannot be a column",
            ),
        })
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
                path: smol_str::SmolStr::new_static("$"),
                reason: smol_str::SmolStr::new_static("a schema-free run names no Arrow layout"),
            })
    }

    /// Which leaf this is, for the tie a run of equal rows leaves open.
    const fn leaf_rank(&self) -> u8 {
        match self {
            Self::List(_) => 0,
            _ => 1,
        }
    }
}

impl Default for Serie {
    fn default() -> Self {
        Self::List(List::default())
    }
}

impl PartialEq for Serie {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Serie {}

impl Ord for Serie {
    /// The rows first, because that is what a reader of a sequence sees; the
    /// leaf only breaks a tie, so a column never sorts away from the run it
    /// holds, and two columns over one run order by the field that types
    /// them.
    fn cmp(&self, other: &Self) -> Ordering {
        // A column reads its rows here; a run has none to read. Rows that
        // cannot be read order after rows that can, and two that both refuse
        // order equal, so the order stays total.
        match (self.rows(), other.rows()) {
            (Ok(left), Ok(right)) => left.as_ref().cmp(right.as_ref()),
            (Ok(_), Err(_)) => return Ordering::Less,
            (Err(_), Ok(_)) => return Ordering::Greater,
            (Err(_), Err(_)) => Ordering::Equal,
        }
        // The length breaks the tie the rows leave when neither can be read:
        // a column hashes its field and its length, so two that compare
        // equal have to agree on both or `Eq` and `Hash` disagree.
        .then_with(|| self.len().cmp(&other.len()))
        .then_with(|| self.leaf_rank().cmp(&other.leaf_rank()))
        .then_with(|| match (self.field(), other.field()) {
            (Some(left), Some(right)) => left.cmp(right),
            _ => Ordering::Equal,
        })
    }
}

impl PartialOrd for Serie {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Hash for Serie {
    /// The leaf's own hash, never the discriminant: a schema-free run hashes
    /// exactly what it hashed before the family had a second leaf, which is
    /// what keeps every pinned stable hash byte-identical. A column hashes
    /// its field and its length, so a hash never reads a buffer.
    fn hash<H: Hasher>(&self, state: &mut H) {
        column!(
            self,
            run => run.hash(state),
            column => hash_leaf(column.as_ref(), state)
        );
    }
}

/// Name the mutable narrowing accessor for one leaf of the root.
///
/// The buffer writers live on the leaf, so this is how a caller holding the
/// root reaches them: [`Int64Serie::push_value`] and
/// [`Int64Serie::set_value`] write the values buffer in place, where
/// [`SerieValue::push`] and [`SerieValue::set`] read a value through the
/// field first.
macro_rules! narrow_mut {
    ($(#[$meta:meta])* $name:ident, $leaf:ty, $family:ident, $held:ident, $variant:ident) => {
        $(#[$meta])*
        pub fn $name(&mut self) -> Option<&mut $leaf> {
            match self {
                Self::$family(family) => match ::std::sync::Arc::make_mut(family) {
                    $held::$variant(column) => Some(column),
                    _ => None,
                },
                _ => None,
            }
        }
    };
}

/// Name the narrowing accessor for one leaf of the root.
macro_rules! narrow {
    ($(#[$meta:meta])* $name:ident, $leaf:ty, $family:ident, $held:ident, $variant:ident) => {
        $(#[$meta])*
        pub fn $name(&self) -> Option<&$leaf> {
            match self {
                Self::$family(family) => match family.as_ref() {
                    $held::$variant(column) => Some(column),
                    _ => None,
                },
                _ => None,
            }
        }
    };
}

impl Serie {
    /// Return the null column when this is that leaf.
    pub fn as_null(&self) -> Option<&NullSerie> {
        match self {
            Self::Null(column) => Some(column.as_ref()),
            _ => None,
        }
    }

    /// Return the boolean column when this is that leaf.
    pub fn as_boolean(&self) -> Option<&BooleanSerie> {
        match self {
            Self::Boolean(column) => Some(column.as_ref()),
            _ => None,
        }
    }

    /// Return the integer family when this column is one.
    pub fn as_integer(&self) -> Option<&IntegerSerie> {
        match self {
            Self::Integer(family) => Some(family.as_ref()),
            _ => None,
        }
    }

    /// Return the floating family when this column is one.
    pub fn as_floating(&self) -> Option<&FloatingSerie> {
        match self {
            Self::Floating(family) => Some(family.as_ref()),
            _ => None,
        }
    }

    /// Return the decimal family when this column is one.
    pub fn as_decimal(&self) -> Option<&DecimalSerie> {
        match self {
            Self::Decimal(family) => Some(family.as_ref()),
            _ => None,
        }
    }

    /// Return the temporal family when this column is one.
    pub fn as_temporal(&self) -> Option<&TemporalSerie> {
        match self {
            Self::Temporal(family) => Some(family.as_ref()),
            _ => None,
        }
    }

    /// Return the string family when this column is one.
    pub fn as_string(&self) -> Option<&StringSerie> {
        match self {
            Self::String(family) => Some(family.as_ref()),
            _ => None,
        }
    }

    /// Return the byte family when this column is one.
    pub fn as_bytes(&self) -> Option<&BytesSerie> {
        match self {
            Self::Bytes(family) => Some(family.as_ref()),
            _ => None,
        }
    }

    /// Return the record column when this is that leaf.
    pub fn as_struct(&self) -> Option<&StructSerie> {
        match self {
            Self::Struct(column) => Some(column.as_ref()),
            _ => None,
        }
    }

    /// Return the 32-bit-offset sequence column when this is that leaf.
    pub fn as_sequence(&self) -> Option<&SequenceSerie> {
        match self {
            Self::Sequence(column) => Some(column.as_ref()),
            _ => None,
        }
    }

    /// Return the 64-bit-offset sequence column when this is that leaf.
    pub fn as_large_sequence(&self) -> Option<&LargeSequenceSerie> {
        match self {
            Self::LargeSequence(column) => Some(column.as_ref()),
            _ => None,
        }
    }

    /// Return the mapping column when this is that leaf.
    pub fn as_mapping(&self) -> Option<&MappingSerie> {
        match self {
            Self::Mapping(column) => Some(column.as_ref()),
            _ => None,
        }
    }

    /// Return the variant column when this is that leaf.
    pub fn as_variant(&self) -> Option<&VariantSerie> {
        match self {
            Self::Variant(column) => Some(column.as_ref()),
            _ => None,
        }
    }

    narrow!(
        /// Return the signed 8-bit column when this is that leaf.
        as_int8, Int8Serie, Integer, IntegerSerie, Int8
    );
    narrow!(
        /// Return the signed 16-bit column when this is that leaf.
        as_int16, Int16Serie, Integer, IntegerSerie, Int16
    );
    narrow!(
        /// Return the signed 32-bit column when this is that leaf.
        as_int32, Int32Serie, Integer, IntegerSerie, Int32
    );
    narrow!(
        /// Return the signed 64-bit column when this is that leaf.
        as_int64, Int64Serie, Integer, IntegerSerie, Int64
    );
    narrow!(
        /// Return the unsigned 8-bit column when this is that leaf.
        as_uint8, UInt8Serie, Integer, IntegerSerie, UInt8
    );
    narrow!(
        /// Return the unsigned 16-bit column when this is that leaf.
        as_uint16, UInt16Serie, Integer, IntegerSerie, UInt16
    );
    narrow!(
        /// Return the unsigned 32-bit column when this is that leaf.
        as_uint32, UInt32Serie, Integer, IntegerSerie, UInt32
    );
    narrow!(
        /// Return the unsigned 64-bit column when this is that leaf.
        as_uint64, UInt64Serie, Integer, IntegerSerie, UInt64
    );
    narrow!(
        /// Return the binary32 column when this is that leaf.
        as_float32, Float32Serie, Floating, FloatingSerie, Float32
    );
    narrow!(
        /// Return the binary64 column when this is that leaf.
        as_float64, Float64Serie, Floating, FloatingSerie, Float64
    );
    narrow!(
        /// Return the UTF-8 column when this is that leaf.
        as_utf8, Utf8StringSerie, String, StringSerie, Utf8
    );
    narrow!(
        /// Return the 64-bit-offset UTF-8 column when this is that leaf.
        as_large_utf8, LargeUtf8StringSerie, String, StringSerie, LargeUtf8
    );
    narrow!(
        /// Return the plain byte column when this is that leaf.
        as_binary, BinarySerie, Bytes, BytesSerie, Binary
    );

    /// Return the null column to write when this is that leaf.
    pub fn as_null_mut(&mut self) -> Option<&mut NullSerie> {
        match self {
            Self::Null(held) => Some(Arc::make_mut(held)),
            _ => None,
        }
    }

    /// Return the boolean column to write when this is that leaf.
    pub fn as_boolean_mut(&mut self) -> Option<&mut BooleanSerie> {
        match self {
            Self::Boolean(held) => Some(Arc::make_mut(held)),
            _ => None,
        }
    }

    /// Return the struct column to write when this is that leaf.
    pub fn as_struct_mut(&mut self) -> Option<&mut StructSerie> {
        match self {
            Self::Struct(held) => Some(Arc::make_mut(held)),
            _ => None,
        }
    }

    /// Return the list column to write when this is that leaf.
    pub fn as_sequence_mut(&mut self) -> Option<&mut SequenceSerie> {
        match self {
            Self::Sequence(held) => Some(Arc::make_mut(held)),
            _ => None,
        }
    }

    /// Return the 64-bit-offset sequence column to write when this is that leaf.
    pub fn as_large_sequence_mut(&mut self) -> Option<&mut LargeSequenceSerie> {
        match self {
            Self::LargeSequence(held) => Some(Arc::make_mut(held)),
            _ => None,
        }
    }

    /// Return the map column to write when this is that leaf.
    pub fn as_mapping_mut(&mut self) -> Option<&mut MappingSerie> {
        match self {
            Self::Mapping(held) => Some(Arc::make_mut(held)),
            _ => None,
        }
    }

    /// Return the variant column to write when this is that leaf.
    pub fn as_variant_mut(&mut self) -> Option<&mut VariantSerie> {
        match self {
            Self::Variant(held) => Some(Arc::make_mut(held)),
            _ => None,
        }
    }

    /// Return the integer family to write when this column is one.
    pub fn as_integer_mut(&mut self) -> Option<&mut IntegerSerie> {
        match self {
            Self::Integer(held) => Some(Arc::make_mut(held)),
            _ => None,
        }
    }

    /// Return the floating family to write when this column is one.
    pub fn as_floating_mut(&mut self) -> Option<&mut FloatingSerie> {
        match self {
            Self::Floating(held) => Some(Arc::make_mut(held)),
            _ => None,
        }
    }

    /// Return the decimal family to write when this column is one.
    pub fn as_decimal_mut(&mut self) -> Option<&mut DecimalSerie> {
        match self {
            Self::Decimal(held) => Some(Arc::make_mut(held)),
            _ => None,
        }
    }

    /// Return the temporal family to write when this column is one.
    pub fn as_temporal_mut(&mut self) -> Option<&mut TemporalSerie> {
        match self {
            Self::Temporal(held) => Some(Arc::make_mut(held)),
            _ => None,
        }
    }

    /// Return the string family to write when this column is one.
    pub fn as_string_mut(&mut self) -> Option<&mut StringSerie> {
        match self {
            Self::String(held) => Some(Arc::make_mut(held)),
            _ => None,
        }
    }

    /// Return the bytes family to write when this column is one.
    pub fn as_bytes_mut(&mut self) -> Option<&mut BytesSerie> {
        match self {
            Self::Bytes(held) => Some(Arc::make_mut(held)),
            _ => None,
        }
    }

    narrow_mut!(
        /// Return the `Int8Serie` buffers to write when this is that leaf.
        as_int8_mut, Int8Serie, Integer, IntegerSerie, Int8
    );

    narrow_mut!(
        /// Return the `Int16Serie` buffers to write when this is that leaf.
        as_int16_mut, Int16Serie, Integer, IntegerSerie, Int16
    );

    narrow_mut!(
        /// Return the `Int32Serie` buffers to write when this is that leaf.
        as_int32_mut, Int32Serie, Integer, IntegerSerie, Int32
    );

    narrow_mut!(
        /// Return the `Int64Serie` buffers to write when this is that leaf.
        as_int64_mut, Int64Serie, Integer, IntegerSerie, Int64
    );

    narrow_mut!(
        /// Return the `UInt8Serie` buffers to write when this is that leaf.
        as_uint8_mut, UInt8Serie, Integer, IntegerSerie, UInt8
    );

    narrow_mut!(
        /// Return the `UInt16Serie` buffers to write when this is that leaf.
        as_uint16_mut, UInt16Serie, Integer, IntegerSerie, UInt16
    );

    narrow_mut!(
        /// Return the `UInt32Serie` buffers to write when this is that leaf.
        as_uint32_mut, UInt32Serie, Integer, IntegerSerie, UInt32
    );

    narrow_mut!(
        /// Return the `UInt64Serie` buffers to write when this is that leaf.
        as_uint64_mut, UInt64Serie, Integer, IntegerSerie, UInt64
    );

    narrow_mut!(
        /// Return the `Float32Serie` buffers to write when this is that leaf.
        as_float32_mut, Float32Serie, Floating, FloatingSerie, Float32
    );

    narrow_mut!(
        /// Return the `Float64Serie` buffers to write when this is that leaf.
        as_float64_mut, Float64Serie, Floating, FloatingSerie, Float64
    );

    narrow_mut!(
        /// Return the `Utf8StringSerie` buffers to write when this is that leaf.
        as_utf8_mut, Utf8StringSerie, String, StringSerie, Utf8
    );

    narrow_mut!(
        /// Return the `LargeUtf8StringSerie` buffers to write when this is that leaf.
        as_large_utf8_mut, LargeUtf8StringSerie, String, StringSerie, LargeUtf8
    );

    narrow_mut!(
        /// Return the `BinarySerie` buffers to write when this is that leaf.
        as_binary_mut, BinarySerie, Bytes, BytesSerie, Binary
    );
}

// ------------------------------------------------------------------------
// The serde shadow: a column is its field and its rows, which is the one
// thing the sequence wire cannot write for it.
// ------------------------------------------------------------------------

/// The private serde shadow of a [`Serie`].
#[derive(Deserialize, Serialize)]
struct SerieWire {
    field: Field,
    rows: Vec<Scalar>,
}

impl Serialize for Serie {
    /// A schema-free run writes its values; a column writes the field that
    /// types them beside those values, because that is the one thing the
    /// values cannot say for it.
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        use serde::ser::Error as _;

        let Some(field) = self.field() else {
            return match self {
                Self::List(run) => run.serialize(serializer),
                _ => unreachable!("only a run carries no field"),
            };
        };
        SerieWire {
            field: field.clone(),
            rows: self.scalars().map_err(S::Error::custom)?,
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Serie {
    /// Either wire shape, told apart by its own form: a column is the
    /// document a field and its rows make, a run is the values alone.
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        use serde::de::Error as _;

        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Wire {
            Column(SerieWire),
            Run(List),
        }

        Ok(match Wire::deserialize(deserializer)? {
            Wire::Column(wire) => {
                Self::from_scalars(wire.field, wire.rows).map_err(D::Error::custom)?
            }
            Wire::Run(run) => Self::List(run),
        })
    }
}

impl crate::value::Value for Serie {
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

impl crate::value::NestedValue for Serie {
    fn len(&self) -> usize {
        Self::len(self)
    }

    fn children(&self) -> crate::value::Children<'_> {
        match self {
            Self::List(run) => crate::value::Children::Sequence(run.as_slice().iter()),
            column => crate::value::Children::Column(crate::value::ColumnRows::new(column)),
        }
    }
}

impl From<Serie> for Scalar {
    fn from(value: Serie) -> Self {
        Self::Sequence(value)
    }
}

impl From<List> for Serie {
    fn from(value: List) -> Self {
        Self::List(value)
    }
}

// ------------------------------------------------------------------------
// Arrow: the buffers a column is, both directions. The field decides which
// leaf they land in, and nothing is copied on the way.
// ------------------------------------------------------------------------

mod arrow;
