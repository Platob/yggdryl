//! The ordering, uniqueness, grouping and windowing verbs every leaf
//! answers, through one ladder over one order.
//!
//! The order is the values': `Scalar`'s total order, every absent value -
//! a row, or one nested in a sequence, a record or a map - at the end the
//! options name, and every present one reversed when descending. Each rung
//! of the ladder answers exactly that order, so a column and the run of its
//! rows sort, deduplicate and group alike. A primitive column sorts its
//! native slice, stable, every NaN of a float one value above every number;
//! a column whose stored bytes order as its values - every leaf beneath its
//! field one whose storage is its value order, no float holding a NaN other
//! than the one a value reads - goes through Arrow's row format and
//! comparator; any other column - a version, a windows-1252 text, a
//! registered code, a URL, a union, a float holding a foreign NaN - and a run
//! go through the values' own order, each row built once. Uniqueness is one
//! hash set over the row format's bytes on the same rung, or over the values.
//! So every leaf answers every verb, and the cost is the ladder's rung,
//! stated on each.
//!
//! The reads answer a new serie and leave this one as it is; the `as_*`
//! writes bring this serie into the state in place, rewriting a uniquely
//! held primitive or boolean buffer where it stands and replacing any other
//! leaf's buffers by the kernel's one copy.

use std::borrow::Cow;
use std::cmp::Ordering;
use std::collections::HashSet;
use std::ops::Range;
use std::sync::Arc;

use arrow_array::{Array, ArrayRef, BooleanArray, UInt32Array};
use arrow_buffer::BooleanBuffer;
use arrow_data::ArrayData;
use arrow_ord::ord::{DynComparator, make_comparator};
use arrow_row::{RowConverter, Rows, SortField};
use arrow_schema::DataType as ArrowDataType;

use super::{Proof, Rows as _, Serie, land};
use crate::arrow::{array_memory_size, scalar_memory_size};
use crate::expression::{BoundSelector, IntoSelector};
use crate::{
    DataType, Error, Field, FieldPath, Result, Scalar, Selector, SerieReader, SerieWindows,
    SortOptions,
};

/// The name an index column answers: the positions a sort chose.
const INDEX_NAME: &str = "index";

/// The refusal a verb answers at this serie.
fn refuse(serie: &Serie, reason: smol_str::SmolStr) -> Error {
    Error::InvalidRecord {
        path: smol_str::SmolStr::new(serie.name()),
        reason,
    }
}

/// Refuse a row count one `uint32` index column cannot address.
fn require_indexable(serie: &Serie) -> Result<u32> {
    u32::try_from(serie.len()).map_err(|_| {
        refuse(
            serie,
            smol_str::format_smolstr!(
                "{} holds {} rows, past what one uint32 index column addresses",
                serie.name(),
                serie.len()
            ),
        )
    })
}

/// The `uint32` column of row positions `indices` names.
fn index_column(indices: Vec<u32>) -> Result<Serie> {
    let field = Arc::new(Field::new(INDEX_NAME, DataType::UInt32, false));
    Ok(land(
        field,
        Arc::new(UInt32Array::from(indices)),
        &Proof::Proven,
    )?)
}

/// `array`'s rows in Arrow's row format under `options`, where the format
/// has one for the layout: bytes whose order is the rows' order.
fn row_format(array: &ArrayRef, options: SortOptions) -> Option<Rows> {
    let field = SortField::new_with_options(array.data_type().clone(), options.into_arrow());
    if !RowConverter::supports_fields(std::slice::from_ref(&field)) {
        return None;
    }
    let converter = RowConverter::new(vec![field]).ok()?;
    converter.convert_columns(std::slice::from_ref(array)).ok()
}

/// Whether Arrow's comparator and row format over `dtype`'s storage order
/// and equate its values exactly as the values' own order does.
///
/// Nested layouts answer for their children, because Arrow places a nested
/// absence and reverses a nested value exactly as [`compare_values`] does.
/// A leaf answers `false` where its value is not its stored bytes read in
/// order: a version orders by its numbers, windows-1252 text by the
/// characters its bytes decode to, a registered code trims the padding a
/// foreign column may store, a URL, URN, zone, MIME or media type orders by
/// what it parses to, a union and a variant by more than their buffers say,
/// and a geospatial value by the WKB it validates to. A float leaf answers
/// `true` and leaves its NaN payloads to [`holds_foreign_nan`].
fn stored_order_is_value_order(dtype: &DataType) -> bool {
    match dtype {
        DataType::Null
        | DataType::Boolean
        | DataType::Int8
        | DataType::Int16
        | DataType::Int32
        | DataType::Int64
        | DataType::UInt8
        | DataType::UInt16
        | DataType::UInt32
        | DataType::UInt64
        | DataType::Float16
        | DataType::Float32
        | DataType::Float64
        | DataType::DateTime64 { .. }
        | DataType::Date32
        | DataType::Date64
        | DataType::Time32(_)
        | DataType::Time64(_)
        | DataType::Duration32(_)
        | DataType::Duration64(_)
        | DataType::Interval(_)
        | DataType::Decimal32 { .. }
        | DataType::Decimal64 { .. }
        | DataType::Decimal128 { .. }
        | DataType::Decimal256 { .. }
        | DataType::Decimal
        | DataType::BigDecimal
        // An enum member orders by its code, which is what the column
        // stores, and a UUID by its 128 bits, stored big-endian.
        | DataType::Side
        | DataType::State
        | DataType::TimeInForce
        | DataType::MarketDataKind
        | DataType::MarketDataType
        | DataType::Uuid
        | DataType::Binary
        | DataType::LargeBinary
        | DataType::BinaryView
        | DataType::LargeBinaryView
        | DataType::FixedBinary(_)
        | DataType::SizedBinary(_)
        // UTF-8 bytes order as the characters they spell, and a fixed slot
        // pads with NUL, the least byte, so the padding orders as the end.
        | DataType::Utf8String
        | DataType::LargeUtf8String
        | DataType::Utf8StringView
        | DataType::LargeUtf8StringView
        | DataType::FixedUtf8String(_)
        | DataType::SizedUtf8String(_)
        | DataType::AsciiString
        | DataType::LargeAsciiString
        | DataType::AsciiStringView
        | DataType::LargeAsciiStringView
        | DataType::FixedAsciiString(_)
        | DataType::SizedAsciiString(_) => true,
        DataType::Serie(item)
        | DataType::SerieView(item)
        | DataType::FixedSizeSerie(item, _)
        | DataType::LargeSerie(item)
        | DataType::LargeSerieView(item) => stored_order_is_value_order(item.dtype()),
        DataType::Struct(fields) => fields
            .iter()
            .all(|field| stored_order_is_value_order(field.dtype())),
        DataType::Map(map) | DataType::SortedMap(map) => {
            stored_order_is_value_order(map.entries().dtype())
        }
        DataType::Dictionary(dictionary) => stored_order_is_value_order(dictionary.value()),
        DataType::RunEndEncoded(encoded) => stored_order_is_value_order(encoded.values().dtype()),
        _ => false,
    }
}

/// Whether `values` holds a NaN of `$float` other than its positive quiet
/// NaN, the one a value reads every NaN as.
macro_rules! foreign_nan {
    ($values:expr, $float:ty) => {{
        let canonical = <$float>::NAN.to_bits();
        $values
            .iter()
            .any(|value| value.is_nan() && value.to_bits() != canonical)
    }};
}

/// Whether `array`, or any array beneath it, holds a float NaN other than
/// the one every value reads a NaN as - the positive quiet NaN of its width.
///
/// Arrow orders and encodes a NaN by its bits, where a value holds one NaN,
/// so a foreign payload is where the two orders part. A leaf is read off
/// its own buffer; a nested array whose layout holds no float is answered
/// from its datatype, and one that does is walked once.
fn holds_foreign_nan(array: &dyn Array) -> bool {
    use arrow_array::cast::AsArray as _;
    use arrow_array::types::{Float16Type, Float32Type, Float64Type};

    match array.data_type() {
        ArrowDataType::Float16 => {
            foreign_nan!(array.as_primitive::<Float16Type>().values(), half::f16)
        }
        ArrowDataType::Float32 => foreign_nan!(array.as_primitive::<Float32Type>().values(), f32),
        ArrowDataType::Float64 => foreign_nan!(array.as_primitive::<Float64Type>().values(), f64),
        dtype if layout_holds_float(dtype) => data_holds_foreign_nan(&array.to_data()),
        _ => false,
    }
}

/// Whether a layout of `dtype` has a float leaf anywhere beneath it.
fn layout_holds_float(dtype: &ArrowDataType) -> bool {
    match dtype {
        ArrowDataType::Float16 | ArrowDataType::Float32 | ArrowDataType::Float64 => true,
        ArrowDataType::List(item)
        | ArrowDataType::LargeList(item)
        | ArrowDataType::ListView(item)
        | ArrowDataType::LargeListView(item)
        | ArrowDataType::FixedSizeList(item, _)
        | ArrowDataType::Map(item, _) => layout_holds_float(item.data_type()),
        ArrowDataType::Struct(fields) => fields
            .iter()
            .any(|field| layout_holds_float(field.data_type())),
        ArrowDataType::Union(fields, _) => fields
            .iter()
            .any(|(_, field)| layout_holds_float(field.data_type())),
        ArrowDataType::Dictionary(_, values) => layout_holds_float(values),
        ArrowDataType::RunEndEncoded(_, values) => layout_holds_float(values.data_type()),
        _ => false,
    }
}

/// [`holds_foreign_nan`] over one array's data and its children's, every
/// slot read whether or not a row reaches it: a foreign NaN in a hidden
/// slot only sends the column to the values' order, which agrees anyway.
fn data_holds_foreign_nan(data: &ArrayData) -> bool {
    let len = data.len();
    let foreign = match data.data_type() {
        ArrowDataType::Float16 => foreign_nan!(data.buffer::<half::f16>(0)[..len], half::f16),
        ArrowDataType::Float32 => foreign_nan!(data.buffer::<f32>(0)[..len], f32),
        ArrowDataType::Float64 => foreign_nan!(data.buffer::<f64>(0)[..len], f64),
        _ => false,
    };
    foreign || data.child_data().iter().any(data_holds_foreign_nan)
}

/// How two rows of one serie compare under `options`: Arrow's comparator
/// over the buffers where they order as the values, the values' own order
/// over rows built once otherwise and for a run.
enum Compare<'a> {
    /// Arrow's comparator, which reads the buffers and builds no value.
    Buffers(DynComparator),
    /// The values' order over the rows: lent by a run, built once for a
    /// column.
    Values {
        rows: Cow<'a, [Scalar]>,
        options: SortOptions,
    },
}

impl<'a> Compare<'a> {
    fn new(serie: &'a Serie, options: SortOptions) -> Self {
        if let Some(array) = serie.ordered_buffers()
            && let Ok(compare) =
                make_comparator(array.as_ref(), array.as_ref(), options.into_arrow())
        {
            return Self::Buffers(compare);
        }
        Self::Values {
            rows: serie.rows(),
            options,
        }
    }

    fn cmp(&self, left: usize, right: usize) -> Ordering {
        match self {
            Self::Buffers(compare) => compare(left, right),
            Self::Values { rows, options } => compare_values(&rows[left], &rows[right], *options),
        }
    }

    /// Whether row `index` opens a run of equal rows: the first row, or one
    /// the row before it does not equal. The one boundary a sorted group
    /// and a window are both cut at.
    fn opens(&self, index: usize) -> bool {
        index == 0 || self.cmp(index - 1, index) != Ordering::Equal
    }
}

/// Where an absent value goes against a present one under `options`.
const fn absent_against_present(options: SortOptions) -> Ordering {
    if options.is_nulls_first() {
        Ordering::Less
    } else {
        Ordering::Greater
    }
}

/// `step` as `options` orders it: reversed when descending.
const fn directed(step: Ordering, options: SortOptions) -> Ordering {
    if options.is_descending() {
        step.reverse()
    } else {
        step
    }
}

/// Two values under `options`: an absent one at the end the options name,
/// two present ones in their own total order, reversed when descending.
///
/// A sequence - a serie value or a record's run - and a map compare item by
/// item under the same options, every nested absence placed as a top-level
/// one is, then the shorter first, reversed when descending: Arrow's own
/// reading of nested buffers, so a run and a column of the same rows agree.
fn compare_values(left: &Scalar, right: &Scalar, options: SortOptions) -> Ordering {
    match (left.is_null(), right.is_null()) {
        (true, true) => Ordering::Equal,
        (true, false) => absent_against_present(options),
        (false, true) => absent_against_present(options).reverse(),
        (false, false) => match (left, right) {
            (
                Scalar::Serie(left)
                | Scalar::SerieView(left)
                | Scalar::FixedSizeSerie(left)
                | Scalar::LargeSerie(left)
                | Scalar::LargeSerieView(left),
                Scalar::Serie(right)
                | Scalar::SerieView(right)
                | Scalar::FixedSizeSerie(right)
                | Scalar::LargeSerie(right)
                | Scalar::LargeSerieView(right),
            ) => {
                for index in 0..left.len().min(right.len()) {
                    let step = compare_values(&left.row_at(index), &right.row_at(index), options);
                    if step != Ordering::Equal {
                        return step;
                    }
                }
                directed(left.len().cmp(&right.len()), options)
            }
            (
                Scalar::Map(left) | Scalar::SortedMap(left),
                Scalar::Map(right) | Scalar::SortedMap(right),
            ) => {
                let (left, right) = (left.as_slice(), right.as_slice());
                for ((left_key, left_value), (right_key, right_value)) in left.iter().zip(right) {
                    let step = compare_values(left_key, right_key, options)
                        .then_with(|| compare_values(left_value, right_value, options));
                    if step != Ordering::Equal {
                        return step;
                    }
                }
                directed(left.len().cmp(&right.len()), options)
            }
            _ => directed(left.cmp(right), options),
        },
    }
}

/// Forward one verb to the primitive leaf holding the rows, `None` for any
/// other leaf: the typed rung of the ladder.
macro_rules! primitive {
    ($self:ident, $column:ident => $answer:expr) => {
        match $self {
            Serie::Int8($column) => Some($answer),
            Serie::Int16($column) => Some($answer),
            Serie::Int32($column) => Some($answer),
            Serie::Int64($column) => Some($answer),
            Serie::UInt8($column)
            | Serie::Side($column)
            | Serie::MarketDataKind($column)
            | Serie::TimeInForce($column) => Some($answer),
            Serie::UInt16($column) | Serie::State($column) | Serie::MarketDataType($column) => {
                Some($answer)
            }
            Serie::UInt32($column) => Some($answer),
            Serie::UInt64($column) => Some($answer),
            Serie::Float16($column) => Some($answer),
            Serie::Float32($column) => Some($answer),
            Serie::Float64($column) => Some($answer),
            Serie::DateTimeSecond($column) => Some($answer),
            Serie::DateTimeMillisecond($column) => Some($answer),
            Serie::DateTimeMicrosecond($column) => Some($answer),
            Serie::DateTimeNanosecond($column) => Some($answer),
            Serie::Date32($column) => Some($answer),
            Serie::Date64($column) => Some($answer),
            Serie::Time32Second($column) => Some($answer),
            Serie::Time32Millisecond($column) => Some($answer),
            Serie::Time64Microsecond($column) => Some($answer),
            Serie::Time64Nanosecond($column) => Some($answer),
            Serie::DurationSecond($column) => Some($answer),
            Serie::DurationMillisecond($column) => Some($answer),
            Serie::DurationMicrosecond($column) => Some($answer),
            Serie::DurationNanosecond($column) => Some($answer),
            Serie::IntervalYearMonth($column) => Some($answer),
            Serie::IntervalDayTime($column) => Some($answer),
            Serie::IntervalMonthDayNano($column) => Some($answer),
            Serie::Decimal32($column) => Some($answer),
            Serie::Decimal64($column) => Some($answer),
            Serie::Decimal128($column) => Some($answer),
            Serie::Decimal256($column) => Some($answer),
            _ => None,
        }
    };
}

/// The same, where the verb writes the leaf: copied once when shared.
macro_rules! primitive_mut {
    ($self:ident, $column:ident => $answer:expr) => {
        match $self {
            Serie::Int8(held) => {
                let $column = Arc::make_mut(held);
                Some($answer)
            }
            Serie::Int16(held) => {
                let $column = Arc::make_mut(held);
                Some($answer)
            }
            Serie::Int32(held) => {
                let $column = Arc::make_mut(held);
                Some($answer)
            }
            Serie::Int64(held) => {
                let $column = Arc::make_mut(held);
                Some($answer)
            }
            Serie::UInt8(held)
            | Serie::Side(held)
            | Serie::MarketDataKind(held)
            | Serie::TimeInForce(held) => {
                let $column = Arc::make_mut(held);
                Some($answer)
            }
            Serie::UInt16(held) | Serie::State(held) | Serie::MarketDataType(held) => {
                let $column = Arc::make_mut(held);
                Some($answer)
            }
            Serie::UInt32(held) => {
                let $column = Arc::make_mut(held);
                Some($answer)
            }
            Serie::UInt64(held) => {
                let $column = Arc::make_mut(held);
                Some($answer)
            }
            Serie::Float16(held) => {
                let $column = Arc::make_mut(held);
                Some($answer)
            }
            Serie::Float32(held) => {
                let $column = Arc::make_mut(held);
                Some($answer)
            }
            Serie::Float64(held) => {
                let $column = Arc::make_mut(held);
                Some($answer)
            }
            Serie::DateTimeSecond(held) => {
                let $column = Arc::make_mut(held);
                Some($answer)
            }
            Serie::DateTimeMillisecond(held) => {
                let $column = Arc::make_mut(held);
                Some($answer)
            }
            Serie::DateTimeMicrosecond(held) => {
                let $column = Arc::make_mut(held);
                Some($answer)
            }
            Serie::DateTimeNanosecond(held) => {
                let $column = Arc::make_mut(held);
                Some($answer)
            }
            Serie::Date32(held) => {
                let $column = Arc::make_mut(held);
                Some($answer)
            }
            Serie::Date64(held) => {
                let $column = Arc::make_mut(held);
                Some($answer)
            }
            Serie::Time32Second(held) => {
                let $column = Arc::make_mut(held);
                Some($answer)
            }
            Serie::Time32Millisecond(held) => {
                let $column = Arc::make_mut(held);
                Some($answer)
            }
            Serie::Time64Microsecond(held) => {
                let $column = Arc::make_mut(held);
                Some($answer)
            }
            Serie::Time64Nanosecond(held) => {
                let $column = Arc::make_mut(held);
                Some($answer)
            }
            Serie::DurationSecond(held) => {
                let $column = Arc::make_mut(held);
                Some($answer)
            }
            Serie::DurationMillisecond(held) => {
                let $column = Arc::make_mut(held);
                Some($answer)
            }
            Serie::DurationMicrosecond(held) => {
                let $column = Arc::make_mut(held);
                Some($answer)
            }
            Serie::DurationNanosecond(held) => {
                let $column = Arc::make_mut(held);
                Some($answer)
            }
            Serie::IntervalYearMonth(held) => {
                let $column = Arc::make_mut(held);
                Some($answer)
            }
            Serie::IntervalDayTime(held) => {
                let $column = Arc::make_mut(held);
                Some($answer)
            }
            Serie::IntervalMonthDayNano(held) => {
                let $column = Arc::make_mut(held);
                Some($answer)
            }
            Serie::Decimal32(held) => {
                let $column = Arc::make_mut(held);
                Some($answer)
            }
            Serie::Decimal64(held) => {
                let $column = Arc::make_mut(held);
                Some($answer)
            }
            Serie::Decimal128(held) => {
                let $column = Arc::make_mut(held);
                Some($answer)
            }
            Serie::Decimal256(held) => {
                let $column = Arc::make_mut(held);
                Some($answer)
            }
            _ => None,
        }
    };
}

impl Serie {
    // --------------------------------------------------------------------
    // Reads: a new serie, this one untouched.
    // --------------------------------------------------------------------

    /// The row positions in sorted order under `options`, as a `uint32`
    /// column named `index`: stable, so equal rows keep their order.
    ///
    /// The order is the values': `Scalar`'s total order, with every absent
    /// value, a row or one nested inside it, at the end `options` names. A
    /// primitive column sorts its native slice, every NaN one value; a
    /// column whose stored bytes order as its values sorts through Arrow's
    /// row format, one allocation beside the answer for the format's bytes;
    /// a run and any other column (a version, windows-1252 text, a code, a
    /// URL, a union, a float holding a foreign NaN payload) sort through
    /// the values' own order, each row built once.
    ///
    /// ```
    /// use yggdryl::{Scalar, Serie, SortOptions};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let prices = Serie::new(vec![Scalar::from(3_i64), Scalar::Null, Scalar::from(1_i64)]);
    /// let order = prices.sort_indices(SortOptions::default())?;
    /// assert_eq!(order.rows().to_vec(), vec![Scalar::from(2_u32), Scalar::from(0_u32), Scalar::from(1_u32)]);
    /// let order = prices.sort_indices(SortOptions::descending().with_nulls_first(true))?;
    /// assert_eq!(order.rows().to_vec(), vec![Scalar::from(1_u32), Scalar::from(0_u32), Scalar::from(2_u32)]);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error naming the serie when it holds more rows than one
    /// `uint32` index column addresses.
    pub fn sort_indices(&self, options: SortOptions) -> Result<Self> {
        index_column(self.sorted_order(options)?)
    }

    /// The positions [`Self::sort_indices`] answers, as the vector it
    /// builds.
    pub(crate) fn sorted_order(&self, options: SortOptions) -> Result<Vec<u32>> {
        let len = require_indexable(self)? as usize;
        if let Some(order) = primitive!(self, column => column.sort_indices(0..len, options)) {
            return Ok(order);
        }
        let mut order: Vec<u32> = (0..len as u32).collect();
        if let Some(array) = self.ordered_buffers()
            && let Some(rows) = row_format(&array, options)
        {
            order.sort_by(|left, right| rows.row(*left as usize).cmp(&rows.row(*right as usize)));
            return Ok(order);
        }
        let compare = Compare::new(self, options);
        order.sort_by(|left, right| compare.cmp(*left as usize, *right as usize));
        Ok(order)
    }

    /// This column's buffers, where Arrow's comparator and row format over
    /// them order and equate the rows exactly as their values do: every leaf
    /// beneath the field one whose stored order is its value order, and no
    /// float holding a NaN a value reads as another. `None` for a run and for
    /// any other column, which the values' own order answers.
    fn ordered_buffers(&self) -> Option<ArrayRef> {
        if !stored_order_is_value_order(self.field()?.dtype()) {
            return None;
        }
        let array = self.into_arrow_array()?;
        (!holds_foreign_nan(array.as_ref())).then_some(array)
    }

    /// Row `left` of this serie against row `right` of `other`, a serie
    /// under the same field, under `options`: Arrow's comparator over both
    /// buffers where both order as their values - one comparator built - and
    /// the values' own order otherwise, one row built per side.
    pub(crate) fn compare_across(
        &self,
        left: usize,
        other: &Self,
        right: usize,
        options: SortOptions,
    ) -> Ordering {
        if let (Some(left_array), Some(right_array)) =
            (self.ordered_buffers(), other.ordered_buffers())
            && let Ok(compare) = make_comparator(
                left_array.as_ref(),
                right_array.as_ref(),
                options.into_arrow(),
            )
        {
            return compare(left, right);
        }
        compare_values(&self.row_at(left), &other.row_at(right), options)
    }

    /// Whether the rows are in sorted order under `options`: one pass over
    /// adjacent rows, through Arrow's comparator over the buffers where they
    /// order as the values and the values' own order elsewhere, so such a
    /// column builds no row, any other builds each row once, and a run reads
    /// what it holds.
    ///
    /// ```
    /// use yggdryl::{Scalar, Serie, SortOptions};
    ///
    /// let prices = Serie::new(vec![Scalar::from(1_i64), Scalar::from(2_i64), Scalar::Null]);
    /// assert!(prices.is_sorted(SortOptions::default()));
    /// assert!(!prices.is_sorted(SortOptions::descending()));
    /// assert!(!prices.is_sorted(SortOptions::ascending().with_nulls_first(true)));
    /// ```
    pub fn is_sorted(&self, options: SortOptions) -> bool {
        if let Some(array) = self.ordered_buffers()
            && let Ok(compare) =
                make_comparator(array.as_ref(), array.as_ref(), options.into_arrow())
        {
            return (1..self.len()).all(|index| compare(index - 1, index) != Ordering::Greater);
        }
        let mut rows = self.iter();
        let Some(mut before) = rows.next() else {
            return true;
        };
        for row in rows {
            if compare_values(&before, &row, options) == Ordering::Greater {
                return false;
            }
            before = row;
        }
        true
    }

    /// Walk the rows, telling `visit` whether each is the first of its
    /// value; `visit` answers whether to go on. One hash set over the row
    /// format's bytes where the buffers order as the values, over the values
    /// elsewhere; an absent row is one value.
    fn walk_distinct(&self, mut visit: impl FnMut(usize, bool) -> bool) {
        let len = self.len();
        if let Some(array) = self.ordered_buffers()
            && let Some(rows) = row_format(&array, SortOptions::default())
        {
            let mut seen: HashSet<&[u8]> = HashSet::with_capacity(len);
            for index in 0..len {
                if !visit(index, seen.insert(rows.row(index).data())) {
                    return;
                }
            }
            return;
        }
        // `Scalar`'s hash reads canonical content only, never the
        // interior-mutable caches a datatype holds, so the key is stable.
        #[allow(clippy::mutable_key_type)]
        let mut seen: HashSet<Scalar> = HashSet::with_capacity(len);
        for index in 0..len {
            if !visit(index, seen.insert(self.row_at(index).into_owned())) {
                return;
            }
        }
    }

    /// Whether no two rows hold one value; an absent row is one value, so
    /// two of them are a repeat. Stops at the first repeat.
    ///
    /// ```
    /// use yggdryl::{Scalar, Serie};
    ///
    /// assert!(Serie::new(vec![Scalar::from(1_i64), Scalar::Null]).is_unique());
    /// assert!(!Serie::new(vec![Scalar::Null, Scalar::Null]).is_unique());
    /// ```
    pub fn is_unique(&self) -> bool {
        let mut unique = true;
        self.walk_distinct(|_, first| {
            unique = first;
            first
        });
        unique
    }

    /// How many distinct values the rows hold, an absent row one of them.
    ///
    /// ```
    /// use yggdryl::{Scalar, Serie};
    ///
    /// let venues = Serie::new(vec![Scalar::from("XNAS"), Scalar::Null, Scalar::from("XNAS")]);
    /// assert_eq!(venues.unique_count(), 2);
    /// ```
    pub fn unique_count(&self) -> usize {
        let mut count = 0;
        self.walk_distinct(|_, first| {
            count += usize::from(first);
            true
        });
        count
    }

    /// The mask of first occurrences: `true` where a row is the first of
    /// its value.
    fn first_occurrences(&self) -> Vec<bool> {
        let mut mask = Vec::with_capacity(self.len());
        self.walk_distinct(|_, first| {
            mask.push(first);
            true
        });
        mask
    }

    /// The rows in sorted order under `options`, this serie untouched:
    /// [`Self::sort_indices`] then [`Self::into_taken`].
    ///
    /// ```
    /// use yggdryl::{Scalar, Serie, SortOptions};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let prices = Serie::new(vec![Scalar::from(3_i64), Scalar::from(1_i64), Scalar::from(2_i64)]);
    /// let sorted = prices.into_sorted(SortOptions::descending())?;
    /// assert_eq!(sorted.rows().to_vec(), vec![Scalar::from(3_i64), Scalar::from(2_i64), Scalar::from(1_i64)]);
    /// assert_eq!(prices.scalar(0)?, Scalar::from(3_i64));
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// [`Self::sort_indices`]'s refusal.
    pub fn into_sorted(&self, options: SortOptions) -> Result<Self> {
        let order = self.sorted_order(options)?;
        self.taken(&order)
    }

    /// The first occurrence of every value, in order of first occurrence;
    /// an absent row is one value. The field is kept as it is.
    ///
    /// ```
    /// use yggdryl::{Scalar, Serie};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let venues = Serie::new(vec![Scalar::from("XNYS"), Scalar::from("XNAS"), Scalar::from("XNYS")]);
    /// assert_eq!(venues.into_unique()?.rows().to_vec(), vec![Scalar::from("XNYS"), Scalar::from("XNAS")]);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error when the filter kernel refuses the layout, naming
    /// the serie.
    pub fn into_unique(&self) -> Result<Self> {
        let mask = self.first_occurrences();
        self.filtered(&mask)
    }

    /// The rows in reverse order, this serie untouched: one kernel pass for
    /// a column, a copy for a run.
    ///
    /// ```
    /// use yggdryl::{Scalar, Serie};
    ///
    /// let prices = Serie::new(vec![Scalar::from(1_i64), Scalar::from(2_i64)]);
    /// assert_eq!(prices.into_reversed().rows().to_vec(), vec![Scalar::from(2_i64), Scalar::from(1_i64)]);
    /// ```
    pub fn into_reversed(&self) -> Self {
        let len = self.len();
        if let Self::Run(run) = self {
            let mut values = run.as_slice().to_vec();
            values.reverse();
            return Self::new(values);
        }
        let order: Vec<u32> = (0..len as u32).rev().collect();
        // A column's take refuses only an index past the end or a count past
        // u32, and this is neither: a column over u32::MAX rows reversed by
        // rows instead.
        if len > u32::MAX as usize {
            return Self::new(self.rows().iter().rev().cloned().collect::<Vec<_>>());
        }
        self.taken(&order)
            .expect("a reversal names every row once, which the take accepts")
    }

    /// The rows `indices` names, in that order, this serie untouched:
    /// Arrow's take over the buffers for a column, a copy of the chosen
    /// values for a run. `indices` is an integer column or run of any
    /// width.
    ///
    /// ```
    /// use yggdryl::{Scalar, Serie};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let prices = Serie::new(vec![Scalar::from(1_i64), Scalar::from(2_i64), Scalar::from(3_i64)]);
    /// let picked = Serie::new(vec![Scalar::from(2_u32), Scalar::from(0_u32), Scalar::from(2_u32)]);
    /// assert_eq!(
    ///     prices.into_taken(&picked)?.rows().to_vec(),
    ///     vec![Scalar::from(3_i64), Scalar::from(1_i64), Scalar::from(3_i64)]
    /// );
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error naming the serie when an index is absent, not an
    /// integer, negative or past the end.
    pub fn into_taken(&self, indices: &Self) -> Result<Self> {
        let order = self.take_order(indices)?;
        self.taken(&order)
    }

    /// `indices` read as row positions of this serie, each checked.
    fn take_order(&self, indices: &Self) -> Result<Vec<u32>> {
        let len = self.len();
        let mut order = Vec::with_capacity(indices.len());
        for (at, index) in indices.iter().enumerate() {
            let position = index
                .as_u128()
                .and_then(|value| u32::try_from(value).ok())
                .filter(|value| (*value as usize) < len)
                .ok_or_else(|| {
                    refuse(
                        self,
                        smol_str::format_smolstr!(
                            "index {at} is {index:?}, which names no row of the {len} {} holds",
                            self.name()
                        ),
                    )
                })?;
            order.push(position);
        }
        Ok(order)
    }

    /// The rows at `order`, every position already checked.
    pub(crate) fn taken(&self, order: &[u32]) -> Result<Self> {
        match self {
            Self::Run(run) => {
                let values = run.as_slice();
                Ok(Self::new(
                    order
                        .iter()
                        .map(|index| values[*index as usize].clone())
                        .collect::<Vec<_>>(),
                ))
            }
            column => {
                let field = Arc::clone(column.field_ref().expect("a column carries its field"));
                let array = column
                    .into_arrow_array()
                    .expect("a column lays out buffers");
                let indices = UInt32Array::from(order.to_vec());
                let taken = arrow_select::take::take(array.as_ref(), &indices, None)
                    .map_err(Error::Arrow)?;
                if taken.len() != order.len() {
                    // Arrow's take answers a zero-width fixed-size list by its
                    // child, which has no rows to count: the rows are read.
                    return Self::from_scalars(
                        field,
                        order
                            .iter()
                            .map(|index| column.row_at(*index as usize).into_owned()),
                    );
                }
                Ok(land(field, taken, &Proof::Proven)?)
            }
        }
    }

    /// The rows `mask` keeps, this serie untouched: Arrow's filter over the
    /// buffers for a column, a copy of the kept values for a run. `mask` is
    /// a boolean column or run of the same length; an absent mask row keeps
    /// nothing.
    ///
    /// ```
    /// use yggdryl::{Scalar, Serie};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let prices = Serie::new(vec![Scalar::from(1_i64), Scalar::from(2_i64), Scalar::from(3_i64)]);
    /// let mask = Serie::new(vec![Scalar::from(true), Scalar::Null, Scalar::from(true)]);
    /// assert_eq!(prices.into_filtered(&mask)?.rows().to_vec(), vec![Scalar::from(1_i64), Scalar::from(3_i64)]);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error naming the serie when `mask` is another length or
    /// holds a row that is neither a boolean nor absent.
    pub fn into_filtered(&self, mask: &Self) -> Result<Self> {
        if mask.len() != self.len() {
            return Err(refuse(
                self,
                smol_str::format_smolstr!(
                    "a mask of {} rows cannot filter the {} rows {} holds",
                    mask.len(),
                    self.len(),
                    self.name()
                ),
            ));
        }
        let mut keep = Vec::with_capacity(mask.len());
        for (at, row) in mask.iter().enumerate() {
            keep.push(match row.as_ref() {
                Scalar::Null => false,
                other => other.as_bool().ok_or_else(|| {
                    refuse(
                        self,
                        smol_str::format_smolstr!(
                            "mask row {at} is {other:?}, which is neither a boolean nor absent"
                        ),
                    )
                })?,
            });
        }
        self.filtered(&keep)
    }

    /// The rows `keep` marks, which is as long as this serie.
    pub(crate) fn filtered(&self, keep: &[bool]) -> Result<Self> {
        match self {
            Self::Run(run) => Ok(Self::new(
                run.as_slice()
                    .iter()
                    .zip(keep)
                    .filter(|(_, kept)| **kept)
                    .map(|(value, _)| value.clone())
                    .collect::<Vec<_>>(),
            )),
            column => {
                let field = Arc::clone(column.field_ref().expect("a column carries its field"));
                let array = column
                    .into_arrow_array()
                    .expect("a column lays out buffers");
                let mask = BooleanArray::from(keep.to_vec());
                let kept =
                    arrow_select::filter::filter(array.as_ref(), &mask).map_err(Error::Arrow)?;
                Ok(land(field, kept, &Proof::Proven)?)
            }
        }
    }

    /// The rows grouped by `keys`, a serie of the same length: one `(key,
    /// rows)` per distinct key value, in order of first occurrence, an
    /// absent key one value. Sorted keys cut every group as a zero-copy
    /// slice; any other keys take each group's rows once. A record column
    /// partitions by one of its children through `child("venue")`, by
    /// several through [`Self::partition_by_paths`].
    ///
    /// ```
    /// use yggdryl::{Scalar, Serie};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let prices = Serie::new(vec![Scalar::from(1_i64), Scalar::from(2_i64), Scalar::from(3_i64)]);
    /// let venues = Serie::new(vec![Scalar::from("XNAS"), Scalar::from("XNYS"), Scalar::from("XNAS")]);
    /// let groups = prices.partition_by(&venues)?;
    /// assert_eq!(groups.len(), 2);
    /// assert_eq!(groups[0].0, Scalar::from("XNAS"));
    /// assert_eq!(groups[0].1.rows().to_vec(), vec![Scalar::from(1_i64), Scalar::from(3_i64)]);
    /// assert_eq!(groups[1].0, Scalar::from("XNYS"));
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error naming the serie when `keys` is another length.
    pub fn partition_by(&self, keys: &Self) -> Result<Vec<(Scalar, Self)>> {
        if keys.len() != self.len() {
            return Err(refuse(
                self,
                smol_str::format_smolstr!(
                    "{} keys cannot partition the {} rows {} holds",
                    keys.len(),
                    self.len(),
                    self.name()
                ),
            ));
        }
        let len = self.len();
        // One comparator says whether the keys are sorted and, when they
        // are, where each group ends.
        let compare = Compare::new(keys, SortOptions::default());
        if (1..len).all(|index| compare.cmp(index - 1, index) != Ordering::Greater) {
            let mut groups = Vec::new();
            let mut start = 0;
            for index in 1..=len {
                if index == len || compare.opens(index) {
                    groups.push((keys.scalar(start)?, self.slice(start, index - start)?));
                    start = index;
                }
            }
            return Ok(groups);
        }
        let mut groups = Vec::new();
        for order in keys.groups()? {
            groups.push((keys.scalar(order[0] as usize)?, self.taken(&order)?));
        }
        Ok(groups)
    }

    /// The row positions of every distinct value, one group per value in
    /// order of first occurrence, each group in row order: one pass over
    /// one map, keyed by the row format's bytes where the buffers order as
    /// the values and by the values elsewhere, that names each row's group,
    /// then each group laid out at its exact size - so what this costs
    /// follows the groups and never the rows.
    fn groups(&self) -> Result<Vec<Vec<u32>>> {
        require_indexable(self)?;
        let len = self.len();
        let mut group_of_row: Vec<u32> = Vec::with_capacity(len);
        let mut sizes: Vec<usize> = Vec::new();
        let mut assign = |group: usize| {
            if group == sizes.len() {
                sizes.push(0);
            }
            sizes[group] += 1;
            group_of_row.push(group as u32);
        };
        if let Some(array) = self.ordered_buffers()
            && let Some(rows) = row_format(&array, SortOptions::default())
        {
            let mut group_of: std::collections::HashMap<&[u8], usize> =
                std::collections::HashMap::new();
            for index in 0..len {
                let next = group_of.len();
                assign(*group_of.entry(rows.row(index).data()).or_insert(next));
            }
        } else {
            // `Scalar`'s hash reads canonical content only, never the
            // interior-mutable caches a datatype holds, so the key is stable.
            #[allow(clippy::mutable_key_type)]
            let mut group_of: std::collections::HashMap<Scalar, usize> =
                std::collections::HashMap::new();
            for row in self.iter() {
                let next = group_of.len();
                assign(*group_of.entry(row.into_owned()).or_insert(next));
            }
        }
        let mut positions: Vec<Vec<u32>> =
            sizes.iter().map(|size| Vec::with_capacity(*size)).collect();
        for (index, group) in group_of_row.into_iter().enumerate() {
            positions[group as usize].push(index as u32);
        }
        Ok(positions)
    }

    /// The rows of a record column grouped by the cells `paths` reach: one
    /// group per distinct combination, keyed by the run of those cells in
    /// `paths` order, exactly as [`Self::partition_by`] groups by one key.
    ///
    /// ```
    /// use yggdryl::{DataType, Field, FieldPath, Scalar, Serie, StructType};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let root = Field::new(
    ///     "quote",
    ///     DataType::from(StructType::from_fields([
    ///         Field::new("venue", DataType::utf8(), false),
    ///         Field::new("side", DataType::utf8(), false),
    ///         Field::new("price", DataType::Int64, false),
    ///     ])?),
    ///     false,
    /// );
    /// let quotes = Serie::from_scalars(root, [
    ///     Scalar::from_sequence([Scalar::from("XNAS"), Scalar::from("B"), Scalar::from(1_i64)]),
    ///     Scalar::from_sequence([Scalar::from("XNAS"), Scalar::from("S"), Scalar::from(2_i64)]),
    ///     Scalar::from_sequence([Scalar::from("XNAS"), Scalar::from("B"), Scalar::from(3_i64)]),
    /// ])?;
    /// let groups = quotes.partition_by_paths(&["venue".parse::<FieldPath>()?, "side".parse()?])?;
    /// assert_eq!(groups.len(), 2);
    /// assert_eq!(groups[0].0, Scalar::from_sequence([Scalar::from("XNAS"), Scalar::from("B")]));
    /// assert_eq!(groups[0].1.len(), 2);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error naming the serie when it is a run or not a record
    /// column, when `paths` is empty, or when a path reaches no column.
    pub fn partition_by_paths(&self, paths: &[FieldPath]) -> Result<Vec<(Scalar, Self)>> {
        let Self::Struct(_) = self else {
            return Err(self.not_a_record("partitions by no path"));
        };
        if paths.is_empty() {
            return Err(refuse(
                self,
                smol_str::format_smolstr!(
                    "{} partitions by no path: pass at least one",
                    self.name()
                ),
            ));
        }
        let mut children = Vec::with_capacity(paths.len());
        for path in paths {
            let child = self.get_child_by_path(path).ok_or_else(|| {
                refuse(
                    self,
                    smol_str::format_smolstr!("{path} reaches no column of {}", self.name()),
                )
            })?;
            children.push(child.clone());
        }
        let keys = Self::record_of(children)?;
        self.partition_by(&keys)
    }

    /// The rows cut into windows of equal adjacent keys, the keys `by`
    /// computes from each row: one `(key, window)` per maximal run of
    /// adjacent rows whose keys are equal, in row order, every window a view
    /// over this serie. The windows are never empty, never overlap, and
    /// cover every row; a key that comes back after another opens a window
    /// of its own, where [`Self::partition_by`] gathers every row of a key
    /// into one group - over keys already in order the two agree.
    ///
    /// `by` is a selector - a clause text such as `"venue, minutes(ts, 15)
    /// as bucket"`, or a [`Selector`], a projection, a term or a path -
    /// parsed once and bound once against
    /// [`SerieReader::root_of`]: a record column binds against its own
    /// field, any other column as the one child of its record, under its own
    /// name. Names fold ASCII case, as every expression's do. A `*` beside
    /// projections keys by every column it keeps, then the projections.
    ///
    /// A key is the run of its projected cells at a window's first row, in
    /// selector order - one term keys a one-cell run. An absent record row
    /// keys [`Scalar::Null`], and an absent cell is a null cell. Keys are
    /// equal as the ordering verbs equate them: an absent key equals an
    /// absent key, every NaN is one value, and a nested key compares item by
    /// item. A period term such as `minutes(ts, 15)` keys the number of its
    /// period since the epoch, in UTC whatever zone the column states.
    ///
    /// The cost is one plan per call and one key per window: the key column
    /// computed once, one comparator over it and one bitmap of where the
    /// windows open, then each window costs the run of its key and nothing
    /// else. No key row is built where the keys order as their buffers -
    /// text, integers, temporals and records of them. Any other key - a
    /// registered code, a windows-1252 text, a version, a URL, a union -
    /// builds each of its rows once and holds them for the call, as
    /// [`Self::partition_by`] builds them: an allocation and a value per
    /// row. A period term such as `minutes(ts, 15)` is evaluated row by row
    /// through the expression's row tier, so it costs a constant count of
    /// allocations but time and a transient value per row.
    ///
    /// ```
    /// use yggdryl::{DataType, Field, Scalar, Serie, StructType};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let root = Field::new(
    ///     "quote",
    ///     DataType::from(StructType::from_fields([
    ///         Field::new("venue", DataType::utf8(), false),
    ///         Field::new("price", DataType::Int64, false),
    ///     ])?),
    ///     false,
    /// );
    /// let quotes = Serie::from_scalars(root, [
    ///     Scalar::from_sequence([Scalar::from("XNAS"), Scalar::from(1_i64)]),
    ///     Scalar::from_sequence([Scalar::from("XNAS"), Scalar::from(2_i64)]),
    ///     Scalar::from_sequence([Scalar::from("XNYS"), Scalar::from(3_i64)]),
    ///     Scalar::from_sequence([Scalar::from("XNAS"), Scalar::from(4_i64)]),
    /// ])?;
    /// let windows: Vec<_> = quotes.window_by("venue")?.collect();
    /// assert_eq!(windows.len(), 3);
    /// assert_eq!(windows[0].0, Scalar::from_sequence([Scalar::from("XNAS")]));
    /// assert_eq!((windows[0].1.offset(), windows[0].1.len()), (0, 2));
    /// // XNAS comes back after XNYS, so it opens a window of its own.
    /// assert_eq!(windows[2].0, Scalar::from_sequence([Scalar::from("XNAS")]));
    /// assert_eq!((windows[2].1.offset(), windows[2].1.len()), (3, 1));
    /// assert!(std::ptr::eq(windows[2].1.serie(), &quotes));
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error, before any row is read, for text that is not a
    /// selector; naming the serie, for a run, which windows by no term, for
    /// a key stating no projection - an empty list, or a `*` alone - and for
    /// an `unnest`; and the binder's own refusal for a column the key
    /// reaches none of, or reaches two of, and for a period step that is not
    /// a positive literal.
    pub fn window_by(&self, by: impl IntoSelector) -> Result<SerieWindows<'_>> {
        let key = self.window_key(&by.into_selector()?)?;
        Ok(SerieWindows::new(self, 0, key.apply_serie(self)?))
    }

    /// `by` bound as the key this serie's rows are windowed by: refused
    /// for a run, which no term reads, and under the key rule every keyed
    /// verb shares, naming this serie.
    pub(crate) fn window_key(&self, by: &Selector) -> Result<BoundSelector> {
        let Some(field) = self.field() else {
            return Err(self.not_a_record("windows by no term"));
        };
        by.bind_key(&SerieReader::root_of(field)?, self.name(), "window by")
    }

    /// Where the windows of equal adjacent rows open: one bit per row, set
    /// where the row opens a run - one comparator over the rows and one
    /// bitmap, whatever the run count.
    pub(crate) fn window_starts(&self) -> BooleanBuffer {
        let compare = Compare::new(self, SortOptions::default());
        BooleanBuffer::collect_bool(self.len(), |index| compare.opens(index))
    }

    /// Whether two keys are one key, as [`Self::window_starts`] equates
    /// adjacent rows: the values' own order, which the comparator over
    /// buffers is gated to agree with. A keyed walk across chunks asks it
    /// at each edge, of two keys it already built.
    pub(crate) fn same_key(left: &Scalar, right: &Scalar) -> bool {
        compare_values(left, right, SortOptions::default()) == Ordering::Equal
    }

    /// The record column whose children are `children`, which the caller
    /// took from one column so their rows align; the record is never
    /// absent.
    fn record_of(children: Vec<Self>) -> Result<Self> {
        let fields = children
            .iter()
            .map(|child| child.require_field().cloned())
            .collect::<Result<Vec<Field>>>()?;
        let arrays: Vec<ArrayRef> = children
            .iter()
            .map(|child| child.require_arrow_array())
            .collect::<Result<Vec<_>>>()?;
        let arrow_fields: arrow_schema::Fields = arrays
            .iter()
            .zip(&fields)
            .map(|(array, field)| {
                Arc::new(arrow_schema::Field::new(
                    field.name(),
                    array.data_type().clone(),
                    field.is_nullable(),
                ))
            })
            .collect::<Vec<_>>()
            .into();
        let record =
            arrow_array::StructArray::try_new(arrow_fields, arrays, None).map_err(Error::Arrow)?;
        let root = Field::new(
            "key",
            DataType::from(crate::StructType::from_fields(fields)?),
            false,
        );
        Ok(land(Arc::new(root), Arc::new(record), &Proof::Proven)?)
    }

    /// The bytes this serie's rows occupy: a column's buffers as its own
    /// slice counts them, a run's values as the row estimator charges them.
    ///
    /// ```
    /// use yggdryl::{Scalar, Serie};
    ///
    /// let prices = Serie::new(vec![Scalar::from(1_i64), Scalar::from(2_i64)]);
    /// assert!(prices.memory_size() > 0);
    /// ```
    pub fn memory_size(&self) -> usize {
        match self {
            Self::Run(run) => run.as_slice().iter().map(scalar_memory_size).sum(),
            column => array_memory_size(
                &column
                    .into_arrow_array()
                    .expect("a column lays out buffers"),
            ),
        }
    }

    // --------------------------------------------------------------------
    // Writes: this serie brought into the state, in place.
    // --------------------------------------------------------------------

    /// Sort the rows in place under `options`, answering this serie so
    /// calls chain.
    ///
    /// A primitive column holding its buffer alone sorts the native slice
    /// where it stands, gathers its absent rows to the end `options` names
    /// and rewrites the validity bits, and a boolean column holding its
    /// bitmaps alone counts its falses and trues and rewrites both bitmaps:
    /// what either costs is Arrow's builder handshake, never a row, and a
    /// shared buffer is copied once. Every other column replaces its
    /// buffers by the kernel's one copy, and a run sorts its values in place
    /// when it holds them alone, copied once when it does not.
    ///
    /// ```
    /// use yggdryl::{Scalar, Serie, SortOptions};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let mut prices = Serie::new(vec![Scalar::from(2_i64), Scalar::from(1_i64)]);
    /// prices.as_sorted(SortOptions::default())?.as_reversed()?;
    /// assert_eq!(prices.rows().to_vec(), vec![Scalar::from(2_i64), Scalar::from(1_i64)]);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// [`Self::sort_indices`]'s refusal, which leaves this serie as it was.
    pub fn as_sorted(&mut self, options: SortOptions) -> Result<&mut Self> {
        let len = self.len();
        if !self.sort_range_in_place(0..len, options) {
            *self = self.into_sorted(options)?;
        }
        Ok(self)
    }

    /// Sort rows `range` where they stand, answering whether the leaf could:
    /// a primitive column through its native slice, a boolean column through
    /// its two bitmaps, a run through its values, each copied once where it
    /// is shared; any other leaf answers `false` and is left as it was.
    pub(crate) fn sort_range_in_place(
        &mut self,
        range: Range<usize>,
        options: SortOptions,
    ) -> bool {
        match self {
            Self::Run(run) => {
                run.make_mut()[range].sort_by(|left, right| compare_values(left, right, options));
                true
            }
            Self::Boolean(held) => {
                Arc::make_mut(held).sort_in_place(range, options);
                true
            }
            _ => primitive_mut!(self, column => column.sort_in_place(range, options)).is_some(),
        }
    }

    /// Reverse rows `range` where they stand, answering whether the leaf
    /// could, exactly as [`Self::sort_range_in_place`] does.
    pub(crate) fn reverse_range_in_place(&mut self, range: Range<usize>) -> bool {
        match self {
            Self::Run(run) => {
                run.make_mut()[range].reverse();
                true
            }
            Self::Boolean(held) => {
                Arc::make_mut(held).reverse_in_place(range);
                true
            }
            _ => primitive_mut!(self, column => column.reverse_in_place(range)).is_some(),
        }
    }

    /// Keep the first occurrence of every value, in place, answering this
    /// serie: the kernel's one copy replaces the buffers of a column, and a
    /// run keeps its chosen values. The field is kept as it is.
    ///
    /// ```
    /// use yggdryl::{Scalar, Serie};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let mut venues = Serie::new(vec![Scalar::from("XNYS"), Scalar::from("XNAS"), Scalar::from("XNYS")]);
    /// venues.as_unique()?.as_reversed()?;
    /// assert_eq!(venues.rows().to_vec(), vec![Scalar::from("XNAS"), Scalar::from("XNYS")]);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// [`Self::into_unique`]'s refusal, which leaves this serie as it was.
    pub fn as_unique(&mut self) -> Result<&mut Self> {
        *self = self.into_unique()?;
        Ok(self)
    }

    /// Reverse the rows in place, answering this serie: a primitive column
    /// holding its buffer alone reverses the native slice and its validity
    /// bits where they stand, a boolean column its two bitmaps; every other
    /// column replaces its buffers by the kernel's one copy, and a run
    /// reverses its values in place when it holds them alone.
    ///
    /// ```
    /// use yggdryl::{Scalar, Serie};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let mut flags = Serie::new(vec![Scalar::from(true), Scalar::Null, Scalar::from(false)]);
    /// flags.as_reversed()?;
    /// assert_eq!(flags.rows().to_vec(), vec![Scalar::from(false), Scalar::Null, Scalar::from(true)]);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Never, in practice: the signature matches the other `as_*` writes so
    /// calls chain.
    pub fn as_reversed(&mut self) -> Result<&mut Self> {
        let len = self.len();
        if !self.reverse_range_in_place(0..len) {
            *self = self.into_reversed();
        }
        Ok(self)
    }

    /// Keep the rows `indices` names, in that order, in place, answering
    /// this serie: the kernel's one copy replaces the buffers of a column.
    ///
    /// ```
    /// use yggdryl::{Scalar, Serie};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let mut prices = Serie::new(vec![Scalar::from(1_i64), Scalar::from(2_i64), Scalar::from(3_i64)]);
    /// prices.as_taken(&Serie::new(vec![Scalar::from(2_u32), Scalar::from(0_u32)]))?;
    /// assert_eq!(prices.rows().to_vec(), vec![Scalar::from(3_i64), Scalar::from(1_i64)]);
    /// assert!(prices.as_taken(&Serie::new(vec![Scalar::from(9_u32)])).is_err());
    /// assert_eq!(prices.len(), 2);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// [`Self::into_taken`]'s refusal, which leaves this serie as it was.
    pub fn as_taken(&mut self, indices: &Self) -> Result<&mut Self> {
        *self = self.into_taken(indices)?;
        Ok(self)
    }

    /// Keep the rows `mask` keeps, in place, answering this serie: the
    /// kernel's one copy replaces the buffers of a column.
    ///
    /// ```
    /// use yggdryl::{Scalar, Serie};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let mut prices = Serie::new(vec![Scalar::from(1_i64), Scalar::from(2_i64), Scalar::from(3_i64)]);
    /// let mask = Serie::new(vec![Scalar::from(true), Scalar::Null, Scalar::from(true)]);
    /// prices.as_filtered(&mask)?;
    /// assert_eq!(prices.rows().to_vec(), vec![Scalar::from(1_i64), Scalar::from(3_i64)]);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// [`Self::into_filtered`]'s refusal, which leaves this serie as it was.
    pub fn as_filtered(&mut self, mask: &Self) -> Result<&mut Self> {
        *self = self.into_filtered(mask)?;
        Ok(self)
    }
}
