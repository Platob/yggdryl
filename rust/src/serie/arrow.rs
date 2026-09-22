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
    Array, ArrayRef, OffsetSizeTrait, RecordBatch, RecordBatchOptions, RecordBatchReader,
    StructArray, new_empty_array,
};
use arrow_buffer::{NullBuffer, OffsetBuffer};

use super::{
    Serie, boolean, bytes, enums, mapping, null, primitive, runend, sequence, structure, union,
    variant,
};
use crate::arrow::{
    BatchReader, Error, Result, arrow_schema_from_field, batch_reader, field_from_arrow_schema,
};
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
        DataType::Enum(_) | DataType::RunEndEncoded(_) | DataType::Union(..) => {
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
fn require_present(field: &Field, array: &dyn Array, parent: Option<&NullBuffer>) -> Result<()> {
    if field.is_nullable() {
        return Ok(());
    }
    let absent = absent_rows(field.dtype(), array, parent);
    if absent == 0 {
        return Ok(());
    }
    Err(Error::IncompatibleSchema(format!(
        "column {:?} is not null, got {absent} absent of {} rows",
        field.name(),
        array.len(),
    )))
}

/// Whether the door reads this level's rows to prove them.
///
/// A leaf whose layout is its contract is proven by the layout; a nested
/// layout is proven through its children; an encoding is proven through its
/// values column. What remains is a leaf whose datatype is narrower than its
/// storage, and it is read once here.
fn reads_rows(dtype: &DataType) -> bool {
    !dtype.layout_is_contract() && dtype.field_len() == 0 && !matches!(dtype, DataType::Enum(_))
}

/// Read every row this level holds once, refusing the first one the field's
/// own contract refuses, naming the column and the row.
///
/// A row hidden under an absent parent record is not read: Arrow leaves its
/// slot unspecified.
fn prove_rows(field: &Field, array: &dyn Array, parent: Option<&NullBuffer>) -> Result<()> {
    let refused = |index: usize, refusal: &dyn std::fmt::Display| {
        Error::IncompatibleSchema(format!(
            "column {:?} row {index} is not one its field accepts: {refusal}",
            field.name()
        ))
    };
    for index in 0..array.len() {
        if parent.is_some_and(|above| above.len() == array.len() && above.is_null(index)) {
            continue;
        }
        let value = crate::arrow::value::value_from_array(field.dtype(), array, index)
            .map_err(|refusal| refused(index, &refusal))?;
        field
            .scalar(value)
            .map_err(|refusal| refused(index, &refusal))?;
    }
    Ok(())
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

/// One module's door: the column its layout is, or `None` for another's.
type ColumnOf = fn(Arc<Field>, ArrayRef, Option<&NullBuffer>, bool) -> Result<Option<Serie>>;

/// Build the column `field` types out of buffers that already hold it.
///
/// Every level proves itself - the projection, the absence, the values -
/// so a child whose buffers disagree with the child field is refused where
/// it lies rather than at the row that reads it. `proven` says the rows went
/// through the field's contract already (the crate laid them out), so no row
/// is read. Each module answers `None` for a layout that is not its own,
/// tried in the order the families are listed; the variant pair is tried
/// before the record module because it projects to a struct.
pub(crate) fn column_of(
    field: Arc<Field>,
    array: ArrayRef,
    parent: Option<&NullBuffer>,
    proven: bool,
) -> Result<Serie> {
    field.dtype().validate_bounded()?;
    crate::arrow::require_projection(&field, array.as_ref())?;
    require_present(&field, array.as_ref(), parent)?;
    if !proven && reads_rows(field.dtype()) {
        prove_rows(&field, array.as_ref(), parent)?;
    }
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
        if let Some(serie) = module(Arc::clone(&field), Arc::clone(&array), parent, proven)? {
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

impl Serie {
    /// Take one Arrow array as the column of `field`.
    ///
    /// Nothing is copied: the layout is proven against the field's own Arrow
    /// projection, absence against its nullability on the validity words,
    /// and the values by the layout where the layout is the datatype's whole
    /// contract - every integer, float, boolean, plain UTF-8 or byte leaf,
    /// and every nesting of them - or by one read of each row where it is
    /// not. A sliced list's offsets are rebased onto the items they reach.
    ///
    /// # Errors
    ///
    /// Returns an error naming the column when the array does not hold
    /// `field`'s exact physical layout, holds an absent row under a required
    /// field, holds a value the field's contract refuses, or holds a layout
    /// this crate keeps no column for.
    pub fn from_arrow_array(field: Field, array: ArrayRef) -> Result<Self> {
        column_of(Arc::new(field), array, None, false)
    }

    /// The empty column of `field`.
    ///
    /// # Errors
    ///
    /// Returns an error when `field` has no Arrow projection or a layout
    /// this crate keeps no column for.
    pub fn empty(field: Field) -> crate::Result<Self> {
        let storage = field.as_arrow_field_ref()?.data_type().clone();
        Ok(Self::from_arrow_array(field, new_empty_array(&storage))?)
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
    pub fn with_capacity(field: Field, rows: usize) -> crate::Result<Self> {
        let storage = field.as_arrow_field_ref()?.data_type().clone();
        let array = match storage {
            arrow_schema::DataType::Dictionary(..)
            | arrow_schema::DataType::RunEndEncoded(..)
            | arrow_schema::DataType::Union(..) => new_empty_array(&storage),
            _ => make_builder(&storage, rows).finish(),
        };
        Ok(Self::from_arrow_array(field, array)?)
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
        field: Field,
        rows: impl IntoIterator<Item = Scalar>,
    ) -> crate::Result<Self> {
        let proven = rows
            .into_iter()
            .map(|row| field.scalar(row))
            .collect::<crate::Result<Vec<Scalar>>>()?;
        let borrowed: Vec<&Scalar> = proven.iter().collect();
        let array = crate::arrow::value::array_from_values(&field, &borrowed)?;
        Ok(column_of(Arc::new(field), array, None, true)?)
    }

    /// Read one Arrow table as the column of its rows.
    ///
    /// The root is named [`DEFAULT_ROOT_NAME`], because Arrow names columns
    /// and never the record. The batch's own columns become this column's
    /// children, shared rather than copied.
    ///
    /// # Errors
    ///
    /// Returns an error when the batch schema does not project to a Yggdryl
    /// Struct root, or [`Self::from_arrow_array`]'s refusal.
    pub fn from_arrow_batch(batch: &RecordBatch) -> Result<Self> {
        let root = field_from_arrow_schema(DEFAULT_ROOT_NAME, batch.schema().as_ref())?;
        let records: ArrayRef = Arc::new(StructArray::from(batch.clone()));
        Self::from_arrow_array(root, records)
    }

    /// Drain one Arrow batch stream into the column of its rows.
    ///
    /// The whole stream is held: a column is one contiguous set of buffers,
    /// so the bound is the stream itself.
    ///
    /// # Errors
    ///
    /// [`Self::from_arrow_batch`] carries the rule, and the reader carries
    /// its own.
    pub fn from_arrow_reader(reader: BatchReader) -> Result<Self> {
        let root = field_from_arrow_schema(DEFAULT_ROOT_NAME, reader.schema().as_ref())?;
        let schema = arrow_schema_from_field(&root)?;
        let gathered = reader.collect::<std::result::Result<Vec<RecordBatch>, _>>()?;
        let joined = arrow_select::concat::concat_batches(&schema, &gathered)?;
        Self::from_arrow_batch(&joined)
    }

    /// Every row of this column as one Arrow table.
    ///
    /// # Errors
    ///
    /// Returns an error unless this column's field is a record root.
    pub fn into_arrow_batch(&self) -> Result<RecordBatch> {
        let schema = arrow_schema_from_field(self.require_field()?)?;
        let array = self.require_arrow_array()?;
        let records = array
            .as_any()
            .downcast_ref::<StructArray>()
            .ok_or_else(|| {
                Error::IncompatibleSchema(format!(
                    "column {:?} is not a record column, so it is not a table",
                    self.field().map_or("$", Field::name)
                ))
            })?;
        Ok(RecordBatch::try_new_with_options(
            schema,
            records.columns().to_vec(),
            &RecordBatchOptions::new().with_row_count(Some(array.len())),
        )?)
    }

    /// Stream this column's rows as one Arrow batch reader, of one batch.
    ///
    /// # Errors
    ///
    /// [`Self::into_arrow_batch`] carries the rule.
    pub fn into_arrow_reader(&self) -> Result<BatchReader> {
        let schema = arrow_schema_from_field(self.require_field()?)?;
        let batch = self.into_arrow_batch()?;
        Ok(batch_reader(schema, [batch]))
    }
}
