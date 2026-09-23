//! The one door buffers take into a column, and the one they take out.
//!
//! A field says which leaf its buffers are, so this is where that reading
//! happens and nowhere else: the layout is proven against the field's own
//! Arrow projection, absence is judged on the validity words, the values are
//! proven by the layout where the layout is the contract and read once
//! where it is not, the array is taken as it is - no buffer copied - and a
//! nested layout recurses into the columns under it with each child field.
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
    Serie, boolean, bytes, enums, list, mapping, null, primitive, runend, structure, union, variant,
};
use arrow_schema::{ArrowError, Field as ArrowField, SchemaRef};

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
        DataType::List(item)
        | DataType::ListView(item)
        | DataType::FixedSizeList(item, _)
        | DataType::LargeList(item)
        | DataType::LargeListView(item) => lands_as_is(item.dtype()),
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
    array
        .as_any()
        .downcast_ref::<A>()
        .cloned()
        .ok_or(Error::Internal {
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

/// One module's door: the column its layout is, or `None` for another's.
type ColumnOf = fn(Arc<Field>, ArrayRef, Option<&NullBuffer>, &Proof) -> Result<Option<Serie>>;

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
    child_of(field, array, parent, proof)
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
) -> Result<Serie> {
    require_present(&field, array.as_ref(), parent)?;
    let proves = proof.reads_at_landing() && reads_rows(field.dtype());
    let modules: [ColumnOf; 11] = [
        null::column_of,
        boolean::column_of,
        primitive::column_of,
        bytes::column_of,
        variant::column_of,
        structure::column_of,
        list::column_of,
        mapping::column_of,
        union::column_of,
        enums::column_of,
        runend::column_of,
    ];
    for module in modules {
        if let Some(serie) = module(Arc::clone(&field), Arc::clone(&array), parent, proof)? {
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
    child_of(Arc::clone(&field), Arc::clone(&array), None, proof)
        .map_err(|refusal| located(&field, array.as_ref(), refusal))
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
pub(crate) fn land_batch(root: &Arc<Field>, batch: &RecordBatch, proof: &Proof) -> Result<Serie> {
    let records = crate::cast::struct_array_from_batch(batch.clone());
    column_of(Arc::clone(root), Arc::clone(&records), None, proof)
        .map_err(|refusal| located(root, records.as_ref(), refusal))
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
        DataType::List(item) => {
            let list = array.as_list_opt::<i32>()?;
            items(
                list.values(),
                list.value_offsets()[row].as_usize()..list.value_offsets()[row + 1].as_usize(),
                item,
            )
        }
        DataType::LargeList(item) => {
            let list = array.as_list_opt::<i64>()?;
            items(
                list.values(),
                list.value_offsets()[row].as_usize()..list.value_offsets()[row + 1].as_usize(),
                item,
            )
        }
        DataType::FixedSizeList(item, _) => {
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
    canonical_rows(field, rows).map(|(serie, _)| serie)
}

/// [`from_canonical_rows`], answering the buffers as they were laid out
/// beside the landed column: a caller handing the buffers on - a batch
/// reader - takes them as built rather than reassembling them from the
/// column, which would validate every level of a nested layout again.
pub(crate) fn canonical_rows(
    field: Arc<Field>,
    rows: &[&Scalar],
) -> crate::Result<(Serie, ArrayRef)> {
    let array = crate::serie::value::array_of_rows(&field, rows)?;
    let serie = column_of(field, Arc::clone(&array), None, &Proof::Proven)?;
    Ok((serie, array))
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

impl Serie {
    /// Take one Arrow array as a column: of its own field, or cast into
    /// `field`.
    ///
    /// With no field the array is the column of its own layout, named
    /// `item` and nullable exactly where it holds an absent row; nothing is
    /// copied, and each row of a leaf whose layout is not its datatype's
    /// whole contract is read once to prove it, because a bare array carries
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
            let dtype = DataType::from_arrow_datatype(array.data_type())?;
            let nullable = array.null_count() != 0;
            return land(
                Arc::new(Field::new("item", dtype, nullable)),
                array,
                &Proof::Unproven,
            );
        };
        if lands_as_is(field.dtype())
            && field.as_arrow_field_ref()?.data_type() == array.data_type()
        {
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
        let storage = field.as_arrow_field_ref()?.data_type().clone();
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
        let storage = field.as_arrow_field_ref()?.data_type().clone();
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
            return land_batch(&Arc::new(root), batch, &Proof::Unproven);
        };
        ArrowCastPlan::compile_schema(batch.schema_ref(), root, options, Deferred::default())?
            .cast_batch(batch)
    }

    /// Drain one Arrow batch stream into the record column of its rows.
    ///
    /// This is [`SerieReader::from_arrow_reader`] collected: every batch is
    /// cast by the reader's one plan, and the landed columns are joined once.
    /// The whole stream is held - a column is one contiguous set of buffers,
    /// so the bound is the stream itself.
    ///
    /// # Errors
    ///
    /// [`SerieReader::from_arrow_reader`] carries the rule, and the reader
    /// carries its own.
    pub fn from_arrow_reader(
        root: Option<&Field>,
        reader: BatchReader,
        options: ArrowCastOptions,
    ) -> Result<Self> {
        let series = SerieReader::from_arrow_reader(root, reader, options)?;
        let root = Arc::clone(&series.root);
        let landed = series.collect::<Result<Vec<Self>>>()?;
        let arrays = landed
            .iter()
            .map(Self::require_arrow_array)
            .collect::<crate::Result<Vec<ArrayRef>>>()?;
        if arrays.is_empty() {
            return Ok(Self::empty(root)?);
        }
        let borrowed: Vec<&dyn Array> = arrays.iter().map(AsRef::as_ref).collect();
        let joined = arrow_select::concat::concat(&borrowed)?;
        land(root, joined, &Proof::Proven)
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
        let field = self.require_field()?;
        let array = self.require_arrow_array()?;
        let Some(records) = array.as_any().downcast_ref::<StructArray>() else {
            let root = record_root(field)?;
            let schema = arrow_schema_from_field(&root)?;
            return Ok(RecordBatch::try_new(schema, vec![array])?);
        };
        let absent = records.null_count();
        if absent != 0 {
            return Err(Error::IncompatibleSchema(format!(
                "record column {:?} holds {absent} absent rows, which a table cannot state",
                field.name()
            )));
        }
        let schema = arrow_schema_from_field(&field.clone().with_nullable(false))?;
        Ok(RecordBatch::try_new_with_options(
            schema,
            records.columns().to_vec(),
            &RecordBatchOptions::new().with_row_count(Some(array.len())),
        )?)
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
    plan: ArrowCastPlan,
    root: Arc<Field>,
    schema: SchemaRef,
}

/// What a [`SerieReader`] yields its columns from.
enum Source {
    /// A stream, each batch cast by the reader's plan as it arrives.
    Stream(BatchReader),
    /// One column already held, yielded as it stands.
    Held(Serie),
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
            inner: Some(Source::Stream(reader)),
            root: Arc::new(root.clone()),
            plan,
            schema,
        })
    }

    /// Read one held column as a stream of one record column.
    ///
    /// A record column is the one batch it is; any other column is the one
    /// child of a [`DEFAULT_ROOT_NAME`] record, named as it is - the rule
    /// [`Serie::into_arrow_batch`] states. Nothing is cast, copied or read:
    /// the plan is the identity, and the column is yielded as it stands.
    ///
    /// # Errors
    ///
    /// Returns an error for a run, which names no layout, and for a record
    /// column holding an absent row, which a table cannot state.
    pub fn from_serie(serie: Serie) -> Result<Self> {
        let field = serie.require_field()?;
        let rows = serie.len();
        let (root, record) = match serie.as_struct() {
            Some(records) => {
                let absent = crate::SerieValue::null_count(records);
                if absent != 0 {
                    return Err(Error::IncompatibleSchema(format!(
                        "record column {:?} holds {absent} absent rows, which a table cannot state",
                        field.name()
                    )));
                }
                let root = Arc::new(field.clone().with_nullable(false));
                let children = records.children().to_vec();
                (
                    Arc::clone(&root),
                    structure::StructSerie::new(root, children, None, rows),
                )
            }
            None => {
                let root = Arc::new(record_root(field)?);
                (
                    Arc::clone(&root),
                    structure::StructSerie::new(root, vec![serie], None, rows),
                )
            }
        };
        let plan = ArrowCastPlan::compile(&root, &root, ArrowCastOptions::default())?;
        let schema = arrow_schema_from_field(&root)?;
        Ok(Self {
            inner: Some(Source::Held(crate::SerieValue::into_serie(record))),
            root,
            plan,
            schema,
        })
    }

    /// The record every yielded column is typed by.
    pub fn field(&self) -> &Field {
        &self.root
    }

    /// The stream's transport face: its batches reconciled to the root and
    /// never landed, so no row is proven beyond what the cast itself reads.
    ///
    /// Over an identity plan it is the inner reader, handed back untouched.
    pub fn into_arrow_reader(self) -> BatchReader {
        match self.inner {
            Some(Source::Stream(inner)) if self.plan.is_identity() => inner,
            Some(Source::Stream(inner)) => Box::new(Reconciled {
                inner: Some(inner),
                plan: self.plan,
                schema: self.schema,
            }),
            Some(Source::Held(serie)) => {
                let batch = serie
                    .into_arrow_batch()
                    .map_err(|error| ArrowError::ExternalError(Box::new(error)));
                Box::new(RecordBatchIterator::new(
                    std::iter::once(batch),
                    self.schema,
                ))
            }
            None => batch_reader(self.schema, []),
        }
    }
}

impl Iterator for SerieReader {
    type Item = Result<Serie>;

    fn next(&mut self) -> Option<Self::Item> {
        let reader = match self.inner.as_mut()? {
            Source::Stream(reader) => reader,
            Source::Held(_) => {
                let Some(Source::Held(serie)) = self.inner.take() else {
                    return None;
                };
                return Some(Ok(serie));
            }
        };
        let pulled = reader.next();
        let landed = match pulled {
            Some(Ok(batch)) => self.plan.cast_batch(&batch),
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
        match self.plan.reconcile_batch(batch) {
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
