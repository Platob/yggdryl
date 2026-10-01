//! The ordering, uniqueness and grouping verbs every leaf answers, through
//! one ladder.
//!
//! A column sorts over its buffers where its layout has a typed sort - a
//! primitive leaf sorts the native slice, stable, nulls gathered to the end
//! the options name - and through Arrow's row format where the type has one,
//! which is every layout but what the row format refuses; two rows compare
//! through Arrow's comparator over the buffers wherever it has one, and the
//! values' own total order everywhere else and for a run. Uniqueness is one
//! hash set over the row format's bytes, or over the values. So every leaf
//! answers every verb, and the cost is the ladder's rung, stated on each.
//!
//! The reads answer a new serie and leave this one as it is; the `as_*`
//! writes bring this serie into the state in place, rewriting a uniquely
//! held primitive buffer where it stands and replacing any other leaf's
//! buffers by the kernel's one copy.

use std::cmp::Ordering;
use std::collections::HashSet;
use std::ops::Range;
use std::sync::Arc;

use arrow_array::{Array, ArrayRef, BooleanArray, UInt32Array};
use arrow_ord::ord::{DynComparator, make_comparator};
use arrow_row::{RowConverter, Rows, SortField};

use super::{Proof, Rows as _, Serie, land};
use crate::arrow::{array_memory_size, scalar_memory_size};
use crate::{DataType, Error, Field, FieldPath, Result, Scalar, SortOptions};

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

/// How two rows of one serie compare under `options`: Arrow's comparator
/// over the buffers where it has one, the values' own order elsewhere and
/// for a run.
enum Compare<'a> {
    /// Arrow's comparator, which reads the buffers and builds no value.
    Buffers(DynComparator),
    /// The values' total order, one row built per side.
    Values {
        serie: &'a Serie,
        options: SortOptions,
    },
}

impl<'a> Compare<'a> {
    fn new(serie: &'a Serie, options: SortOptions) -> Self {
        if let Some(array) = serie.into_arrow_array()
            && let Ok(compare) =
                make_comparator(array.as_ref(), array.as_ref(), options.into_arrow())
        {
            return Self::Buffers(compare);
        }
        Self::Values { serie, options }
    }

    fn cmp(&self, left: usize, right: usize) -> Ordering {
        match self {
            Self::Buffers(compare) => compare(left, right),
            Self::Values { serie, options } => {
                compare_values(&serie.row_at(left), &serie.row_at(right), *options)
            }
        }
    }
}

/// Two values under `options`: an absent one at the end the options name,
/// two present ones in their own total order, reversed when descending.
pub(crate) fn compare_values(left: &Scalar, right: &Scalar, options: SortOptions) -> Ordering {
    match (left.is_null(), right.is_null()) {
        (true, true) => Ordering::Equal,
        (true, false) => {
            if options.is_nulls_first() {
                Ordering::Less
            } else {
                Ordering::Greater
            }
        }
        (false, true) => {
            if options.is_nulls_first() {
                Ordering::Greater
            } else {
                Ordering::Less
            }
        }
        (false, false) => {
            let step = left.cmp(right);
            if options.is_descending() {
                step.reverse()
            } else {
                step
            }
        }
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
    /// A primitive column sorts its native slice; any other column sorts
    /// through Arrow's row format where the type has one, and through the
    /// values' own total order elsewhere; a run sorts its values. Absent
    /// rows gather to the end `options` names. One allocation beside the
    /// answer: the row format's bytes, where that rung is reached.
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
        if let Some(array) = self.into_arrow_array()
            && let Some(rows) = row_format(&array, options)
        {
            let mut order: Vec<u32> = (0..len as u32).collect();
            order.sort_by(|left, right| rows.row(*left as usize).cmp(&rows.row(*right as usize)));
            return Ok(order);
        }
        let compare = Compare::new(self, options);
        let mut order: Vec<u32> = (0..len as u32).collect();
        order.sort_by(|left, right| compare.cmp(*left as usize, *right as usize));
        Ok(order)
    }

    /// Whether the rows are in sorted order under `options`: one pass over
    /// adjacent rows, through Arrow's comparator over the buffers where the
    /// layout has one and the values' own order elsewhere, so a column
    /// builds no row and a run reads what it holds.
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
        let compare = Compare::new(self, options);
        (1..self.len()).all(|index| compare.cmp(index - 1, index) != Ordering::Greater)
    }

    /// Walk the rows, telling `visit` whether each is the first of its
    /// value; `visit` answers whether to go on. One hash set over the row
    /// format's bytes where the type has one, over the values elsewhere; an
    /// absent row is one value.
    fn walk_distinct(&self, mut visit: impl FnMut(usize, bool) -> bool) {
        let len = self.len();
        if let Some(array) = self.into_arrow_array()
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
                if index == len || compare.cmp(index - 1, index) != Ordering::Equal {
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
    /// one map, keyed by the row format's bytes where the type has one and
    /// by the values elsewhere.
    fn groups(&self) -> Result<Vec<Vec<u32>>> {
        require_indexable(self)?;
        let len = self.len();
        let mut positions: Vec<Vec<u32>> = Vec::new();
        if let Some(array) = self.into_arrow_array()
            && let Some(rows) = row_format(&array, SortOptions::default())
        {
            let mut group_of: std::collections::HashMap<&[u8], usize> =
                std::collections::HashMap::new();
            for index in 0..len {
                let next = positions.len();
                let group = *group_of.entry(rows.row(index).data()).or_insert(next);
                if group == next {
                    positions.push(Vec::new());
                }
                positions[group].push(index as u32);
            }
            return Ok(positions);
        }
        // `Scalar`'s hash reads canonical content only, never the
        // interior-mutable caches a datatype holds, so the key is stable.
        #[allow(clippy::mutable_key_type)]
        let mut group_of: std::collections::HashMap<Scalar, usize> =
            std::collections::HashMap::new();
        for index in 0..len {
            let next = positions.len();
            let group = *group_of
                .entry(self.row_at(index).into_owned())
                .or_insert(next);
            if group == next {
                positions.push(Vec::new());
            }
            positions[group].push(index as u32);
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
    /// and rewrites the validity bits, allocating nothing; a shared buffer
    /// is copied once. Every other column replaces its buffers by the
    /// kernel's one copy, and a run sorts its values in place when it holds
    /// them alone, copied once when it does not.
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
        if let Self::Run(run) = self {
            run.make_mut()
                .sort_by(|left, right| compare_values(left, right, options));
            return Ok(self);
        }
        if self.primitive_sort_in_place(0..len, options) {
            return Ok(self);
        }
        *self = self.into_sorted(options)?;
        Ok(self)
    }

    /// Sort rows `range` of a primitive column where they stand, answering
    /// whether this is one: the typed rung of the in-place ladder.
    pub(crate) fn primitive_sort_in_place(
        &mut self,
        range: Range<usize>,
        options: SortOptions,
    ) -> bool {
        primitive_mut!(self, column => column.sort_in_place(range, options)).is_some()
    }

    /// Reverse rows `range` of a primitive column where they stand,
    /// answering whether this is one.
    pub(crate) fn primitive_reverse_in_place(&mut self, range: Range<usize>) -> bool {
        primitive_mut!(self, column => column.reverse_in_place(range)).is_some()
    }

    /// Keep the first occurrence of every value, in place, answering this
    /// serie: the kernel's one copy replaces the buffers of a column, and a
    /// run keeps its chosen values. The field is kept as it is.
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
    /// bits where they stand; every other column replaces its buffers by
    /// the kernel's one copy, and a run reverses its values in place when
    /// it holds them alone.
    ///
    /// # Errors
    ///
    /// Never, in practice: the signature matches the other `as_*` writes so
    /// calls chain.
    pub fn as_reversed(&mut self) -> Result<&mut Self> {
        let len = self.len();
        if let Self::Run(run) = self {
            run.make_mut().reverse();
            return Ok(self);
        }
        if self.primitive_reverse_in_place(0..len) {
            return Ok(self);
        }
        *self = self.into_reversed();
        Ok(self)
    }

    /// Keep the rows `indices` names, in that order, in place, answering
    /// this serie: the kernel's one copy replaces the buffers of a column.
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
    /// # Errors
    ///
    /// [`Self::into_filtered`]'s refusal, which leaves this serie as it was.
    pub fn as_filtered(&mut self, mask: &Self) -> Result<&mut Self> {
        *self = self.into_filtered(mask)?;
        Ok(self)
    }
}
