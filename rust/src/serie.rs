//! Serie: one column, on the fourth side of the value model.
//!
//! One type, one file - and a folder beside it, because a column is as wide
//! as Arrow's layouts are. [`DataType`] says what shape a column has,
//! [`Field`] says whose it is and whether a row may be absent, and [`Scalar`]
//! is one row of it. A [`Serie`] is the rows themselves, held the way Arrow
//! holds them.
//!
//! | side | root | trait | widen | narrow |
//! | --- | --- | --- | --- | --- |
//! | datatype | [`DataType`] | [`DataTypeValue`] | `into_dtype` | `from_dtype` |
//! | field | [`Field`] | [`FieldValue`] | `into_field` | `from_field` |
//! | value | [`Scalar`] | [`Value`] | `into_scalar` | `from_scalar` |
//! | column | [`Serie`] | [`SerieValue`] | `into_serie` | `from_serie` |
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
//! | [`ListSerie`] | - | the offsets, and the item column under them |
//! | [`MapSerie`] | - | the offsets, and the entry column under them |
//! | [`VariantSerie`] | - | the encoded bytes of one row, decoded on demand |
//!
//! Every leaf is one [`ArrayRef`]'s buffers behind the field that types them,
//! so a column crosses into Arrow and back by sharing them -
//! [`Serie::from_arrow_array`] and [`SerieValue::into_arrow_array`] copy no
//! row - and a nested column crosses child by child, which is why
//! [`StructSerie::child`] answers a column rather than a projection.
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
//! A column of one field is a list of that field, which is a datatype the
//! sequence family already owns, so a serie registers there rather than as a
//! root of its own: [`Sequence::Serie`] carries it and
//! `Scalar::Sequence(Sequence::Serie(..))` is how a column is a value. It
//! adds no [`DataTypeId`](crate::DataTypeId), no [`DataType`] variant and no
//! [`Field`] variant.
//!
//! A sequence value still has to lend `&[Scalar]`, and a column holds none,
//! so the decode lives in that leaf rather than here:
//! [`SerieRows`](crate::SerieRows) builds the rows the first time one is
//! asked for and shares them from then on, and the column it reads is
//! unchanged by being read.
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

use std::cmp::Ordering;
use std::fmt;

use arrow_array::{Array, ArrayRef};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::sequence::Sequence;
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
    ($name:ident $(, $param:ident : $bound:ident)?) => {
        impl$(<$param: $bound>)? PartialEq for $name$(<$param>)? {
            fn eq(&self, other: &Self) -> bool {
                $crate::serie::compare_leaves(self, other) == ::std::cmp::Ordering::Equal
            }
        }

        impl$(<$param: $bound>)? Eq for $name$(<$param>)? {}

        impl$(<$param: $bound>)? PartialOrd for $name$(<$param>)? {
            fn partial_cmp(&self, other: &Self) -> Option<::std::cmp::Ordering> {
                Some(::std::cmp::Ord::cmp(self, other))
            }
        }

        impl$(<$param: $bound>)? Ord for $name$(<$param>)? {
            fn cmp(&self, other: &Self) -> ::std::cmp::Ordering {
                $crate::serie::compare_leaves(self, other)
            }
        }

        impl$(<$param: $bound>)? ::std::hash::Hash for $name$(<$param>)? {
            fn hash<H: ::std::hash::Hasher>(&self, state: &mut H) {
                $crate::serie::hash_leaf(self, state);
            }
        }

        impl$(<$param: $bound>)? ::std::fmt::Display for $name$(<$param>)? {
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
                Serie::$variant(self)
            }

            fn from_serie(value: &Serie) -> Option<&Self> {
                match value {
                    Serie::$variant(family) => Some(family),
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
                Self::$variant(value)
            }
        }
    };
}

mod nested;
mod primitive;
mod text;
mod variant;

pub use nested::{ListSerie, MapSerie, StructSerie};
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
    BinarySerie, BinaryStringSerie, BinaryViewSerie, BinaryViewStringSerie, ByteSerie,
    ByteViewSerie, FixedBytesSerie, FixedStringSerie, LargeBinarySerie, LargeBinaryStringSerie,
    LargeUtf8StringSerie, Utf8StringSerie, Utf8ViewStringSerie,
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

/// One column: the field that types its rows, and the Arrow buffers that
/// hold them.
///
/// One variant per family, each an enum over the leaves that share a value
/// reading - the same shape [`DataType`] has, for the same reason: a caller
/// branching on the family never asks which width it was stored at, and one
/// that wants the buffers narrows to the leaf and gets them typed.
#[derive(Clone)]
#[non_exhaustive]
pub enum Serie {
    /// A column of nulls: a length, and no buffer at all.
    Null(NullSerie),
    /// A column of booleans.
    Boolean(BooleanSerie),
    /// A column of integers, at any signed or unsigned width.
    Integer(IntegerSerie),
    /// A column of floats, at any IEEE width.
    Floating(FloatingSerie),
    /// A column of exact decimals, at any coefficient width.
    Decimal(DecimalSerie),
    /// A column of temporals, at the unit its field declares.
    Temporal(TemporalSerie),
    /// A column of text, or of one registered code.
    String(StringSerie),
    /// A column of bytes, or of one identity stored as bytes.
    Bytes(BytesSerie),
    /// A column of records, each child a column of its own.
    Struct(StructSerie),
    /// A column of lists, the items a column under the offsets.
    List(ListSerie),
    /// A column of mappings, the entries a column under the offsets.
    Map(MapSerie),
    /// A column of self-describing values, each one encoded run of bytes.
    Variant(VariantSerie),
}

/// Forward one verb from the root to whichever family holds the rows.
macro_rules! dispatch {
    ($self:ident, $column:ident, $answer:expr) => {
        match $self {
            Serie::Null($column) => $answer,
            Serie::Boolean($column) => $answer,
            Serie::Integer($column) => $answer,
            Serie::Floating($column) => $answer,
            Serie::Decimal($column) => $answer,
            Serie::Temporal($column) => $answer,
            Serie::String($column) => $answer,
            Serie::Bytes($column) => $answer,
            Serie::Struct($column) => $answer,
            Serie::List($column) => $answer,
            Serie::Map($column) => $answer,
            Serie::Variant($column) => $answer,
        }
    };
}

impl SerieValue for Serie {
    fn field(&self) -> &Field {
        dispatch!(self, column, column.field())
    }

    fn len(&self) -> usize {
        dispatch!(self, column, column.len())
    }

    fn null_count(&self) -> usize {
        dispatch!(self, column, column.null_count())
    }

    fn is_null(&self, index: usize) -> bool {
        dispatch!(self, column, column.is_null(index))
    }

    fn scalar(&self, index: usize) -> Result<Scalar> {
        dispatch!(self, column, column.scalar(index))
    }

    fn set(&mut self, index: usize, value: Scalar) -> Result<()> {
        dispatch!(self, column, column.set(index, value))
    }

    fn push(&mut self, value: Scalar) -> Result<()> {
        dispatch!(self, column, column.push(value))
    }

    fn into_arrow_array(&self) -> ArrayRef {
        dispatch!(self, column, column.into_arrow_array())
    }

    fn into_serie(self) -> Self {
        self
    }

    fn from_serie(value: &Self) -> Option<&Self> {
        Some(value)
    }
}

impl fmt::Debug for Serie {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        dispatch!(self, column, fmt::Debug::fmt(column, formatter))
    }
}

serie_leaf!(Serie);

impl Serie {
    /// Return the field every row of this column is typed by.
    pub fn field(&self) -> &Field {
        SerieValue::field(self)
    }

    /// Return the number of rows.
    pub fn len(&self) -> usize {
        SerieValue::len(self)
    }

    /// Return whether this column holds no rows.
    pub fn is_empty(&self) -> bool {
        SerieValue::is_empty(self)
    }

    /// Return how many rows hold no value.
    pub fn null_count(&self) -> usize {
        SerieValue::null_count(self)
    }

    /// Return whether row `index` holds no value.
    pub fn is_null(&self, index: usize) -> bool {
        SerieValue::is_null(self, index)
    }

    /// Build row `index` as a value, or [`Scalar::Null`] past the end.
    ///
    /// # Errors
    ///
    /// [`SerieValue::scalar`] carries the rule.
    pub fn scalar(&self, index: usize) -> Result<Scalar> {
        SerieValue::scalar(self, index)
    }

    /// Overwrite row `index`, through the field's contract.
    ///
    /// # Errors
    ///
    /// [`SerieValue::set`] carries the rule.
    pub fn set(&mut self, index: usize, value: Scalar) -> Result<()> {
        SerieValue::set(self, index, value)
    }

    /// Append one row, through the field's contract.
    ///
    /// # Errors
    ///
    /// [`SerieValue::push`] carries the rule.
    pub fn push(&mut self, value: Scalar) -> Result<()> {
        SerieValue::push(self, value)
    }

    /// Return the datatype this column materializes into.
    ///
    /// The field is carried, so this is a read rather than an agreement
    /// computed back out of the rows: an empty column names its datatype
    /// where an empty sequence cannot.
    ///
    /// # Errors
    ///
    /// Infallible today; the signature is [`crate::Value::dtype`]'s.
    pub fn dtype(&self) -> Result<DataType> {
        Ok(DataType::list(self.field().clone()))
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
        (0..self.len()).map(|index| self.scalar(index)).collect()
    }

    /// Return the rows as the schema-free sequence they are.
    ///
    /// The field is what a serie has that a sequence has not, so this is the
    /// one direction that drops something and it is spelled rather than
    /// implied.
    ///
    /// # Errors
    ///
    /// [`Self::scalars`] carries the rule.
    pub fn into_sequence(self) -> Result<Scalar> {
        Ok(Scalar::from_sequence(self.scalars()?))
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
        pub const fn $name(&mut self) -> Option<&mut $leaf> {
            match self {
                Self::$family($held::$variant(column)) => Some(column),
                _ => None,
            }
        }
    };
}

/// Name the narrowing accessor for one leaf of the root.
macro_rules! narrow {
    ($(#[$meta:meta])* $name:ident, $leaf:ty, $family:ident, $held:ident, $variant:ident) => {
        $(#[$meta])*
        pub const fn $name(&self) -> Option<&$leaf> {
            match self {
                Self::$family($held::$variant(column)) => Some(column),
                _ => None,
            }
        }
    };
}

impl Serie {
    /// Return the null column when this is that leaf.
    pub const fn as_null(&self) -> Option<&NullSerie> {
        match self {
            Self::Null(column) => Some(column),
            _ => None,
        }
    }

    /// Return the boolean column when this is that leaf.
    pub const fn as_boolean(&self) -> Option<&BooleanSerie> {
        match self {
            Self::Boolean(column) => Some(column),
            _ => None,
        }
    }

    /// Return the integer family when this column is one.
    pub const fn as_integer(&self) -> Option<&IntegerSerie> {
        match self {
            Self::Integer(family) => Some(family),
            _ => None,
        }
    }

    /// Return the floating family when this column is one.
    pub const fn as_floating(&self) -> Option<&FloatingSerie> {
        match self {
            Self::Floating(family) => Some(family),
            _ => None,
        }
    }

    /// Return the decimal family when this column is one.
    pub const fn as_decimal(&self) -> Option<&DecimalSerie> {
        match self {
            Self::Decimal(family) => Some(family),
            _ => None,
        }
    }

    /// Return the temporal family when this column is one.
    pub const fn as_temporal(&self) -> Option<&TemporalSerie> {
        match self {
            Self::Temporal(family) => Some(family),
            _ => None,
        }
    }

    /// Return the string family when this column is one.
    pub const fn as_string(&self) -> Option<&StringSerie> {
        match self {
            Self::String(family) => Some(family),
            _ => None,
        }
    }

    /// Return the byte family when this column is one.
    pub const fn as_bytes(&self) -> Option<&BytesSerie> {
        match self {
            Self::Bytes(family) => Some(family),
            _ => None,
        }
    }

    /// Return the record column when this is that leaf.
    pub const fn as_struct(&self) -> Option<&StructSerie> {
        match self {
            Self::Struct(column) => Some(column),
            _ => None,
        }
    }

    /// Return the list column when this is that leaf.
    pub const fn as_list(&self) -> Option<&ListSerie> {
        match self {
            Self::List(column) => Some(column),
            _ => None,
        }
    }

    /// Return the mapping column when this is that leaf.
    pub const fn as_map(&self) -> Option<&MapSerie> {
        match self {
            Self::Map(column) => Some(column),
            _ => None,
        }
    }

    /// Return the variant column when this is that leaf.
    pub const fn as_variant(&self) -> Option<&VariantSerie> {
        match self {
            Self::Variant(column) => Some(column),
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
    pub const fn as_null_mut(&mut self) -> Option<&mut NullSerie> {
        match self {
            Self::Null(column) => Some(column),
            _ => None,
        }
    }

    /// Return the boolean column to write when this is that leaf.
    pub const fn as_boolean_mut(&mut self) -> Option<&mut BooleanSerie> {
        match self {
            Self::Boolean(column) => Some(column),
            _ => None,
        }
    }

    /// Return the struct column to write when this is that leaf.
    pub const fn as_struct_mut(&mut self) -> Option<&mut StructSerie> {
        match self {
            Self::Struct(column) => Some(column),
            _ => None,
        }
    }

    /// Return the list column to write when this is that leaf.
    pub const fn as_list_mut(&mut self) -> Option<&mut ListSerie> {
        match self {
            Self::List(column) => Some(column),
            _ => None,
        }
    }

    /// Return the map column to write when this is that leaf.
    pub const fn as_map_mut(&mut self) -> Option<&mut MapSerie> {
        match self {
            Self::Map(column) => Some(column),
            _ => None,
        }
    }

    /// Return the variant column to write when this is that leaf.
    pub const fn as_variant_mut(&mut self) -> Option<&mut VariantSerie> {
        match self {
            Self::Variant(column) => Some(column),
            _ => None,
        }
    }

    /// Return the integer family to write when this column is one.
    pub const fn as_integer_mut(&mut self) -> Option<&mut IntegerSerie> {
        match self {
            Self::Integer(family) => Some(family),
            _ => None,
        }
    }

    /// Return the floating family to write when this column is one.
    pub const fn as_floating_mut(&mut self) -> Option<&mut FloatingSerie> {
        match self {
            Self::Floating(family) => Some(family),
            _ => None,
        }
    }

    /// Return the decimal family to write when this column is one.
    pub const fn as_decimal_mut(&mut self) -> Option<&mut DecimalSerie> {
        match self {
            Self::Decimal(family) => Some(family),
            _ => None,
        }
    }

    /// Return the temporal family to write when this column is one.
    pub const fn as_temporal_mut(&mut self) -> Option<&mut TemporalSerie> {
        match self {
            Self::Temporal(family) => Some(family),
            _ => None,
        }
    }

    /// Return the string family to write when this column is one.
    pub const fn as_string_mut(&mut self) -> Option<&mut StringSerie> {
        match self {
            Self::String(family) => Some(family),
            _ => None,
        }
    }

    /// Return the bytes family to write when this column is one.
    pub const fn as_bytes_mut(&mut self) -> Option<&mut BytesSerie> {
        match self {
            Self::Bytes(family) => Some(family),
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
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        use serde::ser::Error as _;

        SerieWire {
            field: self.field().clone(),
            rows: self.scalars().map_err(S::Error::custom)?,
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Serie {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        use serde::de::Error as _;

        let wire = SerieWire::deserialize(deserializer)?;
        Self::from_scalars(wire.field, wire.rows).map_err(D::Error::custom)
    }
}

impl From<Serie> for Scalar {
    fn from(value: Serie) -> Self {
        Self::Sequence(Sequence::from(value))
    }
}

// ------------------------------------------------------------------------
// Arrow: the buffers a column is, both directions. The field decides which
// leaf they land in, and nothing is copied on the way.
// ------------------------------------------------------------------------

mod arrow;
