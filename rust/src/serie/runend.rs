//! The column a run-end-encoded field is stored in: the run ends over their
//! values, each a column.
//!
//! The run ends are rebased onto the rows this column holds - the first run
//! starts at row 0 and the last ends at the logical length - so a row reads
//! by one binary search over them, and the values column holds exactly one
//! row per run. A write is a cut over the runs it touches: the replacement
//! is folded into runs, a neighbour holding an equal value absorbs the rows
//! beside it rather than starting a run, and only what changed is written -
//! the values column spliced where runs appear or vanish, the run ends from
//! the first one that moves. A push of the value the last run holds is one
//! run-end write, and a push of another value one run appended.

use std::fmt;
use std::ops::Range;
use std::sync::Arc;

use arrow_array::types::{Int16Type, Int32Type, Int64Type, RunEndIndexType};
use arrow_array::{Array, ArrayRef, PrimitiveArray, RunArray, make_array};
use arrow_buffer::{ArrowNativeType, NullBuffer};
use arrow_data::ArrayData;
use arrow_schema::DataType as ArrowDataType;

use super::primitive::{NativeLeaf, PrimitiveLeaf, PrimitiveSerie};
use super::{Serie, proven_row, require_range, require_row, require_window};
use crate::value::SerieValue;
use crate::{DataType, Field, Result, Scalar};

/// The invariant every run-end column keeps: its run ends climb to its
/// length and its values hold one row per run, so the Arrow array assembles.
const ALIGNED: &str = "a run-end column's runs reach its rows: no public path misaligns them";

/// The invariant a write carries in from `check`: the rows fit the run-end
/// width, so no run end it writes can refuse them.
const CHECKED: &str = "check ran on these rows: the run ends they reach fit the run-end width";

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
        match &$column.run_ends {
            Serie::Int16(column) => $function::<Int16Type>(column.values() $(, $argument)*),
            Serie::Int32(column) => $function::<Int32Type>(column.values() $(, $argument)*),
            Serie::Int64(column) => $function::<Int64Type>(column.values() $(, $argument)*),
            _ => unreachable!("{RUN_ENDS}"),
        }
    };
}

/// Call one width-generic function over the run ends column itself, held
/// alone, with the width's Arrow type as its parameter.
macro_rules! over_run_ends_mut {
    ($column:expr, $function:ident($($argument:expr),*)) => {
        match &mut $column.run_ends {
            Serie::Int16(column) => $function::<Int16Type>(Arc::make_mut(column) $(, $argument)*),
            Serie::Int32(column) => $function::<Int32Type>(Arc::make_mut(column) $(, $argument)*),
            Serie::Int64(column) => $function::<Int64Type>(Arc::make_mut(column) $(, $argument)*),
            _ => unreachable!("{RUN_ENDS}"),
        }
    };
}

/// Where a write over rows `range` cuts the runs.
///
/// `runs` are the runs it rewrites: the ones its rows fall in, and the run
/// on either side of it that keeps rows - the part of the run it starts in
/// that lies before it, or the whole run before it when it starts on a
/// boundary, and the part of the run it ends in that lies after it. Holding
/// the neighbours is what lets a replacement equal to one extend it.
struct Cut {
    /// The runs the write rewrites.
    runs: Range<usize>,
    /// The row the first of `runs` starts at.
    base: usize,
    /// The rows the first of `runs` keeps before the write, if it keeps any.
    left: Option<usize>,
    /// The rows the last of `runs` keeps after the write, if it keeps any.
    right: Option<usize>,
}

/// Cut `ends`, whose last run ends at `len`, around rows `range`.
fn cut<R: RunEndIndexType>(ends: &[R::Native], range: &Range<usize>, len: usize) -> Cut {
    let count = ends.len();
    let end_of = |run: usize| ends[run].as_usize();
    let start_of = |run: usize| if run == 0 { 0 } else { end_of(run - 1) };
    let first = if range.start < len {
        run_of::<R>(ends, range.start)
    } else {
        count
    };
    let head = range.start - start_of(first);
    let (lo, base, left) = if head > 0 {
        (first, start_of(first), Some(head))
    } else if first > 0 {
        let before = first - 1;
        (
            before,
            start_of(before),
            Some(end_of(before) - start_of(before)),
        )
    } else {
        (0, 0, None)
    };
    let last = if range.end < len {
        run_of::<R>(ends, range.end)
    } else {
        count
    };
    let (hi, right) = if last < count {
        (last + 1, Some(end_of(last) - range.end))
    } else {
        (count, None)
    };
    Cut {
        runs: lo..hi,
        base,
        left,
        right,
    }
}

/// Rewrite run ends `runs` as `region` and move every later one by `shift`,
/// writing only from the first run end that changes - and, when nothing
/// after the cut moves, only up to the last.
fn rewrite_ends<R: RunEndIndexType + NativeLeaf>(
    column: &mut PrimitiveSerie<R>,
    runs: Range<usize>,
    region: &[usize],
    shift: isize,
) {
    let held = column.values();
    let old = &held[runs.clone()];
    let same = |(new, old): (&usize, &R::Native)| *new == old.as_usize();
    let head = region
        .iter()
        .zip(old)
        .take_while(|pair| same(*pair))
        .count();
    let (from, to, written): (usize, usize, Vec<usize>) = if shift == 0 {
        let tail = region[head..]
            .iter()
            .rev()
            .zip(old[head..].iter().rev())
            .take_while(|pair| same(*pair))
            .count();
        (
            runs.start + head,
            runs.end - tail,
            region[head..region.len() - tail].to_vec(),
        )
    } else {
        let moved = held[runs.end..]
            .iter()
            .map(|end| end.as_usize().checked_add_signed(shift).expect(ALIGNED));
        (
            runs.start + head,
            held.len(),
            region[head..].iter().copied().chain(moved).collect(),
        )
    };
    if from == to && written.is_empty() {
        return;
    }
    let written = written
        .into_iter()
        .map(|end| Some(R::Native::from_usize(end).expect(CHECKED)))
        .collect();
    column.splice_values(from..to, written).expect(RUN_ENDS);
}

/// Append `ends`, another column's run ends, each moved past `shift` rows.
fn extend_ends<R: RunEndIndexType + NativeLeaf>(
    column: &mut PrimitiveSerie<R>,
    ends: &[usize],
    shift: usize,
) {
    let written = ends
        .iter()
        .map(|end| Some(R::Native::from_usize(end + shift).expect(CHECKED)))
        .collect();
    column.extend_values(written).expect(RUN_ENDS);
}

/// The run ends of `ends` as row counts.
fn ends_of<R: RunEndIndexType>(ends: &[R::Native]) -> Vec<usize> {
    ends.iter().map(|end| end.as_usize()).collect()
}

/// What a write does to the runs: runs `runs` become `ends`, the values
/// column's rows `values` become `replacement`, and every later run end
/// moves by `shift`.
struct Plan {
    runs: Range<usize>,
    ends: Vec<usize>,
    values: Range<usize>,
    replacement: Vec<Scalar>,
    shift: isize,
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
) -> (Serie, Range<usize>)
where
    R::Native: Into<Scalar>,
{
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
    // Run ends are plain integers, read as they are stored.
    let reading = crate::serie::value::read_native::<R::Native>;
    (
        PrimitiveSerie::<R>::new(field, rebased, reading).into_serie(),
        runs,
    )
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

    /// Plan a write of canonical `rows` over `range`.
    ///
    /// The replacement is folded into runs of equal neighbours between the
    /// rows the cut keeps on either side, so a value equal to a neighbour
    /// extends its run. A neighbour's run keeps its values row - the one
    /// before the write always, the one after it where it is not the same
    /// run or swallowed by the one before - so the values column is spliced
    /// only where runs appear or vanish.
    fn plan(&self, range: &Range<usize>, rows: &[Scalar]) -> Plan {
        let cut = over_run_ends!(self, cut(range, self.len));
        let mut runs: Vec<(Scalar, usize)> = Vec::new();
        let mut fold = |value: Scalar, length: usize| match runs.last_mut() {
            Some((held, count)) if *held == value => *count += length,
            _ => runs.push((value, length)),
        };
        if let Some(length) = cut.left {
            fold(proven_row(&self.values, cut.runs.start), length);
        }
        for row in rows {
            fold(row.clone(), 1);
        }
        if let Some(length) = cut.right {
            fold(proven_row(&self.values, cut.runs.end - 1), length);
        }
        let keep_left = usize::from(cut.left.is_some());
        let keep_right = usize::from(
            cut.right.is_some() && (keep_left == 0 || (cut.runs.len() > 1 && runs.len() > 1)),
        );
        let mut end = cut.base;
        let ends = runs
            .iter()
            .map(|(_, length)| {
                end += length;
                end
            })
            .collect();
        let replacement = runs[keep_left..runs.len() - keep_right]
            .iter()
            .map(|(value, _)| value.clone())
            .collect();
        Plan {
            values: cut.runs.start + keep_left..cut.runs.end - keep_right,
            runs: cut.runs,
            ends,
            replacement,
            shift: isize::try_from(rows.len()).expect(ALIGNED)
                - isize::try_from(range.len()).expect(ALIGNED),
        }
    }

    /// Refuse what a write could not do: a length past the run-end width,
    /// or what the values column refuses for the runs it would gain.
    pub(crate) fn check(&self, range: &Range<usize>, rows: &[Scalar]) -> Result<()> {
        self.require_fit(self.len - range.len() + rows.len())?;
        let plan = self.plan(range, rows);
        self.values.check(&plan.values, &plan.replacement)
    }

    /// Write canonical `rows` over a checked `range`, in place: the values
    /// column spliced where runs appear or vanish, the run ends from the
    /// first one that moves.
    pub(crate) fn write(&mut self, range: Range<usize>, rows: Vec<Scalar>) {
        let plan = self.plan(&range, &rows);
        if !(plan.values.is_empty() && plan.replacement.is_empty()) {
            self.values.write(plan.values, plan.replacement);
        }
        over_run_ends_mut!(self, rewrite_ends(plan.runs, &plan.ends, plan.shift));
        self.len = self.len - range.len() + rows.len();
    }

    /// Append `other`'s rows, whose field agrees with this one's, answering
    /// whether the run-end width reaches the total; when it does not,
    /// nothing moved.
    ///
    /// The values append buffer to buffer and the run ends follow, moved
    /// past the rows already held; two equal runs meeting at the seam stay
    /// two runs, which the layout allows.
    pub(crate) fn append(&mut self, other: &Self) -> bool {
        if !over_run_ends!(self, fits(self.len + other.len)) {
            return false;
        }
        let ends = over_run_ends!(other, ends_of());
        if !self.values.append(&other.values) {
            return false;
        }
        over_run_ends_mut!(self, extend_ends(&ends, self.len));
        self.len += other.len;
        true
    }
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
        super::Leaf::root(self)
    }

    fn from_serie(value: &Serie) -> Option<&Self> {
        super::Leaf::narrow(value)
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
fn rebased<R: RunEndIndexType>(
    held: &RunArray<R>,
    parent: Option<&NullBuffer>,
    budget: &mut crate::budget::MaterializationBudget,
) -> crate::arrow::Result<(ArrayRef, ArrayRef, usize, Option<NullBuffer>)> {
    let ends = held.run_ends();
    let len = ends.len();
    let hidden = if parent.is_some_and(|above| above.null_count() != 0) {
        crate::cast::columns::run_value_exposure(held, parent.map(NullBuffer::inner), budget)?
            .map(NullBuffer::new)
    } else {
        None
    };
    if ends.offset() == 0 && ends.max_value() == len {
        return Ok((
            Arc::new(PrimitiveArray::<R>::new(ends.inner().clone(), None)),
            Arc::clone(held.values()),
            len,
            hidden,
        ));
    }
    let rebased = PrimitiveArray::<R>::from_iter_values(ends.sliced_values());
    let values = held.values_slice();
    let hidden = hidden.map(|mask| mask.slice(held.get_start_physical_index(), values.len()));
    Ok((Arc::new(rebased), values, len, hidden))
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
    proof: &super::arrow::Proof,
    budget: &mut crate::budget::MaterializationBudget,
) -> crate::arrow::Result<Option<Serie>> {
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
            rebased(
                &super::arrow::held::<RunArray<$arrow>>(&array)?,
                parent,
                budget,
            )?
        };
    }
    let (run_ends, values, len, hidden) = match encoded.run_ends().dtype() {
        DataType::Int16 => parts!(Int16Type),
        DataType::Int32 => parts!(Int32Type),
        DataType::Int64 => parts!(Int64Type),
        _ => return Err(internal()),
    };
    let run_ends = super::arrow::child_of(
        Arc::new(encoded.run_ends().clone()),
        run_ends,
        None,
        &super::arrow::Proof::Proven,
        budget,
    )?;
    let values = super::arrow::child_of(
        Arc::new(encoded.values().clone()),
        values,
        hidden.as_ref(),
        proof.child(0),
        budget,
    )?;
    Ok(Some(
        RunEndEncodedSerie::new(field, run_ends, values, len).into_serie(),
    ))
}
