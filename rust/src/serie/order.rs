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
//! go through the values' own order, each row built once. The verbs that
//! order or group by one comparator - `sort_indices` and the sorts built on
//! it, `partition_by`, `window_by` - take one rung more: a record whose
//! stored bytes do not order as its values goes child by child, each child
//! on its own rung, so only a child whose stored order is not its value
//! order builds its values - once, never one run per row. `is_sorted`, the
//! uniqueness verbs and a comparison across two series take the values' own
//! order for such a record. Uniqueness is one hash set over the row format's
//! bytes on the same rung, or over the values.
//! So every leaf answers every verb, and the cost is the ladder's rung,
//! stated on each. The `*_by` sorts order by a list of `order by` keys,
//! each a term computed once over the whole serie: Arrow's row format over
//! every key cell at once where each cell's stored bytes order as its
//! values, and otherwise the first key whose cells differ, each cell on its
//! own rung under its own key's options.
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
use arrow_buffer::{BooleanBuffer, NullBuffer};
use arrow_data::ArrayData;
use arrow_ord::ord::{DynComparator, make_comparator};
use arrow_row::{RowConverter, Rows, SortField};
use arrow_schema::DataType as ArrowDataType;

use super::{Proof, Rows as _, Serie, land, proven_row};
use crate::arrow::{array_memory_size, scalar_memory_size};
use crate::expression::{self, IntoOrderings, Projection};
use crate::{DataType, Error, Field, Result, Scalar, Selector, SortOptions, StreamChunkedSerie};

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
pub(crate) fn require_indexable(serie: &Serie) -> Result<u32> {
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
pub(crate) fn stored_order_is_value_order(dtype: &DataType) -> bool {
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
/// over the buffers where they order as the values, a record's children
/// each on its own rung where only some of them do, and the values' own
/// order over rows built once otherwise and for a run.
enum Compare<'a> {
    /// Arrow's comparator, which reads the buffers and builds no value.
    Buffers(DynComparator),
    /// A record column whose buffers do not order as its values: its own
    /// absent rows placed where `options` puts an absence, then each child
    /// on its own rung, so only a child whose stored order is not its value
    /// order builds its rows - one leaf's values, never one run per row.
    Record {
        nulls: Option<NullBuffer>,
        children: Box<[Self]>,
        options: SortOptions,
    },
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
        if let Some(record) = serie.as_struct()
            && !record.children().is_empty()
        {
            return Self::Record {
                nulls: record.nulls().cloned(),
                children: record
                    .children()
                    .iter()
                    .map(|child| Self::new(child, options))
                    .collect(),
                options,
            };
        }
        Self::Values {
            rows: serie.rows(),
            options,
        }
    }

    fn cmp(&self, left: usize, right: usize) -> Ordering {
        match self {
            Self::Buffers(compare) => compare(left, right),
            // `compare_values`' sequence arm over two runs of one arity: an
            // absent row where `options` puts an absence, then the first
            // child step that is not equal - each child already directed by
            // `options`, so the answer is not reversed again.
            Self::Record {
                nulls,
                children,
                options,
            } => {
                let absent = |row: usize| nulls.as_ref().is_some_and(|nulls| nulls.is_null(row));
                match (absent(left), absent(right)) {
                    (true, true) => Ordering::Equal,
                    (true, false) => absent_against_present(*options),
                    (false, true) => absent_against_present(*options).reverse(),
                    (false, false) => children
                        .iter()
                        .map(|child| child.cmp(left, right))
                        .find(|step| *step != Ordering::Equal)
                        .unwrap_or(Ordering::Equal),
                }
            }
            Self::Values { rows, options } => compare_values(&rows[left], &rows[right], *options),
        }
    }

    /// Row `index` against the row before it: `Less` for the first row,
    /// which opens whatever comes, `Equal` where it continues its run and
    /// `Greater` where it orders before the row it follows - a descent.
    fn step(&self, index: usize) -> Ordering {
        if index == 0 {
            Ordering::Less
        } else {
            self.cmp(index - 1, index)
        }
    }

    /// Whether row `index` opens a run of equal rows: the first row, or one
    /// the row before it does not equal. The one boundary a sorted group
    /// and a window are both cut at.
    fn opens(&self, index: usize) -> bool {
        self.step(index) != Ordering::Equal
    }
}

/// Every key cell's rows in Arrow's row format, one converter over all of
/// them, each under its own key's options: bytes whose order is the keys'
/// order. `None` unless every cell's buffers order as its values
/// ([`Serie::ordered_buffers`]) and the format has a layout for each. A row
/// the key record leaves absent is masked absent in every cell.
fn key_rows(
    cells: &[Serie],
    keys: &[expression::Ordering],
    absent: Option<&NullBuffer>,
) -> Option<Rows> {
    let mut fields = Vec::with_capacity(cells.len());
    let mut arrays = Vec::with_capacity(cells.len());
    for (cell, key) in cells.iter().zip(keys) {
        let mut array = cell.ordered_buffers()?;
        if let Some(absent) = absent {
            array = masked(&array, absent)?;
        }
        fields.push(SortField::new_with_options(
            array.data_type().clone(),
            key.options().into_arrow(),
        ));
        arrays.push(array);
    }
    if !RowConverter::supports_fields(&fields) {
        return None;
    }
    RowConverter::new(fields)
        .ok()?
        .convert_columns(&arrays)
        .ok()
}

/// `array` absent wherever `absent` is, its buffers shared: a record's
/// absent row says nothing of the slots beneath it. `None` for a layout
/// that holds no validity of its own - a run-end encoding, a union, a null
/// column - which the values' own order reads instead.
fn masked(array: &ArrayRef, absent: &NullBuffer) -> Option<ArrayRef> {
    let nulls = NullBuffer::union(array.nulls(), Some(absent));
    let data = array.to_data().into_builder().nulls(nulls).build().ok()?;
    Some(arrow_array::make_array(data))
}

/// How two rows compare under a list of `order by` keys, off the row
/// format: the first key whose cells differ decides, each cell on its own
/// rung of [`Compare`] under its own key's options, and a row the key
/// record leaves absent is absent in every cell.
struct KeysCompare<'a> {
    /// The key record's absent rows, where it has any.
    absent: Option<&'a NullBuffer>,
    /// Per key: its cell column, how two of its rows compare, its options.
    keys: Box<[(&'a Serie, Compare<'a>, SortOptions)]>,
}

impl<'a> KeysCompare<'a> {
    fn new(
        cells: &'a [Serie],
        keys: &[expression::Ordering],
        absent: Option<&'a NullBuffer>,
    ) -> Self {
        Self {
            absent,
            keys: cells
                .iter()
                .zip(keys)
                .map(|(cell, key)| (cell, Compare::new(cell, key.options()), key.options()))
                .collect(),
        }
    }

    fn cmp(&self, left: usize, right: usize) -> Ordering {
        let absent = |row: usize| self.absent.is_some_and(|nulls| nulls.is_null(row));
        let (left_absent, right_absent) = (absent(left), absent(right));
        for (cell, compare, options) in &self.keys {
            let null = |row: usize| matches!(cell.is_null(row), Ok(true));
            let step = match (left_absent, right_absent) {
                (false, false) => compare.cmp(left, right),
                (true, true) => Ordering::Equal,
                (true, false) if null(right) => Ordering::Equal,
                (true, false) => absent_against_present(*options),
                (false, true) if null(left) => Ordering::Equal,
                (false, true) => absent_against_present(*options).reverse(),
            };
            if step != Ordering::Equal {
                return step;
            }
        }
        Ordering::Equal
    }
}

/// Where the windows of equal adjacent rows open, and whether the rows are
/// in key order: what [`Serie::window_starts`] answers.
pub(crate) struct WindowCut {
    /// One bit per row, set where a window opens.
    pub(crate) starts: BooleanBuffer,
    /// The first row whose key orders before its predecessor's under
    /// `SortOptions::default()`: ascending, absent keys last.
    pub(crate) descent: Option<usize>,
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
pub(crate) fn compare_values(left: &Scalar, right: &Scalar, options: SortOptions) -> Ordering {
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
    /// any other record compares child by child, each on its own rung, so
    /// only a child whose stored order is not its value order builds its
    /// values, once, and no run is built per row; a run and any other
    /// column (a version, windows-1252 text, a code, a URL, a union, a float
    /// holding a foreign NaN payload) sort through the values' own order,
    /// each row built once.
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
        if matches!(self, Self::Lit(_)) {
            // Every row is the one value: the order is the one they stand in.
            return Ok((0..len as u32).collect());
        }
        if let Some(whole) = self.whole_row_order(options)
            && self.declares_at_least(&whole)
        {
            return Ok((0..len as u32).collect());
        }
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

    /// The row positions in the order the `order by` keys of `by` state, as
    /// the `uint32` column named `index` [`Self::sort_indices`] answers:
    /// stable, so rows whose every key is equal keep their order.
    ///
    /// `by` is the keys most significant first - the clause's text
    /// (`"venue, price desc nulls first"`), an
    /// [`Ordering`](crate::expression::Ordering), a list of either, a
    /// [`Selector`] (every projection ascending) or a [`Scalar`] - each a
    /// term with its direction and its nulls placement
    /// ([`IntoOrderings`]). The terms are bound once against
    /// [`StreamChunkedSerie::root_of`]: a record column against its own field, any
    /// other column as the one child of its record under its own name, so
    /// a column named `price` sorts by `"price desc"`. A key that is a
    /// column is the landed column, zero copy; a computed one - `price *
    /// 2`, `lower(venue)` - is evaluated once over the whole serie. A row
    /// the record leaves absent is absent in every key.
    ///
    /// The rung is the keys': where every key cell's stored bytes order as
    /// its values - text, integers, temporals, records of them - one Arrow
    /// row converter encodes every cell at once, each under its own
    /// options, and the order is one stable sort over those rows: the rows'
    /// bytes and the order, one allocation each. Otherwise - a version, a
    /// windows-1252 text, a registered code, a URL, a float holding a
    /// foreign NaN among the keys - the first key whose cells differ
    /// decides, each cell on its own rung of [`Self::sort_indices`]'s
    /// ladder under its own key's options, so only such a cell builds its
    /// values, once. One key alone sorts as its cell's own
    /// [`Self::sort_indices`] does, a primitive cell over its native slice.
    ///
    /// ```
    /// use yggdryl::{DataType, Scalar, Serie, StructType};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let root = StructType::from_fields([
    ///     DataType::utf8().required_field("venue"),
    ///     DataType::Int64.nullable_field("price"),
    /// ])
    /// .map(DataType::from)?
    /// .required_field("quote");
    /// let quote = |venue: &str, price: Option<i64>| {
    ///     Scalar::from_sequence([Scalar::from(venue), price.map_or(Scalar::Null, Scalar::from)])
    /// };
    /// let quotes = Serie::from_scalars(root, [
    ///     quote("XNYS", Some(1)),
    ///     quote("XNAS", Some(2)),
    ///     quote("XNAS", None),
    ///     quote("XNYS", Some(3)),
    /// ])?;
    /// let order = quotes.sort_indices_by("venue, price desc nulls first")?;
    /// assert_eq!(order.rows().to_vec(), [2_u32, 1, 3, 0].map(Scalar::from));
    ///
    /// // A plain column keys as itself, under its own name.
    /// let prices = Serie::from_scalars(DataType::Int64.required_field("price"), [3_i64, 1, 2].map(Scalar::from))?;
    /// assert_eq!(prices.sort_indices_by("price desc")?.rows().to_vec(), [0_u32, 2, 1].map(Scalar::from));
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error, before any row is read: the parse error for text
    /// that is not a list of keys; naming the serie, for no key at all, for
    /// a run, which sorts by no term, and for an `unnest`, which is one row
    /// per element where a key is one value per row; the binder's own
    /// refusal for a term reaching no column, or two, and for two keys
    /// publishing one name - alias one; and, naming the serie, for more
    /// rows than one `uint32` index column addresses.
    pub fn sort_indices_by(&self, by: impl IntoOrderings) -> Result<Self> {
        index_column(self.sorted_order_by(&by.into_orderings()?)?)
    }

    /// The positions [`Self::sort_indices_by`] answers, as the vector it
    /// builds.
    pub(crate) fn sorted_order_by(&self, by: &[expression::Ordering]) -> Result<Vec<u32>> {
        let Some(field) = self.field() else {
            return Err(self.not_a_record("sorts by no term"));
        };
        if by.is_empty() {
            return Err(refuse(
                self,
                smol_str::format_smolstr!(
                    "expected at least one `order by` key to sort {} by, got none",
                    self.name()
                ),
            ));
        }
        let len = require_indexable(self)? as usize;
        if self.declares_at_least(by) {
            return Ok((0..len as u32).collect());
        }
        let key = Selector::new(by.iter().map(|key| Projection::new(key.term().clone())))
            .bind_key(&StreamChunkedSerie::root_of(field)?, self.name(), "sort by")?;
        let keys = key.apply_serie(self)?;
        let record = keys.as_struct().expect("a key is a record column");
        let absent = record.nulls().filter(|nulls| nulls.null_count() > 0);
        let cells = record.children();
        if let ([cell], [key], None) = (cells, by, absent) {
            return cell.sorted_order(key.options());
        }
        let mut order: Vec<u32> = (0..len as u32).collect();
        if let Some(rows) = key_rows(cells, by, absent) {
            order.sort_by(|left, right| rows.row(*left as usize).cmp(&rows.row(*right as usize)));
            return Ok(order);
        }
        let compare = KeysCompare::new(cells, by, absent);
        order.sort_by(|left, right| compare.cmp(*left as usize, *right as usize));
        Ok(order)
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
        if matches!(self, Self::Lit(_)) {
            return true;
        }
        if let Some(whole) = self.whole_row_order(options)
            && self.declares_at_least(&whole)
        {
            return true;
        }
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
        if let Self::Lit(lit) = self {
            return crate::value::SerieValue::len(lit.as_ref()) <= 1;
        }
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
        if let Self::Lit(lit) = self {
            return crate::value::SerieValue::len(lit.as_ref()).min(1);
        }
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
        if let Some(whole) = self.whole_row_order(options)
            && self.declares_at_least(&whole)
        {
            // Already in this order, and the root says so: the same buffers.
            return Ok(self.clone());
        }
        let order = self.sorted_order(options)?;
        let sorted = self.taken(&order)?;
        match self.whole_row_order(options) {
            Some(by) => sorted.declaring_order(&by),
            None => Ok(sorted),
        }
    }

    /// The rows in the order the `order by` keys of `by` state, under the
    /// same field, this serie untouched: [`Self::sort_indices_by`] - its
    /// rung, its stability - then one take of every column.
    ///
    /// ```
    /// use yggdryl::{DataType, Scalar, Serie, StructType};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let root = StructType::from_fields([
    ///     DataType::utf8().required_field("venue"),
    ///     DataType::Int64.required_field("price"),
    /// ])
    /// .map(DataType::from)?
    /// .required_field("quote");
    /// let quote = |venue: &str, price: i64| Scalar::from_sequence([Scalar::from(venue), Scalar::from(price)]);
    /// let quotes = Serie::from_scalars(root, [quote("XNYS", 1), quote("XNAS", 2), quote("XNYS", 3)])?;
    /// let sorted = quotes.into_sort_by("venue desc, price desc")?;
    /// assert_eq!(sorted.rows().to_vec(), vec![quote("XNYS", 3), quote("XNYS", 1), quote("XNAS", 2)]);
    /// // The result declares the order it keeps, so a sort by the same keys is a clone.
    /// assert_eq!(sorted.declared_order()?.map(|by| by.len()), Some(2));
    /// assert_eq!(quotes.scalar(0)?, quote("XNYS", 1));
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// [`Self::sort_indices_by`]'s refusals.
    pub fn into_sort_by(&self, by: impl IntoOrderings) -> Result<Self> {
        let by = by.into_orderings()?;
        if !by.is_empty() && self.declares_at_least(&by) {
            // Already in this order, and the root says so: the same buffers.
            return Ok(self.clone());
        }
        let order = self.sorted_order_by(&by)?;
        self.taken(&order)?.declaring_order(&by)
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
        if matches!(self, Self::Lit(_)) {
            // Every row is the one value, so the column reversed is itself.
            return self.clone();
        }
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
        let reversed = self
            .taken(&order)
            .expect("a reversal names every row once, which the take accepts");
        match self.declared_order() {
            Ok(Some(by)) => reversed
                .declaring_order(&reversed_order(&by))
                .expect("a key read off a declaration is one the declaration takes back"),
            _ => reversed,
        }
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
        let taken = self.taken(&order)?;
        // Rows picked in increasing position keep the order the root
        // declares; any other pick could break it, so the declaration goes.
        if order.windows(2).all(|pair| pair[0] < pair[1]) {
            Ok(taken)
        } else {
            taken.clearing_order()
        }
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
        self.raise_held()?;
        match self.held_leaf() {
            // Every row is the one value: the pick is the count.
            Self::Lit(lit) => Ok(Self::lit(
                Arc::clone(crate::value::SerieValue::field_ref(lit.as_ref())),
                lit.value().clone(),
                order.len(),
            )?),
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
                land(field, taken, &Proof::Proven)?.settled()
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
        self.raise_held()?;
        match self.held_leaf() {
            // Every row is the one value: the kept rows are a count.
            Self::Lit(lit) => Ok(Self::lit(
                Arc::clone(crate::value::SerieValue::field_ref(lit.as_ref())),
                lit.value().clone(),
                keep.iter().filter(|kept| **kept).count(),
            )?),
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
                land(field, kept, &Proof::Proven)?.settled()
            }
        }
    }

    pub(crate) fn cut_partitions(&self, keys: &Self) -> Result<Vec<(Scalar, Self)>> {
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
        let mut direction = None;
        let clustered = (1..len).all(|index| {
            let step = compare.cmp(index - 1, index);
            if step == Ordering::Equal {
                return true;
            }
            *direction.get_or_insert(step) == step
        });
        if clustered {
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

    /// Equal-key run starts and the first descent, using one comparator.
    pub(crate) fn window_starts(&self) -> WindowCut {
        let compare = Compare::new(self, SortOptions::default());
        let mut descent = None;
        let starts = BooleanBuffer::collect_bool(self.len(), |index| {
            let step = compare.step(index);
            if step == Ordering::Greater && descent.is_none() {
                descent = Some(index);
            }
            step != Ordering::Equal
        });
        WindowCut { starts, descent }
    }

    /// `value`, a key already built, against row `row` of this key column
    /// under `options`: what [`compare_values`] answers against that row,
    /// read in place. A record column compares cell by cell - an absent row
    /// reads as [`Scalar::Null`] - building each cell and never the row's
    /// run, so inline cells cost nothing; any other column builds its row.
    /// `row` is below the length.
    pub(crate) fn compare_to_row(
        &self,
        value: &Scalar,
        row: usize,
        options: SortOptions,
    ) -> Ordering {
        let Some(record) = self.as_struct() else {
            return compare_values(value, &proven_row(self, row), options);
        };
        let absent = record.nulls().is_some_and(|nulls| nulls.is_null(row));
        match (value.is_null(), absent) {
            (true, true) => return Ordering::Equal,
            (true, false) => return absent_against_present(options),
            (false, true) => return absent_against_present(options).reverse(),
            (false, false) => {}
        }
        let Some(cells) = value.as_serie() else {
            return compare_values(value, &proven_row(self, row), options);
        };
        let children = record.children();
        for (index, child) in children.iter().enumerate().take(cells.len()) {
            let step = compare_values(&cells.row_at(index), &proven_row(child, row), options);
            if step != Ordering::Equal {
                return step;
            }
        }
        directed(cells.len().cmp(&children.len()), options)
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
        if let Some(media) = self.media_state() {
            return media.held_memory_size();
        }
        match self {
            // Chunks count their own; a stream what it holds so far.
            Self::Chunked(chunked) => chunked.memory_size(),
            Self::Stream(stream) => stream.memory_size(),
            Self::StreamChunked(stream) => stream.memory_size(),
            Self::Key(key) => key.memory_size(),
            Self::Keys(keys) => keys.memory_size(),
            Self::StreamKey(stream) => stream.memory_size(),
            Self::Run(run) => run.as_slice().iter().map(scalar_memory_size).sum(),
            // A constant states its estimate from the one row it holds and
            // builds nothing, so sizing a column never lays it out.
            Self::Lit(lit) => crate::value::SerieValue::memory_size(lit.as_ref()),
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
        if self.sort_range_in_place(0..len, options) {
            self.settle()?;
        } else {
            *self = self.into_sorted(options)?;
        }
        Ok(self)
    }

    /// Sort the rows in place in the order the `order by` keys of `by`
    /// state, answering this serie so calls chain: [`Self::into_sort_by`]'s
    /// one take replaces the buffers.
    ///
    /// ```
    /// use yggdryl::{DataType, Scalar, Serie};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let mut prices = Serie::from_scalars(DataType::Int64.required_field("price"), [2_i64, 3, 1].map(Scalar::from))?;
    /// prices.as_sort_by("price desc")?.as_reversed()?;
    /// assert_eq!(prices.rows().to_vec(), [1_i64, 2, 3].map(Scalar::from));
    /// assert!(prices.as_sort_by("tier").is_err());
    /// assert_eq!(prices.rows().to_vec(), [1_i64, 2, 3].map(Scalar::from));
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// [`Self::sort_indices_by`]'s refusals, which leave this serie as it
    /// was.
    pub fn as_sort_by(&mut self, by: impl IntoOrderings) -> Result<&mut Self> {
        *self = self.into_sort_by(by)?;
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
        match self.held_leaf_mut() {
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
        match self.held_leaf_mut() {
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
        if self.reverse_range_in_place(0..len) {
            self.settle()?;
        } else {
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

// ---------------------------------------------------------------------------
// The order a record declares: `SORT:by` on its root, a fact about its rows
// that every verb keeps true - written by the sorts, kept by what keeps the
// order, cleared by a write that breaks it, read before any row is compared.
// ---------------------------------------------------------------------------

/// The keys as a declaration spells them, `venue, price desc`.
pub(crate) fn spelled(by: &[expression::Ordering]) -> String {
    by.iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

/// Every key of `by` turned around: the direction and where the absent
/// rows go both flipped, which is what reversing sorted rows leaves true.
fn reversed_order(by: &[expression::Ordering]) -> Vec<expression::Ordering> {
    by.iter()
        .map(|key| {
            let options = key.options();
            let flipped = if options.is_descending() {
                SortOptions::ascending()
            } else {
                SortOptions::descending()
            }
            .with_nulls_first(!options.is_nulls_first());
            expression::Ordering::new(key.term().clone(), flipped)
        })
        .collect()
}

impl Serie {
    /// The `order by` keys this record's root declares its rows keep, most
    /// significant first: `SORT:by` on the root, read through
    /// [`Field::as_sort`]. `None` for a run, a column that is not a record,
    /// and a root declaring none.
    ///
    /// A declaration is a proven fact about the rows, never a hint:
    /// [`Self::into_sorted`] and [`Self::into_sort_by`] write it, a slice, a
    /// filter, `into_unique` and a take in increasing position keep it, a
    /// reversal flips it, a write compares the rows it touched against their
    /// neighbours and clears it where the order no longer holds, and
    /// [`Self::is_sorted`], `sort_indices`, `into_sorted` and their `_by`
    /// forms answer without a pass where it states what they ask.
    ///
    /// ```
    /// use yggdryl::{DataType, Scalar, Serie, SortOptions, StructType};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let root = StructType::from_fields([
    ///     DataType::utf8().required_field("venue"),
    ///     DataType::Int64.required_field("price"),
    /// ])
    /// .map(DataType::from)?
    /// .required_field("quote");
    /// let quote = |venue: &str, price: i64| Scalar::from_sequence([Scalar::from(venue), Scalar::from(price)]);
    /// let quotes = Serie::from_scalars(root, [quote("XNYS", 2), quote("XNAS", 1)])?;
    /// assert!(quotes.declared_order()?.is_none());
    ///
    /// let sorted = quotes.into_sort_by("venue, price desc")?;
    /// let keys = sorted.declared_order()?.expect("the sort declared its keys");
    /// assert_eq!(keys.len(), 2);
    /// assert_eq!(keys[0].term().to_string(), "venue");
    /// assert!(keys[1].is_descending());
    /// assert_eq!(sorted.field().expect("a record").get_metadata("SORT:by"), Some(r#"["venue","price desc"]"#));
    ///
    /// // What the declaration states is answered without a pass.
    /// assert_eq!(sorted.sort_indices_by("venue")?.rows().to_vec(), [0_u32, 1].map(Scalar::from));
    /// // A write that breaks the order clears it; one that keeps it does not.
    /// let mut held = sorted.clone();
    /// held.push(quote("XNYS", 1))?;
    /// assert!(held.declared_order()?.is_some());
    /// held.push(quote("AAAA", 0))?;
    /// assert!(held.declared_order()?.is_none());
    /// // A whole-row sort declares every column.
    /// let whole = quotes.into_sorted(SortOptions::default())?;
    /// assert_eq!(whole.field().expect("a record").get_metadata("SORT:by"), Some(r#"["venue","price"]"#));
    /// assert!(whole.is_sorted(SortOptions::default()));
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error naming `SORT:by` when the root's text is not a list
    /// of `order by` keys, which no door of this crate writes.
    pub fn declared_order(&self) -> Result<Option<Vec<expression::Ordering>>> {
        let Some(field) = self.field() else {
            return Ok(None);
        };
        if field.dtype().as_fields().is_none() || !field.as_sort().declares_order() {
            return Ok(None);
        }
        field.as_sort().by()
    }

    /// Whether the root declares an order beginning with `asked`: what the
    /// declaration already proves, a prefix of it included.
    pub(crate) fn declares_at_least(&self, asked: &[expression::Ordering]) -> bool {
        matches!(self.declared_order(), Ok(Some(declared)) if declared.starts_with(asked))
    }

    /// The keys a whole-row sort of this record under `options` is: every
    /// child in declaration order, each under `options`; `None` for anything
    /// but a record.
    fn whole_row_order(&self, options: SortOptions) -> Option<Vec<expression::Ordering>> {
        let record = self.as_struct()?;
        Some(
            record
                .children()
                .iter()
                .map(|child| {
                    expression::Ordering::new(expression::Term::column(child.name()), options)
                })
                .collect(),
        )
    }

    /// This record under its root declaring `by` as the order its rows
    /// keep, the same buffers; a run, a column that is not a record and an
    /// empty `by` as they are, as is a root already stating it. A `by` the
    /// declaration cannot hold - a key repeated - declares nothing, which is
    /// always true.
    pub(crate) fn declaring_order(self, by: &[expression::Ordering]) -> Result<Self> {
        if by.is_empty() || self.as_struct().is_none() {
            return Ok(self);
        }
        let Some(field) = self.field() else {
            return Ok(self);
        };
        if field.as_sort().by()?.as_deref() == Some(by) {
            return Ok(self);
        }
        let mut root = field.clone();
        if root.as_sort_mut().set_by(by.iter().cloned()).is_err() {
            return Ok(self);
        }
        self.relabeled(Arc::new(root))
    }

    /// This record under its root declaring no order, the same buffers; as
    /// it is where it declares none.
    pub(crate) fn clearing_order(self) -> Result<Self> {
        let Some(field) = self.field() else {
            return Ok(self);
        };
        if !field.as_sort().declares_order() {
            return Ok(self);
        }
        let mut root = field.clone();
        root.as_sort_mut().remove_by();
        self.relabeled(Arc::new(root))
    }

    /// This record under `field`, the same children and buffers, stating
    /// the same backing: what a verb changing only what the root declares
    /// answers through - the record's leaf copied once, its field swapped.
    /// Anything but a record is itself.
    pub(crate) fn relabeled(&self, field: Arc<Field>) -> Result<Self> {
        Ok(self.clone().into_relabeled(field))
    }

    /// [`Self::relabeled`], consuming this record: a leaf nothing else
    /// holds has its field swapped where it stands, allocating nothing.
    pub(crate) fn into_relabeled(mut self, field: Arc<Field>) -> Self {
        if let Self::Struct(held) = &mut self {
            Arc::make_mut(held).set_field(field);
        }
        self
    }

    /// Whether the declared order `by` holds across the rows `written` and
    /// the row on either side of them: `Some(false)` where two adjacent rows
    /// there compare out of order, `None` where a key is not a bare column,
    /// which a write does not evaluate.
    fn order_holds(&self, by: &[expression::Ordering], written: Range<usize>) -> Option<bool> {
        let mut keys = Vec::with_capacity(by.len());
        for key in by {
            keys.push((self.child(key.term().as_column()?)?, key.options()));
        }
        let len = self.len();
        if len < 2 {
            return Some(true);
        }
        let first = written.start.saturating_sub(1);
        let last = written.end.min(len - 1);
        for row in first..last {
            for (child, options) in &keys {
                match child.compare_across(row, child, row + 1, *options) {
                    Ordering::Less => break,
                    Ordering::Equal => {}
                    Ordering::Greater => return Some(false),
                }
            }
        }
        Some(true)
    }

    /// The first row out of the order `by` states, `None` where every
    /// adjacent pair is in order: one pass over the key record `by`
    /// computes, as a sort computes it, for rows no verb of the crate laid
    /// out in that order.
    ///
    /// # Errors
    ///
    /// The binder's refusal of a key.
    pub(crate) fn first_disorder(&self, by: &[expression::Ordering]) -> Result<Option<usize>> {
        let Some(field) = self.field() else {
            return Ok(None);
        };
        if by.is_empty() || self.len() < 2 {
            return Ok(None);
        }
        let key = Selector::new(by.iter().map(|key| Projection::new(key.term().clone())))
            .bind_key(&StreamChunkedSerie::root_of(field)?, self.name(), "sort by")?;
        let keys = key.apply_serie(self)?;
        let record = keys.as_struct().expect("a key is a record column");
        let absent = record.nulls().filter(|nulls| nulls.null_count() > 0);
        let compare = KeysCompare::new(record.children(), by, absent);
        Ok((1..self.len()).find(|row| compare.cmp(row - 1, *row) == Ordering::Greater))
    }

    /// This record checked against the order its root declares, refusing
    /// the first row out of it by name; a record declaring none, and a run,
    /// as they are. What every door landing rows no verb of the crate laid
    /// out answers through, so a declaration a `Serie` carries is a proven
    /// one.
    ///
    /// # Errors
    ///
    /// Returns an error naming the serie, the row and the declared keys
    /// when a row is out of order.
    pub(crate) fn verified_order(self) -> Result<Self> {
        let Some(by) = self.declared_order()? else {
            return Ok(self);
        };
        match self.first_disorder(&by)? {
            None => Ok(self),
            Some(row) => Err(self.out_of_order(row, &by)),
        }
    }

    /// The refusal of row `row` lying out of the declared order `by`.
    pub(crate) fn out_of_order(&self, row: usize, by: &[expression::Ordering]) -> Error {
        refuse(
            self,
            smol_str::format_smolstr!(
                "row {row} of {} is out of the order its root declares, `{}`",
                self.name(),
                spelled(by)
            ),
        )
    }

    /// Whether this record's last row and `next`'s first are in the order
    /// `by` states: the edge two chunks or two batches of one column meet
    /// at. Either side empty, or no key, is in order.
    ///
    /// # Errors
    ///
    /// The binder's refusal of a key.
    pub(crate) fn edge_in_order(&self, next: &Self, by: &[expression::Ordering]) -> Result<bool> {
        if self.is_empty() || next.is_empty() || by.is_empty() {
            return Ok(true);
        }
        let Some(field) = self.field() else {
            return Ok(true);
        };
        // Keys that are bare columns compare where they lie, no key bound.
        if let Some(answer) = self.edge_by_columns(next, by) {
            return Ok(answer);
        }
        let key = Selector::new(by.iter().map(|key| Projection::new(key.term().clone())))
            .bind_key(&StreamChunkedSerie::root_of(field)?, self.name(), "sort by")?;
        let mut pair = key.apply_serie(&self.slice(self.len() - 1, 1)?)?;
        pair.extend_from_serie(&key.apply_serie(&next.slice(0, 1)?)?)?;
        let record = pair.as_struct().expect("a key is a record column");
        let absent = record.nulls().filter(|nulls| nulls.null_count() > 0);
        let compare = KeysCompare::new(record.children(), by, absent);
        Ok(compare.cmp(0, 1) != Ordering::Greater)
    }

    /// [`Self::edge_in_order`] over keys that are every one a bare column:
    /// this record's last row against `next`'s first, child by child, with
    /// the comparator a sort uses; `None` where a key is computed or names
    /// no child.
    fn edge_by_columns(&self, next: &Self, by: &[expression::Ordering]) -> Option<bool> {
        let last = self.len().checked_sub(1)?;
        for key in by {
            let name = key.term().as_column()?;
            let (left, right) = (self.child(name)?, next.child(name)?);
            match left.compare_across(last, right, 0, key.options()) {
                Ordering::Less => return Some(true),
                Ordering::Equal => {}
                Ordering::Greater => return Some(false),
            }
        }
        Some(true)
    }

    /// Keep this record's declared order where the rows `written` left it
    /// true, and clear it otherwise - or where a key is computed, which a
    /// write does not evaluate. A record declaring none is untouched.
    pub(crate) fn keep_or_clear_order(&mut self, written: Range<usize>) -> Result<()> {
        let Some(by) = self.declared_order()? else {
            return Ok(());
        };
        if self.order_holds(&by, written) != Some(true) {
            *self = self.clone().clearing_order()?;
        }
        Ok(())
    }
}
