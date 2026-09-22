//! The column a run-end-encoded field is stored in: the run ends over their
//! values, each a column.
//!
//! The run ends are rebased onto the rows this column holds - the first run
//! starts at row 0 and the last ends at the logical length - so a row reads
//! by one binary search over them, and the values column holds exactly one
//! row per run. A write rebuilds: the replacement is laid out at the
//! crate's one scalar-array boundary, joined to the rows around it with
//! Arrow's concatenation, and re-imported through the door with its rows
//! already proven.

use std::fmt;
use std::ops::Range;
use std::sync::Arc;

use arrow_array::types::{Int16Type, Int32Type, Int64Type, RunEndIndexType};
use arrow_array::{Array, ArrayRef, PrimitiveArray, RunArray, make_array, new_empty_array};
use arrow_buffer::{ArrowNativeType, NullBuffer};
use arrow_data::ArrayData;
use arrow_schema::{ArrowError, DataType as ArrowDataType};

use super::primitive::{PrimitiveLeaf, PrimitiveSerie};
use super::{IntegerSerie, Serie, require_range, require_row, require_window};
use crate::value::SerieValue;
use crate::{DataType, Field, Result, Scalar};

/// The invariant every run-end column keeps: its run ends climb to its
/// length and its values hold one row per run, so the Arrow array assembles.
const ALIGNED: &str = "a run-end column's runs reach its rows: no public path misaligns them";

/// The invariant a write carries in from `check`: the rows fit the run-end
/// width, so the rebuild cannot refuse them.
const CHECKED: &str = "check ran on these rows: the run ends they reach fit the run-end width";

/// The invariant a canonical row carries into a write: it lays out as the
/// field's own array, because the field's contract already rewrote it.
const LAID_OUT: &str = "a canonical row lays out as its field's array: the contract rewrote it";

/// The invariant the door keeps: a run-end field's run ends are an integer
/// column of one of the three widths its datatype admits.
const RUN_ENDS: &str =
    "a run-end field's run ends are int16, int32 or int64: the datatype validated it";

/// One column of run-end-encoded rows: the run ends and the values, each a
/// column, over a logical length.
#[derive(Clone)]
pub struct RunEndEncodedSerie {
    field: Arc<Field>,
    run_ends: Serie,
    values: Serie,
    len: usize,
}

/// Call one width-generic function over the run ends as the native slice
/// they are, with the width's Arrow type as its parameter.
macro_rules! over_run_ends {
    ($column:expr, $function:ident($($argument:expr),*)) => {
        match $column.run_ends_family() {
            IntegerSerie::Int16(column) => $function::<Int16Type>(column.values() $(, $argument)*),
            IntegerSerie::Int32(column) => $function::<Int32Type>(column.values() $(, $argument)*),
            IntegerSerie::Int64(column) => $function::<Int64Type>(column.values() $(, $argument)*),
            _ => unreachable!("{RUN_ENDS}"),
        }
    };
}

/// The run row `index` falls in: one binary search over `ends`; the caller
/// keeps `index` below the length.
fn run_of<R: RunEndIndexType>(ends: &[R::Native], index: usize) -> usize {
    ends.partition_point(|end| end.as_usize() <= index)
}

/// Whether `total` rows fit the run-end width: the last run end is the
/// length itself.
fn fits<R: RunEndIndexType>(_: &[R::Native], total: usize) -> bool {
    R::Native::from_usize(total).is_some()
}

/// How many rows the runs of `ends` leave absent, read off `values`'
/// validity once per run.
fn absent_rows<R: RunEndIndexType>(ends: &[R::Native], values: &Serie) -> usize {
    let mut absent = 0;
    let mut start = 0;
    for (run, end) in ends.iter().enumerate() {
        let end = end.as_usize();
        if values.is_null(run).expect(ALIGNED) {
            absent += end - start;
        }
        start = end;
    }
    absent
}

/// The rows `offset..offset + length` as rebased run ends under `field`
/// and the runs they reach: a copy of the run ends the window touches, and
/// nothing of the values.
fn window<R: RunEndIndexType + PrimitiveLeaf>(
    ends: &[R::Native],
    field: Arc<Field>,
    offset: usize,
    length: usize,
) -> (Serie, Range<usize>) {
    let runs = if length == 0 {
        0..0
    } else {
        let first = ends.partition_point(|end| end.as_usize() <= offset);
        let last = ends.partition_point(|end| end.as_usize() < offset + length) + 1;
        first..last
    };
    let rebased = PrimitiveArray::<R>::from_iter_values(
        ends[runs.clone()]
            .iter()
            .map(|end| R::Native::usize_as((end.as_usize() - offset).min(length))),
    );
    (PrimitiveSerie::<R>::new(field, rebased).into_serie(), runs)
}

impl RunEndEncodedSerie {
    /// Pair a run-end field with its run ends, its values and the logical
    /// length they encode.
    pub(crate) const fn new(field: Arc<Field>, run_ends: Serie, values: Serie, len: usize) -> Self {
        Self {
            field,
            run_ends,
            values,
            len,
        }
    }

    /// Borrow the run ends column: an integer column of the run-end datatype.
    pub const fn run_ends(&self) -> &Serie {
        &self.run_ends
    }

    /// Borrow the values column, one row per run.
    pub const fn values(&self) -> &Serie {
        &self.values
    }

    /// Return the logical length: the row count the runs encode.
    pub const fn logical_len(&self) -> usize {
        self.len
    }

    /// The run ends column as the integer family it is.
    fn run_ends_family(&self) -> &IntegerSerie {
        match &self.run_ends {
            Serie::Integer(family) => family,
            _ => unreachable!("{RUN_ENDS}"),
        }
    }

    /// Refuse a length the run-end width cannot reach, naming the column.
    fn require_fit(&self, total: usize) -> Result<()> {
        if over_run_ends!(self, fits(total)) {
            return Ok(());
        }
        Err(crate::Error::InvalidRecord {
            path: smol_str::SmolStr::new(self.field.name()),
            reason: smol_str::format_smolstr!(
                "{total} rows are past the last run end a {} reaches in {}",
                self.run_ends.require_field()?.dtype(),
                self.field.name()
            ),
        })
    }

    /// Lay canonical `rows` out as this column's array, once, at the
    /// crate's one scalar-array boundary.
    fn laid_out(&self, rows: &[Scalar]) -> ArrayRef {
        let borrowed: Vec<&Scalar> = rows.iter().collect();
        crate::arrow::value::array_from_values(&self.field, &borrowed).expect(LAID_OUT)
    }

    /// Take `joined` - this column's storage - as this column, through the
    /// door with its rows already proven.
    fn rebuilt(&self, joined: ArrayRef) -> Self {
        let serie =
            super::arrow::column_of(Arc::clone(&self.field), joined, None, true).expect(CHECKED);
        let Serie::RunEndEncoded(held) = serie else {
            unreachable!("{CHECKED}")
        };
        Arc::unwrap_or_clone(held)
    }

    /// The values the runs of canonical `rows` hold: one per run of equal
    /// neighbours, as the layout cuts them.
    fn runs_of(rows: &[Scalar]) -> Vec<Scalar> {
        let mut runs: Vec<Scalar> = Vec::new();
        for row in rows {
            if runs.last().is_none_or(|previous| previous != row) {
                runs.push(row.clone());
            }
        }
        runs
    }

    /// Refuse what a write could not do: a length past the run-end width,
    /// or what the values column refuses for the runs it would gain.
    pub(crate) fn check(&self, range: &Range<usize>, rows: &[Scalar]) -> Result<()> {
        self.require_fit(self.len - range.len() + rows.len())?;
        let held = self.values.len();
        self.values.check(&(held..held), &Self::runs_of(rows))
    }

    /// Write canonical `rows` over a checked `range`, rebuilding.
    pub(crate) fn write(&mut self, range: Range<usize>, rows: Vec<Scalar>) {
        let array = self.into_arrow_array();
        let joined = joined(
            &self.field,
            vec![
                array.slice(0, range.start),
                self.laid_out(&rows),
                array.slice(range.end, self.len - range.end),
            ],
        )
        .expect(CHECKED);
        *self = self.rebuilt(joined);
    }

    /// Append `other`'s rows, whose field agrees with this one's, answering
    /// whether the run-end width reaches the total; when it does not,
    /// nothing moved.
    pub(crate) fn append(&mut self, other: &Self) -> bool {
        if !over_run_ends!(self, fits(self.len + other.len)) {
            return false;
        }
        let Ok(joined) = joined(
            &self.field,
            vec![self.into_arrow_array(), other.into_arrow_array()],
        ) else {
            return false;
        };
        *self = self.rebuilt(joined);
        true
    }
}

/// Join `pieces` into one array of `field`'s storage, skipping the empty
/// ones: the kernel refuses an empty input, and reads the datatype off the
/// first run array it is given.
fn joined(field: &Field, pieces: Vec<ArrayRef>) -> std::result::Result<ArrayRef, ArrowError> {
    let pieces: Vec<&dyn Array> = pieces
        .iter()
        .filter(|piece| !piece.is_empty())
        .map(|piece| piece.as_ref())
        .collect();
    if pieces.is_empty() {
        let storage = field.as_arrow_field_ref().expect(ALIGNED).data_type();
        return Ok(new_empty_array(storage));
    }
    arrow_select::concat::concat(&pieces)
}

impl SerieValue for RunEndEncodedSerie {
    fn field(&self) -> &Field {
        &self.field
    }

    fn field_ref(&self) -> &Arc<Field> {
        &self.field
    }

    fn len(&self) -> usize {
        self.len
    }

    fn null_count(&self) -> usize {
        if self.values.null_count() == 0 {
            return 0;
        }
        over_run_ends!(self, absent_rows(&self.values))
    }

    fn is_null(&self, index: usize) -> Result<bool> {
        require_row(self.field.name(), index, self.len)?;
        self.values.is_null(over_run_ends!(self, run_of(index)))
    }

    fn scalar(&self, index: usize) -> Result<Scalar> {
        require_row(self.field.name(), index, self.len)?;
        self.values.scalar(over_run_ends!(self, run_of(index)))
    }

    fn slice(&self, offset: usize, length: usize) -> Result<Self> {
        require_window(self.field.name(), offset, length, self.len)?;
        let field = Arc::clone(self.run_ends.field_ref().expect(RUN_ENDS));
        let (run_ends, runs) = over_run_ends!(self, window(field, offset, length));
        Ok(Self::new(
            Arc::clone(&self.field),
            run_ends,
            self.values.slice(runs.start, runs.len())?,
            length,
        ))
    }

    fn splice(&mut self, range: Range<usize>, rows: Vec<Scalar>) -> Result<()> {
        require_range(self.field.name(), &range, self.len)?;
        let canonical = rows
            .into_iter()
            .map(|row| self.field.scalar(row))
            .collect::<Result<Vec<Scalar>>>()?;
        self.check(&range, &canonical)?;
        self.write(range, canonical);
        Ok(())
    }

    fn into_arrow_array(&self) -> ArrayRef {
        let storage = self
            .field
            .as_arrow_field_ref()
            .expect(ALIGNED)
            .data_type()
            .clone();
        let run_ends = self.run_ends.into_arrow_array().expect(ALIGNED);
        let values = self.values.into_arrow_array().expect(ALIGNED);
        let data = ArrayData::builder(storage)
            .len(self.len)
            .add_child_data(run_ends.into_data())
            .add_child_data(values.into_data())
            .build()
            .expect(ALIGNED);
        make_array(data)
    }

    fn into_serie(self) -> Serie {
        Serie::RunEndEncoded(Arc::new(self))
    }

    fn from_serie(value: &Serie) -> Option<&Self> {
        match value {
            Serie::RunEndEncoded(column) => Some(column.as_ref()),
            _ => None,
        }
    }
}

impl fmt::Debug for RunEndEncodedSerie {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        super::debug_column(self, "RunEndEncodedSerie", formatter)
    }
}

serie_leaf!(RunEndEncodedSerie);

/// Rebase a run array's run ends onto exactly the rows it reaches.
///
/// Arrow slices a run array by moving a logical offset over the whole run
/// ends and the whole values, so a sliced array's first run end is past
/// the offset and its values run past the end. An unsliced array shares
/// both buffers untouched; a sliced one copies the run ends it touches and
/// shares the values through Arrow's own slice.
fn rebased<R: RunEndIndexType>(held: &RunArray<R>) -> (ArrayRef, ArrayRef, usize) {
    let ends = held.run_ends();
    let len = ends.len();
    if ends.offset() == 0 && ends.max_value() == len {
        return (
            Arc::new(PrimitiveArray::<R>::new(ends.inner().clone(), None)),
            Arc::clone(held.values()),
            len,
        );
    }
    let rebased = PrimitiveArray::<R>::from_iter_values(ends.sliced_values());
    (Arc::new(rebased), held.values_slice(), len)
}

/// Build the column `field` types out of a run-end-encoded array, or answer
/// `None` for a layout that is not one.
///
/// The run ends take the door as the integer column their field declares,
/// proven by their layout; the values take it under the values field, and
/// are proven there once.
pub(crate) fn column_of(
    field: Arc<Field>,
    array: ArrayRef,
    parent: Option<&NullBuffer>,
    proven: bool,
) -> crate::arrow::Result<Option<Serie>> {
    let _ = parent;
    if !matches!(array.data_type(), ArrowDataType::RunEndEncoded(..)) {
        return Ok(None);
    }
    let internal = || crate::arrow::Error::Internal {
        site: "serie::runend::column_of",
    };
    let DataType::RunEndEncoded(encoded) = field.dtype() else {
        return Err(internal());
    };
    macro_rules! parts {
        ($arrow:ty) => {
            rebased(&super::arrow::held::<RunArray<$arrow>>(&array)?)
        };
    }
    let (run_ends, values, len) = match encoded.run_ends().dtype() {
        DataType::Int16 => parts!(Int16Type),
        DataType::Int32 => parts!(Int32Type),
        DataType::Int64 => parts!(Int64Type),
        _ => return Err(internal()),
    };
    let run_ends =
        super::arrow::column_of(Arc::new(encoded.run_ends().clone()), run_ends, None, true)?;
    let values = super::arrow::column_of(Arc::new(encoded.values().clone()), values, None, proven)?;
    Ok(Some(
        RunEndEncodedSerie::new(field, run_ends, values, len).into_serie(),
    ))
}
