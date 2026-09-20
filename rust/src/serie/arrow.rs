//! The one door buffers take into a column, and the one they take out.
//!
//! A field says which leaf its buffers are, so this is where that reading
//! happens and nowhere else: the layout is proven against the field's own
//! Arrow projection, the array is taken as it is - no row read, no buffer
//! copied - and a nested layout recurses into the columns under it.
//!
//! Inline because this section reads `crate::arrow::{Error, Result}` while
//! the rest of the file reads `crate::{Error, Result}`, and one scope cannot
//! hold both names.

use std::sync::Arc;

use arrow_array::OffsetSizeTrait;
use arrow_array::types::{
    BinaryType, BinaryViewType, Date32Type, Date64Type, Decimal32Type, Decimal64Type,
    Decimal128Type, Decimal256Type, DurationMicrosecondType, DurationMillisecondType,
    DurationNanosecondType, DurationSecondType, Float16Type, Float32Type, Float64Type, Int8Type,
    Int16Type, Int32Type, Int64Type, IntervalDayTimeType, IntervalMonthDayNanoType,
    IntervalYearMonthType, LargeBinaryType, LargeUtf8Type, StringViewType, Time32MillisecondType,
    Time32SecondType, Time64MicrosecondType, Time64NanosecondType, TimestampMicrosecondType,
    TimestampMillisecondType, TimestampNanosecondType, TimestampSecondType, UInt8Type, UInt16Type,
    UInt32Type, UInt64Type, Utf8Type,
};
use arrow_array::{
    Array, ArrayRef, BinaryArray, BooleanArray, FixedSizeBinaryArray, GenericByteArray,
    GenericByteViewArray, LargeListArray, ListArray, MapArray, PrimitiveArray, RecordBatch,
    RecordBatchOptions, RecordBatchReader, StructArray,
};
use arrow_buffer::{NullBuffer, OffsetBuffer};
use arrow_schema::{DataType as ArrowDataType, IntervalUnit, TimeUnit as ArrowTimeUnit};

use super::Serie;
use super::nested::{LargeSequenceSerie, MappingSerie, SequenceSerie, StructSerie};
use super::primitive::{BooleanSerie, NullSerie, PrimitiveSerie};
use super::text::{ByteSerie, ByteViewSerie, FixedSerie, Raw, Text};
use super::variant::VariantSerie;
use crate::arrow::{
    BatchReader, Error, Result, arrow_schema_from_field, batch_reader, field_from_arrow_schema,
};
use crate::media::DEFAULT_ROOT_NAME;
use crate::value::SerieValue as _;
use crate::{DataType, DataTypeKind, Field, Scalar};

/// Whether a field reads its bytes as text.
///
/// A string leaf and a registered code both do; everything else stored in
/// bytes - a UUID, a geospatial reading, a plain byte column - does not.
fn is_text(dtype: &DataType) -> bool {
    matches!(dtype, DataType::String(_)) || dtype.id().kind() == DataTypeKind::Code
}

/// Refuse an array whose layout is not the field's, naming both.
///
/// Nullability is part of that reading: a required field admits no absent
/// row, and the validity bitmap answers that in constant time - which is
/// what lets buffers cross without a row being decoded to prove them.
fn require_layout(field: &Field, array: &dyn Array, parent: Option<&NullBuffer>) -> Result<()> {
    field.dtype().validate_bounded()?;
    let expected = field.clone().into_arrow_field_ref()?.data_type().clone();
    if array.data_type() != &expected {
        return Err(Error::IncompatibleSchema(format!(
            "column {:?} lays out as {expected}, got {}",
            field.name(),
            array.data_type(),
        )));
    }
    if field.is_nullable() {
        return Ok(());
    }
    let absent = absent_rows(array, parent);
    if absent != 0 {
        return Err(Error::IncompatibleSchema(format!(
            "column {:?} is not null, got {absent} absent of {} rows",
            field.name(),
            array.len(),
        )));
    }
    Ok(())
}

/// How many rows this array leaves absent that its parent does not.
///
/// A null record row says nothing about the children under it - Arrow leaves
/// their slots unspecified - so a required child is judged only where the
/// record itself is present. That is one pass over the validity words, never
/// over the rows.
fn absent_rows(array: &dyn Array, parent: Option<&NullBuffer>) -> usize {
    let Some(nulls) = array.nulls() else {
        return 0;
    };
    match parent {
        Some(above) if above.len() == nulls.len() => {
            (&!nulls.inner() & above.inner()).count_set_bits()
        }
        _ => nulls.null_count(),
    }
}

/// Downcast one array to the concrete layout its field proved it is.
///
/// [`require_layout`] already compared the physical datatypes, so a failure
/// here is this module disagreeing with itself rather than a caller handing
/// over the wrong array - the refusal names the column it was reading.
fn held<A: Array + Clone + 'static>(field: &Field, array: &ArrayRef) -> Result<A> {
    array.as_any().downcast_ref::<A>().cloned().ok_or_else(|| {
        Error::IncompatibleSchema(format!(
            "column {:?} lays out as {} but does not hold it",
            field.name(),
            array.data_type(),
        ))
    })
}

/// Build the column `field` types out of buffers that already hold it.
fn column_of(field: Arc<Field>, array: &ArrayRef, parent: Option<&NullBuffer>) -> Result<Serie> {
    let field_ref = field.as_ref().clone();
    let dtype = field_ref.dtype().clone();
    // Every level proves itself, so a child whose buffers disagree with the
    // child field is refused where it lies rather than at the row that reads
    // it.
    require_layout(&field_ref, array.as_ref(), parent)?;

    /// One arm per fixed-width leaf: downcast, pair, widen.
    macro_rules! primitive {
        ($arrow:ty) => {
            Ok(PrimitiveSerie::<$arrow>::new(
                field,
                held::<PrimitiveArray<$arrow>>(&field_ref, array)?,
            )
            .into_serie())
        };
    }

    if matches!(dtype, DataType::Variant) {
        // `DataType::Variant` projects to Arrow `Binary` and nothing else,
        // so there is one storage to take and `require_layout` has already
        // refused anything but it.
        return Ok(VariantSerie::new(field, held::<BinaryArray>(&field_ref, array)?).into_serie());
    }

    match array.data_type() {
        ArrowDataType::Null => Ok(NullSerie::new(field, array.len()).into_serie()),
        ArrowDataType::Boolean => {
            Ok(BooleanSerie::new(field, held::<BooleanArray>(&field_ref, array)?).into_serie())
        }
        ArrowDataType::Int8 => primitive!(Int8Type),
        ArrowDataType::Int16 => primitive!(Int16Type),
        ArrowDataType::Int32 => primitive!(Int32Type),
        ArrowDataType::Int64 => primitive!(Int64Type),
        ArrowDataType::UInt8 => primitive!(UInt8Type),
        ArrowDataType::UInt16 => primitive!(UInt16Type),
        ArrowDataType::UInt32 => primitive!(UInt32Type),
        ArrowDataType::UInt64 => primitive!(UInt64Type),
        ArrowDataType::Float16 => primitive!(Float16Type),
        ArrowDataType::Float32 => primitive!(Float32Type),
        ArrowDataType::Float64 => primitive!(Float64Type),
        ArrowDataType::Decimal32(..) => primitive!(Decimal32Type),
        ArrowDataType::Decimal64(..) => primitive!(Decimal64Type),
        ArrowDataType::Decimal128(..) => primitive!(Decimal128Type),
        ArrowDataType::Decimal256(..) => primitive!(Decimal256Type),
        ArrowDataType::Date32 => primitive!(Date32Type),
        ArrowDataType::Date64 => primitive!(Date64Type),
        ArrowDataType::Time32(ArrowTimeUnit::Second) => primitive!(Time32SecondType),
        ArrowDataType::Time32(_) => primitive!(Time32MillisecondType),
        ArrowDataType::Time64(ArrowTimeUnit::Microsecond) => primitive!(Time64MicrosecondType),
        ArrowDataType::Time64(_) => primitive!(Time64NanosecondType),
        ArrowDataType::Timestamp(ArrowTimeUnit::Second, _) => primitive!(TimestampSecondType),
        ArrowDataType::Timestamp(ArrowTimeUnit::Millisecond, _) => {
            primitive!(TimestampMillisecondType)
        }
        ArrowDataType::Timestamp(ArrowTimeUnit::Microsecond, _) => {
            primitive!(TimestampMicrosecondType)
        }
        ArrowDataType::Timestamp(ArrowTimeUnit::Nanosecond, _) => {
            primitive!(TimestampNanosecondType)
        }
        ArrowDataType::Duration(ArrowTimeUnit::Second) => primitive!(DurationSecondType),
        ArrowDataType::Duration(ArrowTimeUnit::Millisecond) => primitive!(DurationMillisecondType),
        ArrowDataType::Duration(ArrowTimeUnit::Microsecond) => primitive!(DurationMicrosecondType),
        ArrowDataType::Duration(ArrowTimeUnit::Nanosecond) => primitive!(DurationNanosecondType),
        ArrowDataType::Interval(IntervalUnit::YearMonth) => primitive!(IntervalYearMonthType),
        ArrowDataType::Interval(IntervalUnit::DayTime) => primitive!(IntervalDayTimeType),
        ArrowDataType::Interval(IntervalUnit::MonthDayNano) => primitive!(IntervalMonthDayNanoType),
        ArrowDataType::Utf8 => Ok(ByteSerie::<Utf8Type, Text>::new(
            field,
            held::<GenericByteArray<Utf8Type>>(&field_ref, array)?,
        )
        .into_serie()),
        ArrowDataType::LargeUtf8 => Ok(ByteSerie::<LargeUtf8Type, Text>::new(
            field,
            held::<GenericByteArray<LargeUtf8Type>>(&field_ref, array)?,
        )
        .into_serie()),
        ArrowDataType::Utf8View => Ok(ByteViewSerie::<StringViewType, Text>::new(
            field,
            held::<GenericByteViewArray<StringViewType>>(&field_ref, array)?,
        )
        .into_serie()),
        ArrowDataType::Binary => {
            let runs = held::<GenericByteArray<BinaryType>>(&field_ref, array)?;
            Ok(if is_text(&dtype) {
                ByteSerie::<BinaryType, Text>::new(field, runs).into_serie()
            } else {
                ByteSerie::<BinaryType, Raw>::new(field, runs).into_serie()
            })
        }
        ArrowDataType::LargeBinary => {
            let runs = held::<GenericByteArray<LargeBinaryType>>(&field_ref, array)?;
            Ok(if is_text(&dtype) {
                ByteSerie::<LargeBinaryType, Text>::new(field, runs).into_serie()
            } else {
                ByteSerie::<LargeBinaryType, Raw>::new(field, runs).into_serie()
            })
        }
        ArrowDataType::BinaryView => {
            let runs = held::<GenericByteViewArray<BinaryViewType>>(&field_ref, array)?;
            Ok(if is_text(&dtype) {
                ByteViewSerie::<BinaryViewType, Text>::new(field, runs).into_serie()
            } else {
                ByteViewSerie::<BinaryViewType, Raw>::new(field, runs).into_serie()
            })
        }
        ArrowDataType::FixedSizeBinary(_) => {
            let runs = held::<FixedSizeBinaryArray>(&field_ref, array)?;
            Ok(if is_text(&dtype) {
                FixedSerie::<Text>::new(field, runs).into_serie()
            } else {
                FixedSerie::<Raw>::new(field, runs).into_serie()
            })
        }
        ArrowDataType::Struct(_) => {
            let records = held::<StructArray>(&field_ref, array)?;
            let children = field_ref
                .dtype()
                .as_fields()
                .ok_or(Error::Internal {
                    site: "serie::arrow::struct",
                })?
                .iter()
                .enumerate()
                .map(|(index, child)| {
                    let column = records.column(index);
                    column_of(Arc::new(child.clone()), column, records.nulls())
                })
                .collect::<Result<Vec<Serie>>>()?;
            Ok(
                StructSerie::new(field, children, records.nulls().cloned(), records.len())
                    .into_serie(),
            )
        }
        ArrowDataType::List(_) => {
            let lists = held::<ListArray>(&field_ref, array)?;
            let item = item_field(&field_ref)?;
            let (offsets, values) = rebased(lists.offsets(), lists.values());
            let items = column_of(Arc::new(item), &values, None)?;
            Ok(SequenceSerie::new(field, offsets, items, lists.nulls().cloned()).into_serie())
        }
        ArrowDataType::LargeList(_) => {
            let lists = held::<LargeListArray>(&field_ref, array)?;
            let item = item_field(&field_ref)?;
            let (offsets, values) = rebased(lists.offsets(), lists.values());
            let items = column_of(Arc::new(item), &values, None)?;
            Ok(LargeSequenceSerie::new(field, offsets, items, lists.nulls().cloned()).into_serie())
        }
        ArrowDataType::Map(..) => {
            let maps = held::<MapArray>(&field_ref, array)?;
            let entries = entry_field(&field_ref)?;
            let entry_array: ArrayRef = Arc::new(maps.entries().clone());
            let (offsets, values) = rebased(maps.offsets(), &entry_array);
            let held_entries = column_of(Arc::new(entries), &values, None)?;
            // The argument order is the sequence leaf's, because a mapping
            // column is that leaf read as entries.
            Ok(MappingSerie::new(field, offsets, held_entries, maps.nulls().cloned()).into_serie())
        }
        other => Err(Error::Unsupported {
            kind: "serie",
            reason: format!("a column of {other} is not one this crate holds"),
        }),
    }
}

/// Rebase a cut onto exactly the items it reaches.
///
/// Arrow slices a list by slicing its offsets and keeping the whole child,
/// so a sliced array's offsets start past zero and its values run past the
/// end. Reading is unaffected - an offset is absolute - but a column that
/// grows has to know where its items end, so the cut is rebased once here
/// and the child sliced to match. An unsliced array is already in that
/// shape and is returned untouched, so nothing is copied for it; a sliced
/// one shares its buffers through Arrow's own slice.
fn rebased<O: OffsetSizeTrait>(
    offsets: &OffsetBuffer<O>,
    values: &ArrayRef,
) -> (OffsetBuffer<O>, ArrayRef) {
    let first = offsets.first().copied().unwrap_or_default();
    let last = offsets.last().copied().unwrap_or_default();
    if first == O::zero() && last.as_usize() == values.len() {
        return (offsets.clone(), ArrayRef::clone(values));
    }
    let shifted: Vec<O> = offsets.iter().map(|offset| *offset - first).collect();
    (
        OffsetBuffer::new(shifted.into()),
        values.slice(first.as_usize(), (last - first).as_usize()),
    )
}

/// The item field a list-shaped field repeats.
fn item_field(field: &Field) -> Result<Field> {
    field
        .dtype()
        .as_sequence_type()
        .map(|sequence| sequence.item().clone())
        .ok_or(Error::Internal {
            site: "serie::arrow::item",
        })
}

/// The entries field a mapping-shaped field repeats.
fn entry_field(field: &Field) -> Result<Field> {
    match field.dtype() {
        DataType::Mapping(mapping) => Ok(mapping.entries().clone()),
        _ => Err(Error::Internal {
            site: "serie::arrow::entries",
        }),
    }
}

impl Serie {
    /// Take one Arrow array as the column of `field`.
    ///
    /// Nothing is copied and no row is read: the layout is proven against the
    /// field's own Arrow projection, and the buffers are taken as they are. A
    /// value they hold that the field's contract would refuse is refused
    /// where it is read.
    ///
    /// # Errors
    ///
    /// Returns an error when the array does not hold `field`'s exact physical
    /// layout, or holds a layout this crate keeps no column for.
    pub fn from_arrow_array(field: Field, array: ArrayRef) -> Result<Self> {
        column_of(Arc::new(field), &array, None)
    }

    /// The empty column of `field`.
    ///
    /// # Errors
    ///
    /// [`Self::from_arrow_array`] carries the rule.
    pub fn empty(field: Field) -> Result<Self> {
        let storage = field.clone().into_arrow_field_ref()?.data_type().clone();
        Self::from_arrow_array(field, arrow_array::new_empty_array(&storage))
    }

    /// Build a column from the rows a field types.
    ///
    /// Each row goes through [`Field::scalar`], the crate's one value
    /// contract, and the rows are laid out once at the crate's one
    /// scalar-array boundary.
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
        Ok(Self::from_arrow_array(field, array)?)
    }

    /// Read one Arrow table as the column of its rows.
    ///
    /// The root is named [`crate::media::DEFAULT_ROOT_NAME`],
    /// because Arrow names columns and never the record. The batch's own
    /// columns become this column's children, shared rather than copied.
    ///
    /// # Errors
    ///
    /// Returns an error when the batch schema does not project to a Yggdryl
    /// Struct root.
    pub fn from_arrow_batch(batch: &RecordBatch) -> Result<Self> {
        let root = field_from_arrow_schema(DEFAULT_ROOT_NAME, batch.schema().as_ref())?;
        let records: ArrayRef = Arc::new(StructArray::from(batch.clone()));
        Self::from_arrow_array(root, records)
    }

    /// Every row of this column as one Arrow table.
    ///
    /// # Errors
    ///
    /// Returns an error unless this column's field is a record root.
    pub fn into_arrow_batch(&self) -> Result<RecordBatch> {
        let schema = arrow_schema_from_field(self.require_field()?)?;
        let array = self.require_arrow_array()?;
        let Some(records) = array.as_any().downcast_ref::<StructArray>() else {
            return Err(Error::Internal {
                site: "serie::into_arrow_batch",
            });
        };
        Ok(RecordBatch::try_new_with_options(
            schema,
            records.columns().to_vec(),
            &RecordBatchOptions::new().with_row_count(Some(array.len())),
        )?)
    }

    /// Stream this column's rows as one Arrow batch reader.
    ///
    /// # Errors
    ///
    /// [`Self::into_arrow_batch`] carries the rule.
    pub fn into_arrow_reader(&self) -> Result<BatchReader> {
        let schema = arrow_schema_from_field(self.require_field()?)?;
        let batch = self.into_arrow_batch()?;
        Ok(batch_reader(schema, [batch]))
    }

    /// Take one Arrow batch stream as the column of its rows.
    ///
    /// # Errors
    ///
    /// [`Self::from_arrow_batch`] carries the rule, and the reader carries
    /// its own.
    pub fn from_arrow_reader(reader: BatchReader) -> Result<Self> {
        let root = field_from_arrow_schema(DEFAULT_ROOT_NAME, reader.schema().as_ref())?;
        let mut gathered: Vec<RecordBatch> = Vec::new();
        for batch in reader {
            gathered.push(batch?);
        }
        let schema = arrow_schema_from_field(&root)?;
        let joined = arrow_select::concat::concat_batches(&schema, &gathered)?;
        Self::from_arrow_batch(&joined)
    }
}
