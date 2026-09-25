//! The one door buffers take into a column, and the one they take out.
//!
//! A field says which leaf its buffers are, so this is where that reading
//! happens and nowhere else: the layout is proven against the field's own
//! Arrow projection, absence is judged on the validity words, the values are
//! proven by the layout where the layout is the contract and read once
//! where it is not, and a nested layout recurses with each child field.
//! Value buffers are shared where their physical rows are readable. Hidden
//! narrow values become null placeholders; variable spans containing them
//! are compacted where Arrow requires present children.
//!
//! Inline because this section reads `crate::arrow::{Error, Result}` while
//! the rest of the file reads `crate::{Error, Result}`, and one scope cannot
//! hold both names.

use std::sync::Arc;

use arrow_array::builder::make_builder;
use arrow_array::{
    Array, ArrayRef, OffsetSizeTrait, RecordBatch, RecordBatchIterator, RecordBatchOptions,
    RecordBatchReader, StructArray, new_empty_array,
};
use arrow_buffer::{NullBuffer, OffsetBuffer};

use super::{
    Serie, boolean, bytes, enums, mapping, null, primitive, runend, sequence, structure, union,
    variant,
};
use arrow_schema::{ArrowError, DataType as ArrowDataType, Field as ArrowField, SchemaRef};

use crate::arrow::{
    BatchReader, Error, Result, arrow_schema_from_field, batch_reader, field_from_arrow_schema,
    from_reader_error,
};
use crate::cast::{ArrowCastOptions, ArrowCastPlan, Deferred};
use crate::media::DEFAULT_ROOT_NAME;
use crate::{DataType, Field, Scalar};

/// How many rows this array leaves absent that its parent does not.
///
/// A null record row says nothing about the children under it - Arrow leaves
/// their slots unspecified - so a required child is judged only where the
/// record itself is present. That is one pass over the validity words, never
/// over the rows; an encoding's absence is logical, and read as such.
fn absent_rows(dtype: &DataType, array: &dyn Array, parent: Option<&NullBuffer>) -> usize {
    let nulls = match dtype {
        DataType::Dictionary(_) | DataType::RunEndEncoded(_) | DataType::Union(..) => {
            array.logical_nulls()
        }
        _ => array.nulls().cloned(),
    };
    let Some(nulls) = nulls else {
        return 0;
    };
    match parent {
        Some(above) if above.len() == nulls.len() => {
            (&!nulls.inner() & above.inner()).count_set_bits()
        }
        _ => nulls.null_count(),
    }
}

/// Refuse an array a required field cannot hold, naming the column.
///
/// A null is absence everywhere but where it is the datatype's own
/// canonical default - `null`, and an encoding whose values hold only
/// nulls - so such a column holds its nulls under a required field. That
/// is decided once per level, never per row.
fn require_present(field: &Field, array: &dyn Array, parent: Option<&NullBuffer>) -> Result<()> {
    if field.is_nullable() {
        return Ok(());
    }
    // Counting is a read of the validity words; whether a null is the
    // datatype's own default is a question about the datatype, asked only
    // of a level that holds one. A datatype with no default at all - a
    // record whose required child can hold nothing - has no null default
    // either, so the question is `false`.
    let absent = absent_rows(field.dtype(), array, parent);
    if absent == 0 || matches!(field.dtype().is_default_value(&Scalar::Null), Ok(true)) {
        return Ok(());
    }
    Err(Error::IncompatibleSchema(format!(
        "column {:?} is not null, got {absent} absent of {} rows",
        field.name(),
        array.len(),
    )))
}

/// Whether an exact layout of this datatype lands as its identity plan
/// would answer it.
///
/// Every level's layout is its datatype's contract, so the plan neither
/// rewrites nor reads a value, and no union or encoding sits anywhere below,
/// whose logical nulls the plan repairs where the landing takes a null
/// default.
fn lands_as_is(dtype: &DataType) -> bool {
    match dtype {
        DataType::Union(..) | DataType::Dictionary(_) | DataType::RunEndEncoded(_) => false,
        DataType::Struct(fields) => fields
            .as_fields()
            .iter()
            .all(|field| lands_as_is(field.dtype())),
        DataType::Serie(item)
        | DataType::SerieView(item)
        | DataType::FixedSizeSerie(item, _)
        | DataType::LargeSerie(item)
        | DataType::LargeSerieView(item) => lands_as_is(item.dtype()),
        DataType::Map(map) | DataType::SortedMap(map) => lands_as_is(map.entries.dtype()),
        leaf => leaf.layout_is_contract(),
    }
}

/// Whether the door reads this level's rows to prove them.
///
/// A leaf whose layout is its contract is proven by the layout; a nested
/// layout is proven through its children; an encoding is proven through its
/// values column. What remains is a leaf whose datatype is narrower than its
/// storage, and it is read once here.
fn reads_rows(dtype: &DataType) -> bool {
    !dtype.layout_is_contract()
        && dtype.field_len() == 0
        && !matches!(dtype, DataType::Dictionary(_))
}

/// Read every row the leaf just built holds once, refusing the first one the
/// field's own contract refuses, naming the column and the row.
///
/// Each row is read off the leaf's own typed buffer through the reading its
/// landing resolved, so the proof pays no per-row dispatch on the datatype
/// or downcast of the array; the field's contract is then asked of the value
/// that reading answers. A row hidden under an absent parent record is not
/// read: Arrow leaves its slot unspecified. The refusal names the row as this
/// level counts it; [`land`] and [`land_batch`] restate it in the rows they
/// were handed.
fn prove_rows(field: &Field, serie: &Serie, parent: Option<&NullBuffer>) -> Result<()> {
    let refused = |index: usize, refusal: &dyn std::fmt::Display| Error::InvalidValue {
        path: smol_str::format_smolstr!("$.{}[{index}]", field.name()),
        expected: smol_str::SmolStr::new_static("a value its field accepts"),
        actual: smol_str::format_smolstr!("{refusal}"),
    };
    let rows = serie.len();
    for index in 0..rows {
        if parent.is_some_and(|above| above.len() == rows && above.is_null(index)) {
            continue;
        }
        let value = serie
            .scalar(index)
            .map_err(|refusal| refused(index, &refusal))?;
        field
            .scalar(value)
            .map_err(|refusal| refused(index, &refusal))?;
    }
    Ok(())
}

/// Row `index` of a column landed [`Proof::OnRead`], under the contract of
/// `field`: a value whose layout is not the field's whole contract is asked
/// of it here, once, as the landing would have asked it.
///
/// # Errors
///
/// Returns the reading's refusal, or the field's.
pub(crate) fn proven_cell(field: &Field, serie: &Serie, index: usize) -> crate::Result<Scalar> {
    let value = serie.scalar(index)?;
    if value.is_null() || field.dtype().layout_is_contract() {
        return Ok(value);
    }
    field.scalar(value)
}

/// Downcast one array to the concrete layout its field proved it is.
///
/// The projection was already compared, so a failure here is this module
/// disagreeing with itself rather than a caller handing over the wrong
/// array.
pub(crate) fn held<A: Array + Clone + 'static>(array: &ArrayRef) -> Result<A> {
    held_ref::<A>(array).cloned()
}

/// Borrow the typed array behind one Arrow array, for a module that reads
/// its parts and keeps none of it.
pub(crate) fn held_ref<A: Array + 'static>(array: &ArrayRef) -> Result<&A> {
    array.as_any().downcast_ref::<A>().ok_or(Error::Internal {
        site: "serie::arrow::held",
    })
}

/// Rebase a cut onto exactly the items it reaches.
///
/// Arrow slices a list by slicing its offsets and keeping the whole child,
/// so a sliced array's offsets start past zero and its values run past the
/// end. Reading is unaffected - an offset is absolute - but a column that
/// grows has to know where its items end, so the cut is rebased once here
/// and the child sliced to match. An unsliced array is already in that
/// shape and is returned untouched; a sliced one shares its buffers through
/// Arrow's own slice.
pub(crate) fn rebased<O: OffsetSizeTrait>(
    offsets: &OffsetBuffer<O>,
    values: &ArrayRef,
) -> (OffsetBuffer<O>, ArrayRef) {
    let first = offsets.first().copied().unwrap_or_default();
    let last = offsets.last().copied().unwrap_or_default();
    if first == O::zero() && last.as_usize() == values.len() {
        return (offsets.clone(), Arc::clone(values));
    }
    let shifted: Vec<O> = offsets.iter().map(|offset| *offset - first).collect();
    (
        OffsetBuffer::new(shifted.into()),
        values.slice(first.as_usize(), (last - first).as_usize()),
    )
}

/// Combine the two contextual masks without changing stored row validity.
pub(super) fn parent_nulls(
    parent: Option<&NullBuffer>,
    own: Option<&NullBuffer>,
    budget: &mut crate::budget::MaterializationBudget,
) -> Result<Option<NullBuffer>> {
    let parent = parent.filter(|nulls| nulls.null_count() != 0);
    let own = own.filter(|nulls| nulls.null_count() != 0);
    if let (Some(parent), Some(_)) = (parent, own) {
        budget.add_bitmap(parent.len())?;
    }
    Ok(NullBuffer::union(parent, own))
}

/// The entries a present offset row reaches. Hidden physical spans stay shared.
pub(super) fn offset_parent<O: OffsetSizeTrait>(
    offsets: &OffsetBuffer<O>,
    len: usize,
    nulls: Option<&NullBuffer>,
    budget: &mut crate::budget::MaterializationBudget,
) -> Result<Option<NullBuffer>> {
    if nulls.is_none_or(|nulls| nulls.null_count() == 0) {
        return Ok(None);
    }
    crate::cast::columns::range_exposure(
        len,
        offsets.len() - 1,
        nulls.map(NullBuffer::inner),
        |_| true,
        |row| Ok((offsets[row].as_usize(), offsets[row + 1].as_usize())),
        budget,
    )
    .map(|mask| mask.map(NullBuffer::new))
}

/// Copy selected physical spans through Arrow, after reserving bounded
/// buffers and scratch. One contiguous span remains a shared slice.
pub(super) fn gather_ranges(
    values: &ArrayRef,
    dtype: &DataType,
    ranges: &[(usize, usize)],
    budget: &mut crate::budget::MaterializationBudget,
) -> Result<ArrayRef> {
    match ranges {
        [] => return Ok(values.slice(0, 0)),
        &[(start, end)] => return Ok(values.slice(start, end - start)),
        _ => {}
    }
    if let DataType::RunEndEncoded(encoded) = dtype {
        use arrow_array::types::{Int16Type, Int32Type, Int64Type};
        return match encoded.run_ends().dtype() {
            DataType::Int16 => gather_runs::<Int16Type>(values, encoded, ranges, budget),
            DataType::Int32 => gather_runs::<Int32Type>(values, encoded, ranges, budget),
            DataType::Int64 => gather_runs::<Int64Type>(values, encoded, ranges, budget),
            _ => Err(Error::Internal {
                site: "serie::arrow::gather_runs",
            }),
        };
    }
    // Arrow's range extender rescans nested runs from their first value.
    // Take avoids that, but loses row counts on zero-width fixed lists.
    let (runs, zero_width) = gather_layout(dtype);
    if runs && !zero_width {
        use crate::budget::{SourceSelection, reserve_source_selection, scratch_vec};
        let selection = SourceSelection::Ranges(ranges);
        let len = selection.row_count(values.len())?;
        let allocation = budget.mark();
        reserve_source_selection(values.as_ref(), dtype, selection, budget)?;
        budget.add_growth_since(allocation)?;
        let mut indices = scratch_vec::<u64>(budget, len, "gather indices")?;
        for &(start, end) in ranges {
            indices.extend((start..end).map(|index| index as u64));
        }
        return Ok(arrow_select::take::take(
            values.as_ref(),
            &arrow_array::UInt64Array::from(indices),
            None,
        )?);
    }
    let allocation = budget.mark();
    crate::budget::reserve_source_extend(
        values.as_ref(),
        dtype,
        crate::budget::SourceSelection::Ranges(ranges),
        budget,
    )?;
    crate::budget::reserve_to_data_scratch(values, budget)?;
    budget.add_growth_since(allocation)?;
    let data = values.to_data();
    // Zero initial capacity avoids reserving inactive wide union children.
    let mut output = arrow_data::transform::MutableArrayData::new(vec![&data], false, 0);
    for &(start, end) in ranges {
        output.try_extend(0, start, end)?;
    }
    Ok(arrow_array::make_array(output.freeze()))
}

/// Which Arrow gather limitations occur in the child tree.
fn gather_layout(dtype: &DataType) -> (bool, bool) {
    let mut runs = matches!(dtype, DataType::RunEndEncoded(_));
    let mut zero_width = matches!(dtype, DataType::FixedSizeSerie(_, 0));
    for index in 0..dtype.field_len() {
        let child = dtype.get_field_at(index).expect("a declared child");
        let (child_runs, child_zero_width) = gather_layout(child.dtype());
        runs |= child_runs;
        zero_width |= child_zero_width;
    }
    (runs, zero_width)
}

/// Select physical runs by binary-searching each cut once. Arrow's generic
/// range extender scans from its first run for every cut, which is quadratic
/// for alternating visible rows. Typed slices borrow both source buffers.
fn gather_runs<R: arrow_array::types::RunEndIndexType>(
    values: &ArrayRef,
    encoded: &Arc<crate::RunEndEncodedType>,
    ranges: &[(usize, usize)],
    budget: &mut crate::budget::MaterializationBudget,
) -> Result<ArrayRef> {
    use crate::budget::{SourceSelection, scratch_vec};
    use arrow_array::{PrimitiveArray, RunArray};
    use arrow_buffer::ArrowNativeType;

    let source = held::<RunArray<R>>(values)?;
    let len = SourceSelection::Ranges(ranges).row_count(source.len())?;
    if R::Native::from_usize(len).is_none() {
        return Err(Error::IncompatibleSchema(format!(
            "run-end logical length {len} does not fit {}",
            encoded.run_ends().dtype(),
        )));
    }
    let mut runs = 0_usize;
    for &(start, end) in ranges {
        if start != end {
            let cut = source.slice(start, end - start);
            runs = runs
                .checked_add(cut.get_end_physical_index() - cut.get_start_physical_index() + 1)
                .ok_or_else(|| {
                    Error::IncompatibleSchema("selected run count exceeds usize".into())
                })?;
        }
    }
    budget.add_array_layout(&DataType::RunEndEncoded(Arc::clone(encoded)), len)?;
    budget.add_array_layout(encoded.run_ends().dtype(), runs)?;
    let mut physical = scratch_vec::<(usize, usize)>(budget, ranges.len(), "run value ranges")?;
    for &(start, end) in ranges {
        if start != end {
            let cut = source.slice(start, end - start);
            physical.push((
                cut.get_start_physical_index(),
                cut.get_end_physical_index() + 1,
            ));
        }
    }
    let gathered = gather_ranges(source.values(), encoded.values().dtype(), &physical, budget)?;
    crate::budget::reserve_vec_bytes::<arrow_data::ArrayData>(budget, 2)?;
    crate::budget::reserve_vec_bytes::<arrow_buffer::Buffer>(budget, 1)?;
    crate::budget::reserve_to_data_scratch(&gathered, budget)?;
    // The retained run-end buffer was reserved above, separately from scratch.
    let mut ends = Vec::new();
    ends.try_reserve_exact(runs).map_err(|error| {
        Error::IncompatibleSchema(format!("run-end output allocation failed: {error}"))
    })?;
    let mut written = 0_usize;
    for &(start, end) in ranges {
        if start != end {
            let cut = source.slice(start, end - start);
            for end in cut.run_ends().sliced_values() {
                ends.push(
                    R::Native::from_usize(written + end.as_usize())
                        .expect("the whole selected length fits the run-end type"),
                );
            }
            written += end - start;
        }
    }
    let ends = PrimitiveArray::<R>::new(arrow_buffer::ScalarBuffer::from(ends), None);
    let data = arrow_data::ArrayData::builder(values.data_type().clone())
        .len(len)
        .child_data(vec![ends.into_data(), gathered.to_data()])
        .build()?;
    Ok(arrow_array::make_array(data))
}

/// Drop hidden variable spans whose children require logical proof. Other
/// spans can remain borrowed: every physical value is already readable.
pub(super) fn compact_offsets<O: OffsetSizeTrait>(
    offsets: OffsetBuffer<O>,
    values: ArrayRef,
    dtype: &DataType,
    nulls: Option<&NullBuffer>,
    budget: &mut crate::budget::MaterializationBudget,
) -> Result<(OffsetBuffer<O>, ArrayRef)> {
    if dtype.layout_is_contract()
        || nulls.is_none_or(|nulls| nulls.null_count() == 0)
        || !offsets
            .windows(2)
            .enumerate()
            .any(|(row, pair)| nulls.is_some_and(|nulls| nulls.is_null(row)) && pair[0] != pair[1])
    {
        return Ok((offsets, values));
    }
    use crate::budget::{SourceSelection, scratch_vec, selected_child_ranges};
    let rows = offsets.len() - 1;
    let ranges = selected_child_ranges(
        SourceSelection::Ranges(&[(0, rows)]),
        rows,
        values.len(),
        |row| nulls.is_none_or(|nulls| nulls.is_valid(row)),
        |row| Ok((offsets[row].as_usize(), offsets[row + 1].as_usize())),
        budget,
    )?;
    let mut rebuilt = scratch_vec::<O>(budget, offsets.len(), "offsets")?;
    let mut end = O::zero();
    rebuilt.push(end);
    for (row, pair) in offsets.windows(2).enumerate() {
        if nulls.is_none_or(|nulls| nulls.is_valid(row)) {
            end += pair[1] - pair[0];
        }
        rebuilt.push(end);
    }
    let values = gather_ranges(&values, dtype, &ranges, budget)?;
    Ok((OffsetBuffer::new(rebuilt.into()), values))
}

/// What a landing may take on trust about the rows it is handed.
///
/// A column holds only rows its field accepts - [`Serie::scalar`] and the
/// identity built on it rely on that - so every non-contract leaf is read
/// once unless its caller states otherwise here. `Proven` is stated only for
/// rows the crate laid out itself, a selection of a column that already
/// landed, or the output of a cast node that read every value under the
/// target's rule. An extension label is never evidence: foreign bytes that
/// merely claim a datatype are read.
#[derive(Clone, Debug)]
pub(crate) enum Proof {
    /// Nothing is known: every non-contract leaf is read once.
    Unproven,
    /// Every row was laid out, selected from a landed column, or certified.
    Proven,
    /// Nothing is known, and the one reader the landing is private to proves
    /// each row as it takes it, through [`proven_cell`]: a row it never
    /// reads is never proven, so the column never leaves that reader. What a
    /// reader of one row at a time states, where proving the whole batch
    /// first would make its first row cost every row.
    OnRead,
    /// A nested column whose children differ, in the order the landing
    /// recurses: record children in order, a list's item, a map's entries,
    /// a union's members, an encoding's values.
    Children(Arc<[Proof]>),
}

/// The proof a missing child answers: none.
static UNPROVEN: Proof = Proof::Unproven;

impl Proof {
    /// The proof child `index` lands under.
    pub(crate) fn child(&self, index: usize) -> &Self {
        match self {
            Self::Children(children) => children.get(index).unwrap_or(&UNPROVEN),
            whole => whole,
        }
    }

    /// A map cast proves its keys even when its entry leaves remain unproven.
    pub(crate) fn map(entries: Self) -> Self {
        if matches!(entries, Self::Proven) {
            Self::Proven
        } else {
            Self::Children([entries].into())
        }
    }

    /// Collapse a tree whose children all agree into the one answer.
    pub(crate) fn of_children(children: Vec<Self>) -> Self {
        if children.iter().all(|child| matches!(child, Self::Proven)) {
            Self::Proven
        } else if children.iter().all(|child| matches!(child, Self::Unproven)) {
            Self::Unproven
        } else {
            Self::Children(children.into())
        }
    }

    /// Whether this level's own rows are read at the landing.
    const fn reads_at_landing(&self) -> bool {
        !matches!(self, Self::Proven | Self::OnRead)
    }
}

/// The boxed field every level of one root lands under, resolved once.
///
/// A record's children lie in one shared `Arc<[Field]>`, and a landed
/// column holds an `Arc<Field>` of its own, so a landing boxes a clone of
/// every child field it passes: one allocation per child per batch, under a
/// root that never changes. A holder landing many batches under one root -
/// a plan, a reader - resolves those boxes once here, and every landing
/// borrows them; a one-off landing passes `None` and boxes as it goes. A
/// node hands a child out only for the very slot it was resolved from,
/// compared by address in the root's shared storage, so a tree built from
/// another root mislabels nothing: it is simply not asked.
///
/// Bounded by the root's node count, which [`Field::validate_bounded`]
/// caps, and built once per plan or per holder.
pub(crate) struct Resolved {
    field: Arc<Field>,
    /// The address of the slot `field` was resolved from: its place in the
    /// parent's shared storage, or the root's own box.
    origin: usize,
    /// The children in the order their module lands them: a record's
    /// fields, a serie's item, a map's entries, a union's members, a
    /// run-end encoding's values then its run ends, a dictionary's values
    /// then its keys.
    children: Box<[Resolved]>,
}

impl Resolved {
    /// Resolve `root` and every level below it, once.
    ///
    /// The root is one its holder already proved bounded - at its Arrow
    /// import, at the plan's compile, or by [`Field::validate_bounded`]
    /// at a one-off door - so the walk here is bounded by that proof.
    pub(crate) fn of(root: Arc<Field>) -> Self {
        Self::from_arc(&root)
    }

    fn from_arc(field: &Arc<Field>) -> Self {
        Self {
            field: Arc::clone(field),
            origin: Arc::as_ptr(field) as usize,
            children: Self::children_of(field),
        }
    }

    /// A child boxed from the slot it lies in; its own children are read
    /// off the box, which shares the slot's nested storage.
    fn from_slot(slot: &Field) -> Self {
        let field = Arc::new(slot.clone());
        Self {
            children: Self::children_of(&field),
            field,
            origin: std::ptr::from_ref(slot) as usize,
        }
    }

    /// A child built for a level whose module builds its field - a
    /// dictionary's keys and values - keyed by the address of what it is
    /// built from.
    fn built(field: Field, origin: usize) -> Self {
        let field = Arc::new(field);
        Self {
            children: Self::children_of(&field),
            field,
            origin,
        }
    }

    fn children_of(field: &Field) -> Box<[Self]> {
        match field.dtype() {
            DataType::Struct(children) => {
                children.as_fields().iter().map(Self::from_slot).collect()
            }
            serie @ (DataType::Serie(_)
            | DataType::SerieView(_)
            | DataType::FixedSizeSerie(..)
            | DataType::LargeSerie(_)
            | DataType::LargeSerieView(_)) => {
                // The views hand out the shared boxes the datatype holds,
                // so an address read through them is the storage's own.
                let sequence = serie.as_serie_type().expect("the variant was just matched");
                Box::new([Self::from_arc(sequence.item_ref())])
            }
            map @ (DataType::Map(_) | DataType::SortedMap(_)) => {
                let mapping = map.as_mapping().expect("the variant was just matched");
                Box::new([Self::from_slot(mapping.entries())])
            }
            DataType::Union(members, _) => members
                .iter()
                .map(|(_, member)| Self::from_slot(member))
                .collect(),
            DataType::RunEndEncoded(encoded) => Box::new([
                Self::from_slot(encoded.values()),
                Self::from_slot(encoded.run_ends()),
            ]),
            DataType::Dictionary(dictionary) => {
                let origin = std::ptr::from_ref(dictionary) as usize;
                Box::new([
                    Self::built(
                        Field::new(field.name(), dictionary.value().clone(), true),
                        origin,
                    ),
                    Self::built(
                        Field::new(field.name(), dictionary.key().clone(), true),
                        origin,
                    ),
                ])
            }
            _ => Box::new([]),
        }
    }

    /// The child at `index`, when it was resolved from the slot at `origin`.
    fn child(&self, index: usize, origin: usize) -> Option<&Self> {
        self.children
            .get(index)
            .filter(|child| child.origin == origin)
    }
}

/// The box child `index` lands under and the node below it: the one
/// resolved for `slot`, else - for a one-off landing, or a tree built from
/// another root - a box of the slot's field, as every landing paid before.
pub(crate) fn resolved_child<'a>(
    resolved: Option<&'a Resolved>,
    index: usize,
    slot: &Field,
) -> (Arc<Field>, Option<&'a Resolved>) {
    resolved_or(resolved, index, std::ptr::from_ref(slot) as usize, || {
        slot.clone()
    })
}

/// [`resolved_child`] for a level whose module builds the child's field
/// itself, keyed by the address of what it builds it from.
pub(crate) fn resolved_or(
    resolved: Option<&Resolved>,
    index: usize,
    origin: usize,
    build: impl FnOnce() -> Field,
) -> (Arc<Field>, Option<&Resolved>) {
    match resolved.and_then(|node| node.child(index, origin)) {
        Some(child) => (Arc::clone(&child.field), Some(child)),
        None => (Arc::new(build()), None),
    }
}

/// [`resolved_child`] for a serie's item, which its datatype already boxes:
/// the box is the item's own either way, and only the node below it is
/// what the tree answers.
pub(crate) fn resolved_item<'a>(
    resolved: Option<&'a Resolved>,
    item: &Arc<Field>,
) -> (Arc<Field>, Option<&'a Resolved>) {
    (
        Arc::clone(item),
        resolved.and_then(|node| node.child(0, Arc::as_ptr(item) as usize)),
    )
}

/// One module's door: the column its layout is, or `None` for another's.
type ColumnOf = fn(
    Arc<Field>,
    ArrayRef,
    Option<&NullBuffer>,
    &Proof,
    &mut crate::budget::MaterializationBudget,
    Option<&Resolved>,
) -> Result<Option<Serie>>;

/// Whether an array laid out as `layout` is exactly the column `field`
/// declares, so it lands as it stands: a layout that is its datatype's
/// whole contract, under the field's own projection. The one rule every
/// exact door - an array, a batch, a chunked array - answers.
///
/// # Errors
///
/// Returns an error when `field` has no Arrow projection.
pub(crate) fn lands_exactly(field: &Field, layout: &ArrowDataType) -> Result<bool> {
    Ok(lands_as_is(field.dtype()) && field.as_arrow_field_ref()?.data_type() == layout)
}

/// Build the column `field` types out of buffers that already hold it.
///
/// The root proves the layout once: the datatype's depth bound and the
/// field's whole Arrow projection, which already states every child's
/// layout, so [`child_of`] below does not compare them again. Every level
/// then proves its own absence and its own values, so a child whose
/// buffers disagree with the child field is refused where it lies rather
/// than at the row that reads it. `proof` says which rows went through the
/// field's contract already, so they are not read again.
pub(crate) fn column_of(
    field: Arc<Field>,
    array: ArrayRef,
    parent: Option<&NullBuffer>,
    proof: &Proof,
) -> Result<Serie> {
    field.dtype().validate_bounded()?;
    crate::arrow::require_projection(&field, array.as_ref())?;
    child_of(
        field,
        array,
        parent,
        proof,
        &mut crate::budget::MaterializationBudget::default(),
        None,
    )
}

/// Build one level of a column whose layout its root already proved: its
/// absence, its values where they are narrower than their storage, and the
/// leaf its layout is. Each module answers `None` for a layout that is not
/// its own, tried in the order the families are listed; the variant pair is
/// tried before the record module because it projects to a struct.
pub(crate) fn child_of(
    field: Arc<Field>,
    array: ArrayRef,
    parent: Option<&NullBuffer>,
    proof: &Proof,
    budget: &mut crate::budget::MaterializationBudget,
    resolved: Option<&Resolved>,
) -> Result<Serie> {
    require_present(&field, array.as_ref(), parent)?;
    // A public child must also be readable independently of its parent.
    // Encodings without their own bitmap mask their physical values below.
    let array = if !field.dtype().layout_is_contract()
        && !matches!(
            field.dtype(),
            DataType::Union(..) | DataType::RunEndEncoded(_)
        )
        && parent.is_some_and(|above| above.null_count() != 0)
    {
        let nulls = parent_nulls(parent, array.nulls(), budget)?;
        crate::budget::reserve_to_data_scratch(&array, budget)?;
        arrow_array::make_array(array.to_data().into_builder().nulls(nulls).build()?)
    } else {
        array
    };
    let proves = proof.reads_at_landing() && reads_rows(field.dtype());
    let modules: [ColumnOf; 11] = [
        null::column_of,
        boolean::column_of,
        primitive::column_of,
        bytes::column_of,
        variant::column_of,
        structure::column_of,
        sequence::column_of,
        mapping::column_of,
        union::column_of,
        enums::column_of,
        runend::column_of,
    ];
    for module in modules {
        if let Some(serie) = module(
            Arc::clone(&field),
            Arc::clone(&array),
            parent,
            proof,
            budget,
            resolved,
        )
        .map_err(|refusal| match refusal {
            Error::PhysicalLimit { .. } => Error::Core(crate::Error::InvalidRecord {
                path: field.name().into(),
                reason: smol_str::format_smolstr!("{refusal}"),
            }),
            other => other,
        })? {
            if proves {
                prove_rows(&field, &serie, parent)?;
            }
            return Ok(serie);
        }
    }
    Err(Error::Unsupported {
        kind: "serie",
        reason: format!(
            "column {:?} lays out as {}, which no column holds",
            field.name(),
            array.data_type()
        ),
    })
}

/// The field an Arrow array of no declared field is the column of: its own
/// layout, named `item`, nullable where the caller read an absent row - its
/// logical nulls, so a null behind a dictionary key, a run or a union member
/// counts as one.
///
/// # Errors
///
/// Returns an error when the layout imports as no datatype.
pub(crate) fn item_field(storage: &arrow_schema::DataType, nullable: bool) -> crate::Result<Field> {
    Ok(Field::new(
        "item",
        DataType::from_arrow_datatype(storage)?,
        nullable,
    ))
}

/// Whether `holds` answers for `storage` or any datatype nested in it.
pub(crate) fn storage_holds(
    storage: &arrow_schema::DataType,
    holds: &impl Fn(&arrow_schema::DataType) -> bool,
) -> bool {
    use arrow_schema::DataType as A;
    holds(storage)
        || match storage {
            A::List(item)
            | A::LargeList(item)
            | A::ListView(item)
            | A::LargeListView(item)
            | A::FixedSizeList(item, _)
            | A::Map(item, _) => storage_holds(item.data_type(), holds),
            A::Struct(fields) => fields
                .iter()
                .any(|field| storage_holds(field.data_type(), holds)),
            A::Union(members, _) => members
                .iter()
                .any(|(_, member)| storage_holds(member.data_type(), holds)),
            A::Dictionary(key, values) => storage_holds(key, holds) || storage_holds(values, holds),
            A::RunEndEncoded(run_ends, values) => {
                storage_holds(run_ends.data_type(), holds)
                    || storage_holds(values.data_type(), holds)
            }
            _ => false,
        }
}

/// The storage an empty column of `field` lays out as, refusing a union of
/// no member wherever one lies: Arrow builds no empty column of one - it
/// reads the first member's type id - so none is laid out.
fn empty_storage(field: &Field) -> crate::Result<arrow_schema::DataType> {
    let storage = field.as_arrow_field_ref()?.data_type();
    let memberless = |node: &arrow_schema::DataType| matches!(node, arrow_schema::DataType::Union(members, _) if members.is_empty());
    if storage_holds(storage, &memberless) {
        return Err(Error::Unsupported {
            kind: "serie",
            reason: format!(
                "column {:?} lays out as {storage}, and a union of no member lays out no column",
                field.name()
            ),
        }
        .into());
    }
    Ok(storage.clone())
}

/// Land one array as the column of `field`, under what its caller proved.
///
/// A value a leaf refuses is named by the row of `array` it lies in and
/// the path below it.
pub(crate) fn land(field: Arc<Field>, array: ArrayRef, proof: &Proof) -> Result<Serie> {
    land_under(field, array, None, proof)
}

/// Land one array whose layout its caller proved once for every array it
/// hands over: the field's depth bound and Arrow projection were compared
/// when a reader resolved its columns, and the batch this array comes from
/// shares the schema they were compared against - which fixes every
/// column's datatype - so only absence and the values are this array's own.
pub(crate) fn land_resolved(field: Arc<Field>, array: ArrayRef, proof: &Proof) -> Result<Serie> {
    child_of(
        Arc::clone(&field),
        Arc::clone(&array),
        None,
        proof,
        &mut crate::budget::MaterializationBudget::default(),
        None,
    )
    .map_err(|refusal| located(&field, array.as_ref(), refusal))
}

/// Land an array whose layout its caller already proved against the
/// root's projection: the output of a compiled plan, which asserts at every
/// node it applies that what it hands back lays out as the target's
/// projection, or an array an exact door compared itself. The root's shape
/// was proved once when the tree was resolved, so neither is compared
/// again here: only absence and the values are this array's own, and the
/// boxes every level lands under are the tree's.
pub(crate) fn land_planned(resolved: &Resolved, array: ArrayRef, proof: &Proof) -> Result<Serie> {
    child_of(
        Arc::clone(&resolved.field),
        Arc::clone(&array),
        None,
        proof,
        &mut crate::budget::MaterializationBudget::default(),
        Some(resolved),
    )
    .map_err(|refusal| located(&resolved.field, array.as_ref(), refusal))
}

/// Land one child array beneath the validity of the record it sits in: a
/// row the parent leaves absent says nothing about the child, so a required
/// child is judged only where the parent is present.
pub(crate) fn land_under(
    field: Arc<Field>,
    array: ArrayRef,
    parent: Option<&NullBuffer>,
    proof: &Proof,
) -> Result<Serie> {
    column_of(Arc::clone(&field), Arc::clone(&array), parent, proof)
        .map_err(|refusal| located(&field, array.as_ref(), refusal))
}

/// Land one batch as the record column of `root`, resolved once by the
/// caller and never cloned per batch; the children share the batch's
/// columns.
///
/// A value a leaf refuses is named by the batch row it lies in and the path
/// below it: `$[3].bid.live[0].miccode`.
pub(crate) fn land_batch(root: &Resolved, batch: RecordBatch, proof: &Proof) -> Result<Serie> {
    let records = crate::cast::struct_array_from_batch(batch);
    crate::arrow::require_projection(&root.field, records.as_ref())?;
    child_of(
        Arc::clone(&root.field),
        Arc::clone(&records),
        None,
        proof,
        &mut crate::budget::MaterializationBudget::default(),
        Some(root),
    )
    .map_err(|refusal| located(&root.field, records.as_ref(), refusal))
}

/// [`land_batch`] for a root landed once: proved whole here, and boxed as
/// it goes rather than resolved for batches that never come.
fn land_once(root: Arc<Field>, batch: RecordBatch, proof: &Proof) -> Result<Serie> {
    let records = crate::cast::struct_array_from_batch(batch);
    column_of(Arc::clone(&root), Arc::clone(&records), None, proof)
        .map_err(|refusal| located(&root, records.as_ref(), refusal))
}

/// Restate a leaf's value refusal in the rows of the array that was landed.
///
/// The proof reads each leaf in its own rows, which a nested leaf does not
/// share with the record above it, so the refused cell is found again by
/// walking the landed rows down to it - once, on the way out. Any other
/// refusal already names what it refuses.
fn located(field: &Field, array: &dyn Array, refusal: Error) -> Error {
    if !matches!(refusal, Error::InvalidValue { .. }) {
        return refusal;
    }
    let root = crate::path::Path::root();
    (0..array.len())
        .find_map(|row| {
            let at = root.child(crate::path::Segment::Index(row));
            refused_cell(field, array, row, &at)
        })
        .map_or(refusal, |(path, reason)| {
            Error::Core(crate::Error::InvalidRecord {
                path: path.into(),
                reason: reason.into(),
            })
        })
}

/// The first cell below row `row` that `field` refuses, as the path to it
/// and its refusal: a record descends by child, a sequence by item, a map
/// by entry, and a leaf the landing reads is read.
fn refused_cell(
    field: &Field,
    array: &dyn Array,
    row: usize,
    path: &crate::path::Path<'_>,
) -> Option<(String, String)> {
    use arrow_array::cast::AsArray as _;
    use arrow_buffer::ArrowNativeType as _;

    if array.is_null(row) {
        return None;
    }
    let items = |values: &ArrayRef, range: std::ops::Range<usize>, item: &Field| {
        range.enumerate().find_map(|(index, at)| {
            let below = path.child(crate::path::Segment::Index(index));
            refused_cell(item, values.as_ref(), at, &below)
        })
    };
    match field.dtype() {
        DataType::Struct(fields) => {
            let records = array.as_struct_opt()?;
            fields
                .iter()
                .zip(records.columns())
                .find_map(|(child, column)| {
                    refused_cell(child, column.as_ref(), row, &path.field(child.name()))
                })
        }
        DataType::Serie(item) => {
            let list = array.as_list_opt::<i32>()?;
            items(
                list.values(),
                list.value_offsets()[row].as_usize()..list.value_offsets()[row + 1].as_usize(),
                item,
            )
        }
        DataType::LargeSerie(item) => {
            let list = array.as_list_opt::<i64>()?;
            items(
                list.values(),
                list.value_offsets()[row].as_usize()..list.value_offsets()[row + 1].as_usize(),
                item,
            )
        }
        DataType::SerieView(item) => {
            let list = array.as_list_view_opt::<i32>()?;
            let start = list.value_offsets()[row].as_usize();
            items(
                list.values(),
                start..start + list.value_sizes()[row].as_usize(),
                item,
            )
        }
        DataType::LargeSerieView(item) => {
            let list = array.as_list_view_opt::<i64>()?;
            let start = list.value_offsets()[row].as_usize();
            items(
                list.values(),
                start..start + list.value_sizes()[row].as_usize(),
                item,
            )
        }
        DataType::FixedSizeSerie(item, _) => {
            let list = array.as_fixed_size_list_opt()?;
            let start = list.value_offset(row).as_usize();
            items(
                list.values(),
                start..start + list.value_length().as_usize(),
                item,
            )
        }
        DataType::Map(_) | DataType::SortedMap(_) => {
            let map = array.as_map_opt()?;
            let entries: ArrayRef = Arc::new(map.entries().clone());
            let mapping = field.dtype().as_mapping()?;
            let range =
                map.value_offsets()[row].as_usize()..map.value_offsets()[row + 1].as_usize();
            items(&entries, range, mapping.entries())
        }
        dtype if reads_rows(dtype) => {
            let refusal = match crate::serie::value::value_from_array(dtype, array, row) {
                Ok(value) => field.scalar(value).err()?,
                Err(error) => crate::Error::from(error),
            };
            Some(match refusal {
                crate::Error::InvalidRecord {
                    path: below,
                    reason,
                } => (
                    format!(
                        "{}{}",
                        path.render(),
                        below.strip_prefix('$').unwrap_or(&below)
                    ),
                    reason.to_string(),
                ),
                other => (path.render(), other.to_string()),
            })
        }
        _ => None,
    }
}

/// Lay out rows that already went through the field's contract - through
/// [`Field::scalar`] or its canonicalization - and land them proven, so no
/// row is checked or read a second time.
pub(crate) fn from_canonical_rows(field: Arc<Field>, rows: &[&Scalar]) -> crate::Result<Serie> {
    let array = canonical_rows(&field, rows)?;
    Ok(column_of(field, array, None, &Proof::Proven)?)
}

/// Canonical rows laid out as the array they are, and nothing landed: the
/// transport a batch reader hands on takes the buffers as built, because
/// landing them would only prove again what [`Field::scalar`] proved when
/// the rows were canonicalized, and read no cell of them.
pub(crate) fn canonical_rows(field: &Field, rows: &[&Scalar]) -> crate::Result<ArrayRef> {
    Ok(crate::serie::value::array_of_rows(field, rows)?)
}

/// A field's canonical default - [`Field::default_value`] - as a one-row
/// array, for the cast engine that fills and places it.
pub(crate) fn default_array(field: &Field) -> Result<ArrayRef> {
    let value = field.default_value()?;
    crate::serie::value::array_of_rows(field, &[&value])
}

/// A bare datatype's canonical default - [`DataType::default_value`] - as a
/// one-row array laid out under a required field, for the cast engine: the
/// datatype planner is the authority, so a null-only default stays logically
/// null.
pub(crate) fn default_dtype_array(dtype: &DataType) -> Result<ArrayRef> {
    let field = Field::new("value", dtype.clone(), false);
    let value = dtype.default_value()?;
    crate::serie::value::array_of_rows(&field, &[&value])
}

/// The record root a non-record column crosses into a table under: the
/// column is its one child, named as it is.
pub(crate) fn record_root(field: &Field) -> crate::Result<Field> {
    Ok(
        DataType::from(crate::StructType::from_fields([field.clone()])?)
            .required_field(DEFAULT_ROOT_NAME),
    )
}

/// The schema a column of `field` crosses into a table under: a record's
/// own children, or the one column of a [`DEFAULT_ROOT_NAME`] root.
///
/// Decided by the field once, so every batch of a chunked column shares it.
pub(crate) fn batch_schema(field: &Field) -> Result<SchemaRef> {
    arrow_schema_from_field(&SerieReader::root_of(field)?)
}

/// One column's rows as the batch `schema` states, where `schema` is
/// [`batch_schema`]'s answer for the column's field.
///
/// A record column's children are the columns; any other column is the one
/// column. A batch states no row validity, so a record column holding an
/// absent row is refused rather than having that absence dropped.
pub(crate) fn batch_under(schema: &SchemaRef, serie: &Serie) -> Result<RecordBatch> {
    let field = serie.require_field()?;
    let array = serie.require_arrow_array()?;
    if field.dtype().as_fields().is_none() {
        return Ok(RecordBatch::try_new(Arc::clone(schema), vec![array])?);
    }
    let absent = array.null_count();
    if absent != 0 {
        return Err(Error::IncompatibleSchema(format!(
            "record column {:?} holds {absent} absent rows, which a table cannot state",
            field.name()
        )));
    }
    let Some(records) = array.as_any().downcast_ref::<StructArray>() else {
        return Err(Error::IncompatibleSchema(format!(
            "record column {:?} does not lay out as a struct array",
            field.name()
        )));
    };
    Ok(RecordBatch::try_new_with_options(
        Arc::clone(schema),
        records.columns().to_vec(),
        &RecordBatchOptions::new().with_row_count(Some(array.len())),
    )?)
}

impl Serie {
    /// Take one Arrow array as a column: of its own field, or cast into
    /// `field`.
    ///
    /// With no field the array is the column of its own layout, named
    /// `item` and nullable exactly where it holds an absent row. Value buffers
    /// are shared except where hidden narrow spans or noncompact views need
    /// rebuilding. Hidden narrow child slots become null placeholders, so a
    /// child also reads safely on its own. Visible rows of a leaf whose layout
    /// is not its whole contract are read once, because a bare array carries
    /// no evidence. With a field, an array already laid out as the field
    /// lands as it stands, proven exactly as the identity plan would prove it
    /// and with no plan compiled; any other layout - or an exact one the
    /// landing refuses, whose absent rows a required field repairs or whose
    /// values a safe cast nulls - compiles one [`ArrowCastPlan`] and is
    /// converted under `options`. A loop over many arrays holds one plan
    /// instead.
    ///
    /// # Errors
    ///
    /// Returns an error naming the column and the row for a value the field
    /// refuses (nulled instead under `safe`), an absent row a required field
    /// refuses under [`Nullability::Strict`](crate::Nullability::Strict), or
    /// a layout no column holds.
    pub fn from_arrow_array(
        field: Option<&Field>,
        array: ArrayRef,
        options: ArrowCastOptions,
    ) -> Result<Self> {
        let Some(field) = field else {
            let item = item_field(array.data_type(), array.logical_null_count() != 0)?;
            return land(Arc::new(item), array, &Proof::Unproven);
        };
        if lands_exactly(field, array.data_type())? {
            let exact = land(
                Arc::new(field.clone()),
                Arc::clone(&array),
                &Proof::Unproven,
            );
            if let Ok(serie) = exact {
                return Ok(serie);
            }
        }
        let source = Arc::new(ArrowField::new(
            field.name(),
            array.data_type().clone(),
            true,
        ));
        ArrowCastPlan::compile_arrow(&source, field, options, Deferred::default())?
            .cast_array(array)
    }

    /// The empty column of `field`.
    ///
    /// # Errors
    ///
    /// Returns an error when `field` has no Arrow projection or a layout
    /// this crate keeps no column for.
    pub fn empty(field: impl Into<Arc<Field>>) -> crate::Result<Self> {
        let field = field.into();
        let storage = empty_storage(&field)?;
        Ok(land(field, new_empty_array(&storage), &Proof::Proven)?)
    }

    /// The empty column of `field` whose buffers reserve `rows`.
    ///
    /// Arrow's builders reserve for the layout: values, offsets and validity
    /// for a leaf, children and offsets for a nested layout, at the same
    /// capacity. An encoding reserves nothing and is the empty column.
    ///
    /// # Errors
    ///
    /// [`Self::empty`] carries the rule.
    pub fn with_capacity(field: impl Into<Arc<Field>>, rows: usize) -> crate::Result<Self> {
        let field = field.into();
        let storage = empty_storage(&field)?;
        let array = match storage {
            arrow_schema::DataType::Dictionary(..)
            | arrow_schema::DataType::RunEndEncoded(..)
            | arrow_schema::DataType::Union(..) => new_empty_array(&storage),
            _ => make_builder(&storage, rows).finish(),
        };
        Ok(land(field, array, &Proof::Proven)?)
    }

    /// Build a column from the rows a field types.
    ///
    /// Each row goes through [`Field::scalar`], the crate's one value
    /// contract, the rows are laid out once at the crate's one scalar-array
    /// boundary, and the array takes the door with its rows already proven,
    /// so no row is read a second time.
    ///
    /// # Errors
    ///
    /// Returns an error naming the field when a row is not one it accepts.
    pub fn from_scalars(
        field: impl Into<Arc<Field>>,
        rows: impl IntoIterator<Item = Scalar>,
    ) -> crate::Result<Self> {
        let field = field.into();
        let proven = rows
            .into_iter()
            .map(|row| field.scalar(row))
            .collect::<crate::Result<Vec<Scalar>>>()?;
        let borrowed: Vec<&Scalar> = proven.iter().collect();
        from_canonical_rows(field, &borrowed)
    }

    /// `rows` copies of `field`'s canonical default -
    /// [`Field::default_value`] - laid out once and repeated by index.
    ///
    /// # Errors
    ///
    /// Returns an error when the field has no default - a required `null`
    /// field has none - or no Arrow projection.
    pub fn from_default(field: impl Into<Arc<Field>>, rows: usize) -> Result<Self> {
        let field = field.into();
        let default = field.default_value()?;
        from_canonical_rows(field, &[&default])?.repeat(0, rows)
    }

    /// Row `row` of this column, `len` times.
    ///
    /// The selection is Arrow's `take` over one repeated index, so the rows
    /// are laid out once and never proven again: they are rows this column
    /// already holds.
    ///
    /// # Errors
    ///
    /// Returns an error naming the column when `row` is past the end.
    pub(crate) fn repeat(&self, row: usize, len: usize) -> Result<Self> {
        let Some(field) = self.field_ref() else {
            let value = self.scalar(row)?;
            return Ok(Self::new(vec![value; len]));
        };
        if row >= self.len() {
            return Err(Error::IncompatibleSchema(format!(
                "column {:?} has {} rows, so row {row} cannot repeat",
                field.name(),
                self.len()
            )));
        }
        let index = u32::try_from(row).map_err(|_| {
            Error::IncompatibleSchema(format!(
                "column {:?} row {row} is past what one take can index",
                field.name()
            ))
        })?;
        let array = self.require_arrow_array()?;
        let indices = arrow_array::UInt32Array::from(vec![index; len]);
        let repeated = arrow_select::take::take(array.as_ref(), &indices, None)?;
        if repeated.len() != len {
            // Arrow's take answers a zero-width fixed-size list by its child,
            // which has no rows to count, so the row is laid out instead.
            let value = self.scalar(row)?;
            return Ok(from_canonical_rows(Arc::clone(field), &vec![&value; len])?);
        }
        land(Arc::clone(field), repeated, &Proof::Proven)
    }

    /// Read one Arrow table as the record column of its rows: of the
    /// batch's own schema, or cast into `root`.
    ///
    /// With no root the batch's schema is imported once as a
    /// [`DEFAULT_ROOT_NAME`] record, because Arrow names columns and never
    /// the record, and the batch's columns become this column's children,
    /// shared rather than copied. With a root - a non-null record - one plan
    /// casts the batch into it, exactly as [`Self::from_arrow_array`] casts
    /// a column. A batch has no row validity, so every record row is present.
    ///
    /// # Errors
    ///
    /// Returns an error when the schema does not project to a record, when
    /// `root` is not a bounded non-null record, or [`Self::from_arrow_array`]'s
    /// refusal for a column.
    pub fn from_arrow_batch(
        root: Option<&Field>,
        batch: &RecordBatch,
        options: ArrowCastOptions,
    ) -> Result<Self> {
        let Some(root) = root else {
            let root = field_from_arrow_schema(DEFAULT_ROOT_NAME, batch.schema().as_ref())?;
            return land_once(Arc::new(root), batch.clone(), &Proof::Unproven);
        };
        // An exact batch lands as it stands, proven as the identity plan would
        // prove it and with no plan compiled; one the landing refuses - an
        // absent row under a required child, a value a narrower leaf will
        // not take - goes to the plan, which repairs or refuses it under
        // `options` exactly as before.
        if !root.is_nullable()
            && root.dtype().as_fields().is_some()
            && lands_exactly(
                root,
                &ArrowDataType::Struct(batch.schema_ref().fields().clone()),
            )?
        {
            root.validate_bounded()?;
            let resolved = Resolved::of(Arc::new(root.clone()));
            let records = crate::cast::struct_array_from_batch(batch.clone());
            if let Ok(serie) = land_planned(&resolved, records, &Proof::Unproven) {
                return Ok(serie);
            }
        }
        ArrowCastPlan::compile_schema(batch.schema_ref(), root, options, Deferred::default())?
            .cast_batch(batch.clone())
    }

    /// Drain one Arrow batch stream into the record column of its rows.
    ///
    /// This is [`ChunkedSerie::from_arrow_reader`](crate::ChunkedSerie::from_arrow_reader)
    /// joined by [`ChunkedSerie::into_serie`](crate::ChunkedSerie::into_serie):
    /// every batch is cast by the reader's one plan, and the landed columns
    /// are joined once. The whole stream is held - a column is one
    /// contiguous set of buffers, so the bound is the stream itself.
    ///
    /// # Errors
    ///
    /// [`SerieReader::from_arrow_reader`] carries the rule, the reader
    /// carries its own, and the join refuses what
    /// [`ChunkedSerie::into_serie`](crate::ChunkedSerie::into_serie) does.
    pub fn from_arrow_reader(
        root: Option<&Field>,
        reader: BatchReader,
        options: ArrowCastOptions,
    ) -> Result<Self> {
        crate::ChunkedSerie::from_arrow_reader(root, reader, options)?.into_serie()
    }

    /// This column under `target`: cast once, for a column in hand.
    ///
    /// A column already under `target` is itself. An equal layout under
    /// another name or nullability is the same buffers relabelled, with
    /// absence judged again. Anything else compiles one [`ArrowCastPlan`]
    /// and applies it; a loop casting many columns holds that plan instead,
    /// because this compiles on every call.
    ///
    /// # Errors
    ///
    /// Returns an error for a run, which has no layout for a plan to read -
    /// [`Self::from_scalars`] is the door that types a run's rows - and the
    /// plan's refusals.
    pub fn cast(&self, target: &Field, options: ArrowCastOptions) -> Result<Self> {
        let field = self.require_field()?;
        if field == target {
            return Ok(self.clone());
        }
        ArrowCastPlan::compile(field, target, options)?.apply(self)
    }

    /// Every row of this column as one Arrow table.
    ///
    /// A record column's children are the columns. Any other column is the
    /// one column of a [`DEFAULT_ROOT_NAME`] root, named as it is. A batch
    /// states no row validity, so a record column holding an absent row is
    /// refused rather than having that absence dropped.
    ///
    /// # Errors
    ///
    /// Returns an error for a run, which names no layout, and for a record
    /// column holding an absent row.
    pub fn into_arrow_batch(&self) -> Result<RecordBatch> {
        let schema = batch_schema(self.require_field()?)?;
        batch_under(&schema, self)
    }

    /// This column's one row as Arrow's scalar datum, sharing its buffers.
    ///
    /// # Errors
    ///
    /// Returns an error for a run, and for any length but one, naming it.
    pub fn into_arrow_scalar(&self) -> Result<arrow_array::Scalar<ArrayRef>> {
        let array = self.require_arrow_array()?;
        if array.len() != 1 {
            return Err(Error::IncompatibleSchema(format!(
                "an Arrow scalar is exactly one row, got {}",
                array.len()
            )));
        }
        Ok(arrow_array::Scalar::new(array))
    }

    /// Stream this column's rows as one Arrow batch reader, of one batch.
    ///
    /// # Errors
    ///
    /// [`Self::into_arrow_batch`] carries the rule.
    pub fn into_arrow_reader(&self) -> Result<BatchReader> {
        let batch = self.into_arrow_batch()?;
        Ok(batch_reader(batch.schema(), [batch]))
    }
}

/// One record [`Serie`] per batch of an Arrow stream, each cast by one plan.
///
/// The plan is compiled once from the stream's schema, before a batch is
/// pulled, so a planning failure is raised by the constructor and every
/// batch differs only in the masks, offsets and dictionary keys it carries.
/// At most one source batch is held. After the first failure the inner
/// reader is dropped - which releases a C stream at the point an early close
/// would - and the reader is fused.
pub struct SerieReader {
    inner: Option<Source>,
    root: Arc<Field>,
    schema: SchemaRef,
}

/// What a [`SerieReader`] yields its columns from.
enum Source {
    /// A stream: each batch cast by the plan the stream was opened under as
    /// it arrives, then - where the reader was [cast](SerieReader::cast)
    /// again - by the plan that cast landed. A held source compiles no
    /// plan at all, so the plans live here and not on the reader.
    Stream(BatchReader, Box<ArrowCastPlan>, Option<Box<ArrowCastPlan>>),
    /// Record columns already held, each yielded as it stands.
    Held(std::vec::IntoIter<Serie>),
}

/// The record root a held column of `field` streams under, shared, and
/// proved bounded once: a column at the nesting ceiling wrapped one level
/// deeper is refused here, where no plan compiles to refuse it.
fn held_root(field: &Field) -> crate::Result<Arc<Field>> {
    let root = SerieReader::root_of(field)?;
    root.validate_bounded()?;
    Ok(Arc::new(root))
}

/// One held column as the record column it streams as under `root`,
/// nothing cast, copied or read.
fn held_record(root: &Arc<Field>, serie: Serie) -> Result<Serie> {
    let rows = serie.len();
    let record = match serie.as_struct() {
        Some(records) => {
            let absent = crate::SerieValue::null_count(records);
            if absent != 0 {
                return Err(Error::IncompatibleSchema(format!(
                    "record column {:?} holds {absent} absent rows, which a table cannot state",
                    serie.require_field()?.name()
                )));
            }
            structure::StructSerie::new(Arc::clone(root), records.children().to_vec(), None, rows)
        }
        None => structure::StructSerie::new(Arc::clone(root), vec![serie], None, rows),
    };
    Ok(crate::SerieValue::into_serie(record))
}

impl SerieReader {
    /// Read `reader`'s batches as record columns: of its own schema, or cast
    /// into `root`, a bounded non-null record.
    ///
    /// # Errors
    ///
    /// Returns an error when the schema does not project to a record, when
    /// `root` is not a bounded non-null record, or when the cast cannot be
    /// planned from the reader's schema.
    pub fn from_arrow_reader(
        root: Option<&Field>,
        reader: BatchReader,
        options: ArrowCastOptions,
    ) -> Result<Self> {
        let schema = reader.schema();
        let imported;
        let root = match root {
            Some(root) => root,
            None => {
                imported = field_from_arrow_schema(DEFAULT_ROOT_NAME, schema.as_ref())?;
                &imported
            }
        };
        let plan =
            ArrowCastPlan::compile_schema(schema.as_ref(), root, options, Deferred::default())?;
        let schema = Arc::clone(plan.target_schema()?);
        Ok(Self {
            inner: Some(Source::Stream(reader, Box::new(plan), None)),
            root: Arc::new(root.clone()),
            schema,
        })
    }

    /// Read one held column as a stream of one record column.
    ///
    /// A record column is the one batch it is; any other column is the one
    /// child of a [`DEFAULT_ROOT_NAME`] record, named as it is - the rule
    /// [`Serie::into_arrow_batch`] states. Nothing is cast, copied or read,
    /// and no plan is compiled: the column is yielded as it stands.
    ///
    /// # Errors
    ///
    /// Returns an error for a run, which names no layout, and for a record
    /// column holding an absent row, which a table cannot state.
    pub fn from_serie(serie: Serie) -> Result<Self> {
        let root = held_root(serie.require_field()?)?;
        let record = held_record(&root, serie)?;
        Self::held(root, vec![record])
    }

    /// Read a held chunked column as the stream of its chunks, one record
    /// column per chunk.
    ///
    /// A record's chunks are the batches they are; any other field's chunks
    /// are each the one child of a [`DEFAULT_ROOT_NAME`] record, named as
    /// it is - the rule [`Self::from_serie`] states, applied per chunk.
    /// Nothing is cast, copied or read, and a chunked serie of no chunks is
    /// the empty stream of its root.
    ///
    /// # Errors
    ///
    /// Returns an error for a record chunk holding an absent row, which a
    /// table cannot state.
    pub fn from_chunked(chunked: crate::ChunkedSerie) -> Result<Self> {
        let root = held_root(chunked.field())?;
        let mut records = Vec::with_capacity(chunked.num_chunks());
        for chunk in chunked.chunks() {
            records.push(held_record(&root, chunk.clone())?);
        }
        Self::held(root, records)
    }

    /// The stream of `records`, each already a record column under `root`.
    fn held(root: Arc<Field>, records: Vec<Serie>) -> Result<Self> {
        let schema = arrow_schema_from_field(&root)?;
        Ok(Self {
            inner: Some(Source::Held(records.into_iter())),
            root,
            schema,
        })
    }

    /// Every record this reader yields, cast into `target` - a non-null
    /// record root, or a column named as the one child of one, the rule
    /// [`Self::root_of`] states - under one plan.
    ///
    /// A target that is this reader's own root hands the reader back as it
    /// stands. Held records are cast here, once each. A stream's batches
    /// are cast by the plan the stream was opened under and then landed
    /// under `target`, so what the first plan repaired or nulled is what the
    /// second reads - two plans in sequence, never one that skips the
    /// middle - except over an identity first plan, where the one plan from
    /// the stream's own schema says exactly the same.
    ///
    /// # Errors
    ///
    /// Returns an error when `target` is not a bounded record root the
    /// reader's root can be planned into, and, for held records, the first
    /// record a value or an absent row refuses, naming the column.
    pub fn cast(self, target: &Field, options: ArrowCastOptions) -> Result<Self> {
        let target = Self::root_of(target)?;
        if target == *self.root {
            return Ok(self);
        }
        let plan = ArrowCastPlan::compile(&self.root, &target, options)?;
        let root = Arc::new(target);
        let schema = arrow_schema_from_field(&root)?;
        let inner = match self.inner {
            Some(Source::Held(records)) => {
                let cast = records
                    .map(|record| plan.apply(&record))
                    .collect::<Result<Vec<_>>>()?;
                Some(Source::Held(cast.into_iter()))
            }
            Some(Source::Stream(reader, first, None))
                if first.is_identity() && first.source_schema().is_some() =>
            {
                let source = first
                    .source_schema()
                    .expect("the identity plan was compiled from a schema");
                let direct =
                    ArrowCastPlan::compile_schema(source, &root, options, Deferred::default())?;
                Some(Source::Stream(reader, Box::new(direct), None))
            }
            Some(Source::Stream(reader, first, then)) => {
                // A second cast lands what the first two cast, in order.
                let then = match then {
                    Some(then) => ArrowCastPlan::compile(then.as_target(), &root, options)?,
                    None => plan,
                };
                Some(Source::Stream(reader, first, Some(Box::new(then))))
            }
            None => None,
        };
        Ok(Self {
            inner,
            root,
            schema,
        })
    }

    /// The record every yielded column is typed by.
    pub fn field(&self) -> &Field {
        &self.root
    }

    /// The record root a held column of `field` crosses into a table under:
    /// a record's own field, required, or the [`DEFAULT_ROOT_NAME`] record it
    /// is the one child of, named as it is.
    ///
    /// This is the one rule [`Self::from_serie`] and [`Self::from_chunked`]
    /// name their [`Self::field`] by, and [`Serie::into_arrow_batch`],
    /// [`Serie::into_arrow_reader`] and
    /// [`ChunkedSerie::into_arrow_reader`](crate::ChunkedSerie::into_arrow_reader)
    /// project their schema from, so a binding that names a reader's root
    /// reads it here rather than restating it.
    ///
    /// # Errors
    ///
    /// Returns an error when a record of the one child cannot be declared -
    /// a field a record refuses as a child.
    pub fn root_of(field: &Field) -> crate::Result<Field> {
        if field.dtype().as_fields().is_some() {
            Ok(field.clone().with_nullable(false))
        } else {
            record_root(field)
        }
    }

    /// The stream's transport face: its batches reconciled to the root and
    /// never landed, so no row is proven beyond what the cast itself reads.
    ///
    /// Over an identity plan it is the inner reader, handed back untouched.
    pub fn into_arrow_reader(self) -> BatchReader {
        match self.inner {
            Some(Source::Stream(inner, plan, None)) if plan.is_identity() => inner,
            Some(Source::Stream(inner, plan, then)) => Box::new(Reconciled {
                inner: Some(inner),
                plan: *plan,
                then: then.map(|then| *then),
                schema: self.schema,
            }),
            Some(Source::Held(records)) => {
                let schema = Arc::clone(&self.schema);
                let batches = records.map(move |record| {
                    batch_under(&schema, &record)
                        .map_err(|error| ArrowError::ExternalError(Box::new(error)))
                });
                Box::new(RecordBatchIterator::new(batches, self.schema))
            }
            None => batch_reader(self.schema, []),
        }
    }
}

impl Iterator for SerieReader {
    type Item = Result<Serie>;

    fn next(&mut self) -> Option<Self::Item> {
        let (reader, plan, then) = match self.inner.as_mut()? {
            Source::Stream(reader, plan, then) => (reader, &**plan, then.as_deref()),
            Source::Held(records) => {
                let next = records.next();
                if next.is_none() {
                    self.inner = None;
                }
                return next.map(Ok);
            }
        };
        let pulled = reader.next();
        let landed = match pulled {
            Some(Ok(batch)) => plan.cast_batch(batch).and_then(|landed| match then {
                Some(then) => then.apply(&landed),
                None => Ok(landed),
            }),
            Some(Err(error)) => Err(from_reader_error(error)),
            None => {
                self.inner = None;
                return None;
            }
        };
        if landed.is_err() {
            self.inner = None;
        }
        Some(landed)
    }
}

impl std::iter::FusedIterator for SerieReader {}

impl std::fmt::Debug for SerieReader {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SerieReader")
            .field("field", &self.root)
            .field("done", &self.inner.is_none())
            .finish()
    }
}

/// A stream's batches reconciled to one root as they arrive: transport,
/// which makes no column claim and reads no row the cast does not.
struct Reconciled {
    inner: Option<BatchReader>,
    plan: ArrowCastPlan,
    /// The plan a later [`SerieReader::cast`] landed the first's output
    /// under, applied as transport after it.
    then: Option<ArrowCastPlan>,
    schema: SchemaRef,
}

impl Iterator for Reconciled {
    type Item = std::result::Result<RecordBatch, ArrowError>;

    fn next(&mut self) -> Option<Self::Item> {
        let batch = match self.inner.as_mut()?.next() {
            Some(Ok(batch)) => batch,
            other => {
                self.inner = None;
                return other;
            }
        };
        let cast = self
            .plan
            .reconcile_batch(batch)
            .and_then(|cast| match &self.then {
                Some(then) => then.reconcile_batch(cast),
                None => Ok(cast),
            });
        match cast {
            Ok(cast) => Some(Ok(cast)),
            Err(error) => {
                self.inner = None;
                Some(Err(ArrowError::ExternalError(Box::new(error))))
            }
        }
    }
}

impl RecordBatchReader for Reconciled {
    fn schema(&self) -> SchemaRef {
        Arc::clone(&self.schema)
    }
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/allocations.rs` pins and a caller cannot reach.

    use std::sync::Arc;

    use crate::{Field, Scalar, Serie};

    /// Lay rows the field's contract already canonicalized out, and land
    /// them proven.
    ///
    /// # Errors
    ///
    /// Returns the layout's refusal.
    pub fn from_canonical_rows(field: Arc<Field>, rows: &[&Scalar]) -> crate::Result<Serie> {
        super::from_canonical_rows(field, rows)
    }
}
