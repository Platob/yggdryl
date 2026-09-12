//! Bounded scratch and output reservations for Arrow casts.

use std::sync::Arc;

use arrow_array::types::{
    BinaryType, BinaryViewType, ByteArrayType, ByteViewType, Int8Type, Int16Type, Int32Type,
    Int64Type, LargeBinaryType, LargeUtf8Type, RunEndIndexType, StringViewType, UInt8Type,
    UInt16Type, UInt32Type, UInt64Type, Utf8Type,
};
use arrow_array::{
    Array, ArrayRef, BinaryViewArray, DictionaryArray, FixedSizeListArray, GenericByteArray,
    GenericByteViewArray, Int16RunArray, Int32RunArray, Int64RunArray, LargeListArray,
    LargeListViewArray, ListArray, ListViewArray, MapArray, RunArray, StringViewArray, StructArray,
    UnionArray,
};
use arrow_buffer::ArrowNativeType;
use arrow_cast::display::{ArrayFormatter, FormatOptions};
use arrow_schema::DataType as ArrowDataType;

use crate::arrow::{Error, Result};
use crate::types::bytes::casts::{
    byte_array_storage_ptr_eq, checked_valid_payload_bytes, projected_byte_len,
};
use crate::types::cast::downcast;
use crate::types::nested::casts::{dictionary_values_ref, offset_pair};
use crate::types::{bytes, string};
use crate::{DataType, Field, UnionMode};

mod limits;
mod selection;

pub(crate) use limits::{
    MAX_PHYSICAL_SLOTS, MaterializationBudget, checked_physical_mul, invalid_value,
    physical_limit_error, physical_union_branch, unsupported,
};
pub(crate) use selection::SourceSelection;

pub(crate) fn reserve_vec_bytes<T>(
    budget: &mut MaterializationBudget,
    capacity: usize,
) -> Result<()> {
    let bytes = std::mem::size_of::<T>()
        .checked_mul(capacity)
        .ok_or_else(|| {
            Error::IncompatibleSchema(
                "masked Arrow source scratch allocation exceeds usize".to_owned(),
            )
        })?;
    budget.add_bytes(bytes)
}

pub(crate) fn scratch_vec<T>(
    budget: &mut MaterializationBudget,
    capacity: usize,
    purpose: &str,
) -> Result<Vec<T>> {
    reserve_vec_bytes::<T>(budget, capacity)?;
    let mut values = Vec::new();
    values.try_reserve_exact(capacity).map_err(|error| {
        Error::IncompatibleSchema(format!("masked Arrow {purpose} allocation failed: {error}"))
    })?;
    Ok(values)
}

pub(crate) fn selected_child_ranges<Valid, Range>(
    selection: SourceSelection<'_>,
    parent_len: usize,
    child_len: usize,
    is_valid: Valid,
    range: Range,
    budget: &mut MaterializationBudget,
) -> Result<Vec<(usize, usize)>>
where
    Valid: Fn(usize) -> bool,
    Range: Fn(usize) -> Result<(usize, usize)>,
{
    let mut range_count = 0usize;
    let mut previous_end = None;
    selection.try_for_each(parent_len, |row| {
        if !is_valid(row) {
            return Ok(());
        }
        let (start, end) = range(row)?;
        if start > end || end > child_len {
            return Err(Error::IncompatibleSchema(
                "masked Arrow nested source range exceeds its child array".to_owned(),
            ));
        }
        if start != end {
            if previous_end != Some(start) {
                range_count = range_count.checked_add(1).ok_or_else(|| {
                    Error::IncompatibleSchema(
                        "masked Arrow nested range count exceeds usize".to_owned(),
                    )
                })?;
            }
            previous_end = Some(end);
        }
        Ok(())
    })?;

    let mut ranges = scratch_vec::<(usize, usize)>(budget, range_count, "range scratch")?;
    selection.try_for_each(parent_len, |row| {
        if !is_valid(row) {
            return Ok(());
        }
        let (start, end) = range(row)?;
        if start == end {
            return Ok(());
        }
        if let Some(previous) = ranges.last_mut() {
            if previous.1 == start {
                previous.1 = end;
                return Ok(());
            }
        }
        ranges.push((start, end));
        Ok(())
    })?;
    Ok(ranges)
}

fn reserve_byte_payload(
    selection: SourceSelection<'_>,
    array_len: usize,
    mut value_len: impl FnMut(usize) -> usize,
    budget: &mut MaterializationBudget,
) -> Result<()> {
    let mut bytes = 0usize;
    selection.try_for_each(array_len, |index| {
        bytes = bytes.checked_add(value_len(index)).ok_or_else(|| {
            Error::IncompatibleSchema(
                "masked Arrow selected source payload exceeds usize".to_owned(),
            )
        })?;
        Ok(())
    })?;
    budget.add_bytes(bytes)
}

/// The bytes a selection of one byte array's cells hold, text or binary alike.
fn reserve_selected_bytes<T: ByteArrayType>(
    array: &dyn Array,
    selection: SourceSelection<'_>,
    budget: &mut MaterializationBudget,
) -> Result<()> {
    let array = downcast::<GenericByteArray<T>>(array)?;
    reserve_byte_payload(
        selection,
        array.len(),
        |index| {
            if array.is_valid(index) {
                AsRef::<[u8]>::as_ref(array.value(index)).len()
            } else {
                0
            }
        },
        budget,
    )
}

/// One buffer handle per data buffer a view array shares.
fn reserve_view_buffers<T: ByteViewType>(
    array: &dyn Array,
    budget: &mut MaterializationBudget,
) -> Result<()> {
    let array = downcast::<GenericByteViewArray<T>>(array)?;
    reserve_vec_bytes::<arrow_buffer::Buffer>(budget, array.data_buffers().len())
}

/// The payload a selection of one string or byte column's cells hold, in
/// whichever storage its parameters lay out. A fixed width was charged by
/// the layout.
fn reserve_storage_source_payload(
    array: &dyn Array,
    storage: ArrowDataType,
    selection: SourceSelection<'_>,
    budget: &mut MaterializationBudget,
) -> Result<()> {
    match storage {
        ArrowDataType::Utf8 => reserve_selected_bytes::<Utf8Type>(array, selection, budget),
        ArrowDataType::LargeUtf8 => {
            reserve_selected_bytes::<LargeUtf8Type>(array, selection, budget)
        }
        ArrowDataType::Binary => reserve_selected_bytes::<BinaryType>(array, selection, budget),
        ArrowDataType::LargeBinary => {
            reserve_selected_bytes::<LargeBinaryType>(array, selection, budget)
        }
        ArrowDataType::Utf8View => reserve_view_buffers::<StringViewType>(array, budget),
        ArrowDataType::BinaryView => reserve_view_buffers::<BinaryViewType>(array, budget),
        _ => Ok(()),
    }
}

fn reserve_run_source_take<R: RunEndIndexType>(
    source: &RunArray<R>,
    encoded: &crate::RunEndEncodedType,
    selection: SourceSelection<'_>,
    budget: &mut MaterializationBudget,
) -> Result<()> {
    let selected_count = selection.row_count(source.len())?;
    // Arrow's run take first expands every selected logical index to usize.
    reserve_vec_bytes::<usize>(budget, selected_count)?;

    let mut run_count = 0usize;
    let mut previous = None;
    selection.try_for_each(source.len(), |row| {
        let physical = source.get_physical_index(row);
        if previous != Some(physical) {
            run_count += 1;
            previous = Some(physical);
        }
        Ok(())
    })?;
    budget.add_array(encoded.run_ends().dtype(), run_count)?;
    budget.add_array_layout(&DataType::UInt32, run_count)?;

    let mut value_indices = Vec::new();
    value_indices
        .try_reserve_exact(run_count)
        .map_err(|error| {
            Error::IncompatibleSchema(format!(
                "masked Arrow run-value index allocation failed: {error}"
            ))
        })?;
    previous = None;
    selection.try_for_each(source.len(), |row| {
        let physical = source.get_physical_index(row);
        if previous != Some(physical) {
            value_indices.push(u32::try_from(physical).map_err(|_| {
                Error::IncompatibleSchema("masked Arrow run-value index exceeds UInt32".to_owned())
            })?);
            previous = Some(physical);
        }
        Ok(())
    })?;
    reserve_source_selection(
        source.values().as_ref(),
        encoded.values().dtype(),
        SourceSelection::Indices(&value_indices),
        budget,
    )
}

#[allow(clippy::too_many_lines)] // Mirrors Arrow's exhaustive take dispatch and sharing rules.
pub(crate) fn reserve_source_selection(
    array: &dyn Array,
    source_type: &DataType,
    selection: SourceSelection<'_>,
    budget: &mut MaterializationBudget,
) -> Result<()> {
    let selected_count = selection.row_count(array.len())?;
    budget.add_array_layout(source_type, selected_count)?;
    reserve_source_children_and_payload(array, source_type, selection, budget)
}

#[allow(clippy::too_many_lines)] // Mirrors Arrow's exhaustive take dispatch and sharing rules.
fn reserve_source_children_and_payload(
    array: &dyn Array,
    source_type: &DataType,
    selection: SourceSelection<'_>,
    budget: &mut MaterializationBudget,
) -> Result<()> {
    let selected_count = selection.row_count(array.len())?;
    match source_type {
        DataType::Bytes(parameters) => reserve_storage_source_payload(
            array,
            bytes::arrow_storage(*parameters)?,
            selection,
            budget,
        )?,
        DataType::String(parameters) => reserve_storage_source_payload(
            array,
            string::arrow_storage(*parameters)?,
            selection,
            budget,
        )?,
        DataType::List(child) => {
            let array = downcast::<ListArray>(array)?;
            let offsets = array.value_offsets();
            let ranges = selected_child_ranges(
                selection,
                array.len(),
                array.values().len(),
                |row| array.is_valid(row),
                |row| offset_pair(i64::from(offsets[row]), i64::from(offsets[row + 1])),
                budget,
            )?;
            reserve_source_selection(
                array.values().as_ref(),
                child.dtype(),
                SourceSelection::Ranges(&ranges),
                budget,
            )?;
        }
        DataType::LargeList(child) => {
            let array = downcast::<LargeListArray>(array)?;
            let offsets = array.value_offsets();
            let ranges = selected_child_ranges(
                selection,
                array.len(),
                array.values().len(),
                |row| array.is_valid(row),
                |row| offset_pair(offsets[row], offsets[row + 1]),
                budget,
            )?;
            reserve_source_selection(
                array.values().as_ref(),
                child.dtype(),
                SourceSelection::Ranges(&ranges),
                budget,
            )?;
        }
        DataType::FixedSizeList(child, size) => {
            let array = downcast::<FixedSizeListArray>(array)?;
            let size = usize::try_from(*size).map_err(|_| {
                Error::IncompatibleSchema(
                    "masked Arrow fixed-size-list width is negative".to_owned(),
                )
            })?;
            let ranges = selected_child_ranges(
                selection,
                array.len(),
                array.values().len(),
                |_| true,
                |row| {
                    let start = row.checked_mul(size).ok_or_else(|| {
                        Error::IncompatibleSchema(
                            "masked Arrow fixed-size-list offset exceeds usize".to_owned(),
                        )
                    })?;
                    let end = start.checked_add(size).ok_or_else(|| {
                        Error::IncompatibleSchema(
                            "masked Arrow fixed-size-list range exceeds usize".to_owned(),
                        )
                    })?;
                    Ok((start, end))
                },
                budget,
            )?;
            let child_count = SourceSelection::Ranges(&ranges).row_count(array.values().len())?;
            // Arrow expands fixed-list rows into a UInt32 child-index buffer.
            budget.add_array_layout(&DataType::UInt32, child_count)?;
            reserve_source_selection(
                array.values().as_ref(),
                child.dtype(),
                SourceSelection::Ranges(&ranges),
                budget,
            )?;
        }
        DataType::Struct(fields) => {
            let array = downcast::<StructArray>(array)?;
            if fields.len() != array.num_columns() {
                return Err(Error::IncompatibleSchema(
                    "masked Arrow source Struct child count changed after planning".to_owned(),
                ));
            }
            for (field, child) in fields.iter().zip(array.columns()) {
                reserve_source_selection(child.as_ref(), field.dtype(), selection, budget)?;
            }
        }
        DataType::Map(map) => {
            let array = downcast::<MapArray>(array)?;
            let offsets = array.value_offsets();
            let ranges = selected_child_ranges(
                selection,
                array.len(),
                array.entries().len(),
                |row| array.is_valid(row),
                |row| offset_pair(i64::from(offsets[row]), i64::from(offsets[row + 1])),
                budget,
            )?;
            reserve_source_selection(
                array.entries(),
                map.entries().dtype(),
                SourceSelection::Ranges(&ranges),
                budget,
            )?;
        }
        DataType::Union(fields, UnionMode::Sparse) => {
            let array = downcast::<UnionArray>(array)?;
            for (type_id, field) in fields {
                reserve_source_selection(
                    array.child(type_id).as_ref(),
                    field.dtype(),
                    selection,
                    budget,
                )?;
            }
        }
        DataType::Union(fields, UnionMode::Dense) => {
            let array = downcast::<UnionArray>(array)?;
            // Dense take keeps one row-sized mask and filtered-offset scratch
            // alive at a time while retaining every already-built child.
            budget.add_bitmap(selected_count)?;
            budget.add_array_layout(&DataType::UInt32, selected_count)?;
            let mut branch = Vec::new();
            branch.try_reserve_exact(selected_count).map_err(|error| {
                Error::IncompatibleSchema(format!(
                    "masked Arrow dense-union index allocation failed: {error}"
                ))
            })?;
            for (type_id, field) in fields {
                branch.clear();
                selection.try_for_each(array.len(), |row| {
                    if array.type_id(row) == type_id {
                        branch.push(u32::try_from(array.value_offset(row)).map_err(|_| {
                            Error::IncompatibleSchema(
                                "masked Arrow dense-union offset exceeds UInt32".to_owned(),
                            )
                        })?);
                    }
                    Ok(())
                })?;
                reserve_source_selection(
                    array.child(type_id).as_ref(),
                    field.dtype(),
                    SourceSelection::Indices(&branch),
                    budget,
                )?;
            }
        }
        DataType::RunEndEncoded(encoded) => match encoded.run_ends().dtype() {
            DataType::Int16 => reserve_run_source_take(
                downcast::<Int16RunArray>(array)?,
                encoded,
                selection,
                budget,
            )?,
            DataType::Int32 => reserve_run_source_take(
                downcast::<Int32RunArray>(array)?,
                encoded,
                selection,
                budget,
            )?,
            DataType::Int64 => reserve_run_source_take(
                downcast::<Int64RunArray>(array)?,
                encoded,
                selection,
                budget,
            )?,
            _ => {
                return Err(Error::IncompatibleSchema(
                    "masked Arrow run-end type must be Int16, Int32, or Int64".to_owned(),
                ));
            }
        },
        // List views and dictionaries share their child/value arrays. All
        // scalar/fixed-width storage was fully charged by the shallow layout.
        _ => {}
    }
    Ok(())
}

#[allow(clippy::too_many_lines)]
#[derive(Default)]
struct CountingWriter {
    bytes: usize,
}

impl std::fmt::Write for CountingWriter {
    fn write_str(&mut self, value: &str) -> std::fmt::Result {
        self.bytes = self.bytes.checked_add(value.len()).ok_or(std::fmt::Error)?;
        Ok(())
    }
}

fn reserve_formatted_payload(
    array: &dyn Array,
    selection: SourceSelection<'_>,
    target_is_view: bool,
    budget: &mut MaterializationBudget,
) -> Result<()> {
    let options = FormatOptions::default();
    let formatter = ArrayFormatter::try_new(array, &options)?;
    let mut total = 0usize;
    let mut maximum = 0usize;
    selection.try_for_each(array.len(), |index| {
        if array.is_null(index) {
            return Ok(());
        }
        let mut counter = CountingWriter::default();
        formatter.value(index).write(&mut counter)?;
        total = total.checked_add(counter.bytes).ok_or_else(|| {
            Error::IncompatibleSchema("Arrow formatted payload exceeds usize".to_owned())
        })?;
        maximum = maximum.max(counter.bytes);
        Ok(())
    })?;
    budget.add_bytes(total)?;
    // Utf8View formatting keeps one growable String alive while appending the
    // final view buffers. Account for its largest selected logical value.
    if target_is_view {
        budget.add_bytes(maximum)?;
    }
    Ok(())
}

pub(crate) fn reserve_cast_output_payload(
    array: &dyn Array,
    source_type: &DataType,
    target_type: &DataType,
    selection: SourceSelection<'_>,
    budget: &mut MaterializationBudget,
) -> Result<()> {
    match target_type {
        // A string's payload is its characters whatever charset writes them:
        // every charset here is at most one byte per scalar above US-ASCII,
        // so the UTF-8 rendering is the reservation's upper bound.
        DataType::String(parameters) => {
            reserve_formatted_payload(array, selection, parameters.layout().is_view(), budget)
        }
        // Offset storage copies every projected payload; a view shares its
        // buffers and a fixed width was charged by the layout.
        DataType::Bytes(parameters) => match bytes::arrow_storage(*parameters)? {
            ArrowDataType::Binary | ArrowDataType::LargeBinary => {
                let mut bytes = 0usize;
                selection.try_for_each(array.len(), |index| {
                    bytes = bytes
                        .checked_add(projected_byte_len(array, source_type, index)?)
                        .ok_or_else(|| {
                            Error::IncompatibleSchema(
                                "Arrow cast output payload exceeds usize".to_owned(),
                            )
                        })?;
                    Ok(())
                })?;
                budget.add_bytes(bytes)
            }
            _ => Ok(()),
        },
        DataType::List(_)
        | DataType::LargeList(_)
        | DataType::FixedSizeList(..)
        | DataType::Struct(_)
        | DataType::Map(_)
        | DataType::Union(..)
        | DataType::Dictionary(_)
        | DataType::RunEndEncoded(_) => {
            reserve_source_children_and_payload(array, source_type, selection, budget)
        }
        _ => Ok(()),
    }
}

pub(crate) fn reserve_selected_source_take(
    array: &dyn Array,
    selected: &[u32],
    target_type: &DataType,
    output_copies: usize,
    budget: &mut MaterializationBudget,
) -> Result<()> {
    let source_type = DataType::from_arrow(array.data_type())?;
    let selection = SourceSelection::Indices(selected);
    reserve_source_selection(array, &source_type, selection, budget)?;
    for _ in 0..output_copies {
        reserve_cast_output_payload(array, &source_type, target_type, selection, budget)?;
    }
    Ok(())
}

#[allow(clippy::too_many_lines)]
pub(crate) fn reserve_concat_copy(
    array: &dyn Array,
    dtype: &DataType,
    budget: &mut MaterializationBudget,
) -> Result<()> {
    match dtype {
        DataType::Bytes(parameters) if parameters.layout().is_view() => {
            let array = downcast::<BinaryViewArray>(array)?;
            budget.add_array_layout(dtype, array.len())?;
            reserve_vec_bytes::<arrow_buffer::Buffer>(budget, array.data_buffers().len())?;
        }
        // Dictionary inputs are vocabulary-aligned before concat, so Arrow
        // allocates only the concatenated key array and retains one vocab Arc.
        DataType::Dictionary(_) => budget.add_array_layout(dtype, array.len())?,
        DataType::List(child) => {
            let array = downcast::<ListArray>(array)?;
            budget.add_array_layout(dtype, array.len())?;
            let start = array.offsets()[0].as_usize();
            let end = array.offsets()[array.len()].as_usize();
            let values = array.values().slice(start, end - start);
            reserve_concat_copy(values.as_ref(), child.dtype(), budget)?;
        }
        DataType::LargeList(child) => {
            let array = downcast::<LargeListArray>(array)?;
            budget.add_array_layout(dtype, array.len())?;
            let start = array.offsets()[0].as_usize();
            let end = array.offsets()[array.len()].as_usize();
            let values = array.values().slice(start, end - start);
            reserve_concat_copy(values.as_ref(), child.dtype(), budget)?;
        }
        // Arrow concat preserves every ListView backing child, including
        // ranges not referenced by a logical view.
        DataType::ListView(child) => {
            let array = downcast::<ListViewArray>(array)?;
            budget.add_array_layout(dtype, array.len())?;
            reserve_concat_copy(array.values().as_ref(), child.dtype(), budget)?;
        }
        DataType::LargeListView(child) => {
            let array = downcast::<LargeListViewArray>(array)?;
            budget.add_array_layout(dtype, array.len())?;
            reserve_concat_copy(array.values().as_ref(), child.dtype(), budget)?;
        }
        DataType::FixedSizeList(child, _) => {
            let array = downcast::<FixedSizeListArray>(array)?;
            budget.add_array_layout(dtype, array.len())?;
            reserve_concat_copy(array.values().as_ref(), child.dtype(), budget)?;
        }
        DataType::Struct(fields) => {
            let array = downcast::<StructArray>(array)?;
            budget.add_array_layout(dtype, array.len())?;
            for (field, child) in fields.iter().zip(array.columns()) {
                reserve_concat_copy(child.as_ref(), field.dtype(), budget)?;
            }
        }
        DataType::Map(map) => {
            let array = downcast::<MapArray>(array)?;
            budget.add_array_layout(dtype, array.len())?;
            let start = array.offsets()[0].as_usize();
            let end = array.offsets()[array.len()].as_usize();
            let entries: ArrayRef = Arc::new(array.entries().slice(start, end - start));
            reserve_concat_copy(entries.as_ref(), map.entries().dtype(), budget)?;
        }
        // Union uses MutableArrayData, which visits every physical child, not
        // only the active branch selected by each logical row.
        DataType::Union(fields, _) => {
            let array = downcast::<UnionArray>(array)?;
            budget.add_array_layout(dtype, array.len())?;
            for (type_id, field) in fields {
                reserve_concat_copy(array.child(type_id).as_ref(), field.dtype(), budget)?;
            }
        }
        DataType::RunEndEncoded(encoded) => {
            macro_rules! reserve_run {
                ($run:ty) => {{
                    let array = downcast::<RunArray<$run>>(array)?;
                    let values = array.values_slice();
                    budget.add_physical_slots(values.len())?;
                    budget.add_array(encoded.run_ends().dtype(), values.len())?;
                    reserve_concat_copy(values.as_ref(), encoded.values().dtype(), budget)?
                }};
            }
            match encoded.run_ends().dtype() {
                DataType::Int16 => reserve_run!(Int16Type),
                DataType::Int32 => reserve_run!(Int32Type),
                DataType::Int64 => reserve_run!(Int64Type),
                _ => {
                    return Err(Error::IncompatibleSchema(
                        "run-end type is not a supported signed integer".to_owned(),
                    ));
                }
            }
        }
        _ => {
            let full = [(0, array.len())];
            reserve_source_selection(array, dtype, SourceSelection::Ranges(&full), budget)?;
        }
    }
    Ok(())
}

fn reserve_new_materialized_array_without_dictionary_values(
    output: &ArrayRef,
    source: &ArrayRef,
    dtype: &DataType,
    budget: &mut MaterializationBudget,
) -> Result<()> {
    if Arc::ptr_eq(output, source) {
        return Ok(());
    }
    if !matches!(dtype, DataType::RunEndEncoded(_)) {
        budget.add_array_layout(dtype, output.len())?;
    }
    match dtype {
        DataType::Bytes(parameters) => match bytes::arrow_storage(*parameters)? {
            ArrowDataType::Binary => reserve_new_bytes::<BinaryType>(output, budget)?,
            ArrowDataType::LargeBinary => reserve_new_bytes::<LargeBinaryType>(output, budget)?,
            ArrowDataType::BinaryView => {
                reserve_new_views::<BinaryViewType>(output, source, budget)?;
            }
            // A fixed width was charged by the layout.
            _ => {}
        },
        DataType::String(parameters) => match string::arrow_storage(*parameters)? {
            ArrowDataType::Utf8 => reserve_new_bytes::<Utf8Type>(output, budget)?,
            ArrowDataType::LargeUtf8 => reserve_new_bytes::<LargeUtf8Type>(output, budget)?,
            ArrowDataType::Binary => reserve_new_bytes::<BinaryType>(output, budget)?,
            ArrowDataType::LargeBinary => reserve_new_bytes::<LargeBinaryType>(output, budget)?,
            ArrowDataType::Utf8View => reserve_new_views::<StringViewType>(output, source, budget)?,
            ArrowDataType::BinaryView => {
                reserve_new_views::<BinaryViewType>(output, source, budget)?;
            }
            // A fixed width was charged by the layout.
            _ => {}
        },
        DataType::List(child) => reserve_new_materialized_array_without_dictionary_values(
            downcast::<ListArray>(output.as_ref())?.values(),
            downcast::<ListArray>(source.as_ref())?.values(),
            child.dtype(),
            budget,
        )?,
        DataType::LargeList(child) => reserve_new_materialized_array_without_dictionary_values(
            downcast::<LargeListArray>(output.as_ref())?.values(),
            downcast::<LargeListArray>(source.as_ref())?.values(),
            child.dtype(),
            budget,
        )?,
        DataType::ListView(child) => reserve_new_materialized_array_without_dictionary_values(
            downcast::<ListViewArray>(output.as_ref())?.values(),
            downcast::<ListViewArray>(source.as_ref())?.values(),
            child.dtype(),
            budget,
        )?,
        DataType::LargeListView(child) => {
            reserve_new_materialized_array_without_dictionary_values(
                downcast::<LargeListViewArray>(output.as_ref())?.values(),
                downcast::<LargeListViewArray>(source.as_ref())?.values(),
                child.dtype(),
                budget,
            )?;
        }
        DataType::FixedSizeList(child, _) => {
            reserve_new_materialized_array_without_dictionary_values(
                downcast::<FixedSizeListArray>(output.as_ref())?.values(),
                downcast::<FixedSizeListArray>(source.as_ref())?.values(),
                child.dtype(),
                budget,
            )?;
        }
        DataType::Struct(fields) => {
            let output = downcast::<StructArray>(output.as_ref())?;
            let source = downcast::<StructArray>(source.as_ref())?;
            for ((field, output), source) in
                fields.iter().zip(output.columns()).zip(source.columns())
            {
                reserve_new_materialized_array_without_dictionary_values(
                    output,
                    source,
                    field.dtype(),
                    budget,
                )?;
            }
        }
        DataType::Map(map) => {
            let output: ArrayRef =
                Arc::new(downcast::<MapArray>(output.as_ref())?.entries().clone());
            let source: ArrayRef =
                Arc::new(downcast::<MapArray>(source.as_ref())?.entries().clone());
            reserve_new_materialized_array_without_dictionary_values(
                &output,
                &source,
                map.entries().dtype(),
                budget,
            )?;
        }
        DataType::Union(fields, _) => {
            let output = downcast::<UnionArray>(output.as_ref())?;
            let source = downcast::<UnionArray>(source.as_ref())?;
            for (type_id, field) in fields {
                reserve_new_materialized_array_without_dictionary_values(
                    output.child(type_id),
                    source.child(type_id),
                    field.dtype(),
                    budget,
                )?;
            }
        }
        DataType::RunEndEncoded(encoded) => {
            macro_rules! reserve_run {
                ($run:ty) => {{
                    let output = downcast::<RunArray<$run>>(output.as_ref())?;
                    let source = downcast::<RunArray<$run>>(source.as_ref())?;
                    budget.add_physical_slots(output.values().len())?;
                    budget.add_array(encoded.run_ends().dtype(), output.values().len())?;
                    reserve_new_materialized_array_without_dictionary_values(
                        output.values(),
                        source.values(),
                        encoded.values().dtype(),
                        budget,
                    )?;
                }};
            }
            match encoded.run_ends().dtype() {
                DataType::Int16 => reserve_run!(Int16Type),
                DataType::Int32 => reserve_run!(Int32Type),
                DataType::Int64 => reserve_run!(Int64Type),
                _ => {
                    return Err(Error::IncompatibleSchema(
                        "run-end type is not a supported signed integer".to_owned(),
                    ));
                }
            }
        }
        _ => {}
    }
    Ok(())
}

/// The payload bytes one materialized byte array holds, text or binary alike.
fn reserve_new_bytes<T: ByteArrayType>(
    output: &ArrayRef,
    budget: &mut MaterializationBudget,
) -> Result<()> {
    let output = downcast::<GenericByteArray<T>>(output.as_ref())?;
    budget.add_bytes(checked_valid_payload_bytes(
        output.len(),
        |index| output.is_valid(index),
        |index| AsRef::<[u8]>::as_ref(output.value(index)).len(),
    )?)
}

/// The data buffers a materialized view array holds beyond its source's.
fn reserve_new_views<T: ByteViewType>(
    output: &ArrayRef,
    source: &ArrayRef,
    budget: &mut MaterializationBudget,
) -> Result<()> {
    reserve_new_view_buffers(
        downcast::<GenericByteViewArray<T>>(output.as_ref())?.data_buffers(),
        downcast::<GenericByteViewArray<T>>(source.as_ref())?.data_buffers(),
        budget,
    )
}

fn reserve_new_view_buffers(
    output: &[arrow_buffer::Buffer],
    source: &[arrow_buffer::Buffer],
    budget: &mut MaterializationBudget,
) -> Result<()> {
    reserve_vec_bytes::<arrow_buffer::Buffer>(budget, output.len())?;
    let shared_prefix = output
        .iter()
        .zip(source)
        .take_while(|(output, source)| output.ptr_eq(source))
        .count();
    if output.len() == source.len() && shared_prefix == source.len() {
        return Ok(());
    }

    let phase = budget.mark();
    let mut source_buffers =
        scratch_vec::<(usize, usize)>(budget, source.len(), "ByteView shared-buffer identities")?;
    source_buffers.extend(
        source
            .iter()
            .map(|buffer| (buffer.as_ptr() as usize, buffer.len())),
    );
    source_buffers.sort_unstable();
    source_buffers.dedup();
    let mut new_buffers =
        scratch_vec::<(usize, usize)>(budget, output.len(), "ByteView new-buffer identities")?;
    for buffer in output {
        let identity = (buffer.as_ptr() as usize, buffer.len());
        if source_buffers.binary_search(&identity).is_err() {
            new_buffers.push(identity);
        }
    }
    new_buffers.sort_unstable();
    new_buffers.dedup();
    let new_bytes = new_buffers.iter().try_fold(0usize, |bytes, (_, len)| {
        bytes.checked_add(*len).ok_or_else(|| {
            Error::IncompatibleSchema("ByteView backing bytes exceed usize".to_owned())
        })
    })?;
    budget.restore(phase);
    budget.add_bytes(new_bytes)?;
    Ok(())
}

#[allow(clippy::too_many_lines)] // Mirrors every nested Arrow container layout.
pub(crate) fn reserve_new_dictionary_vocabularies(
    output: &ArrayRef,
    source: &ArrayRef,
    dtype: &DataType,
    budget: &mut MaterializationBudget,
) -> Result<()> {
    match dtype {
        DataType::Dictionary(dictionary) => {
            let output_values = dictionary_values_ref(output.as_ref(), dictionary)?;
            let source_values = dictionary_values_ref(source.as_ref(), dictionary)?;
            let shared = if Arc::ptr_eq(output_values, source_values)
                || byte_array_storage_ptr_eq(
                    output_values.as_ref(),
                    source_values.as_ref(),
                    dictionary.value(),
                )? {
                true
            } else {
                let phase = budget.mark();
                reserve_to_data_scratch(output_values, budget)?;
                reserve_to_data_scratch(source_values, budget)?;
                let shared = output_values.to_data().ptr_eq(&source_values.to_data());
                budget.restore(phase);
                shared
            };
            if !shared {
                reserve_new_materialized_array_without_dictionary_values(
                    output_values,
                    source_values,
                    dictionary.value(),
                    budget,
                )?;
                reserve_new_dictionary_vocabularies(
                    output_values,
                    source_values,
                    dictionary.value(),
                    budget,
                )?;
            }
        }
        DataType::Struct(fields) => {
            let output = downcast::<StructArray>(output.as_ref())?;
            let source = downcast::<StructArray>(source.as_ref())?;
            for ((field, output), source) in
                fields.iter().zip(output.columns()).zip(source.columns())
            {
                reserve_new_dictionary_vocabularies(output, source, field.dtype(), budget)?;
            }
        }
        DataType::List(child) => reserve_new_dictionary_vocabularies(
            downcast::<ListArray>(output.as_ref())?.values(),
            downcast::<ListArray>(source.as_ref())?.values(),
            child.dtype(),
            budget,
        )?,
        DataType::LargeList(child) => reserve_new_dictionary_vocabularies(
            downcast::<LargeListArray>(output.as_ref())?.values(),
            downcast::<LargeListArray>(source.as_ref())?.values(),
            child.dtype(),
            budget,
        )?,
        DataType::ListView(child) => reserve_new_dictionary_vocabularies(
            downcast::<ListViewArray>(output.as_ref())?.values(),
            downcast::<ListViewArray>(source.as_ref())?.values(),
            child.dtype(),
            budget,
        )?,
        DataType::LargeListView(child) => reserve_new_dictionary_vocabularies(
            downcast::<LargeListViewArray>(output.as_ref())?.values(),
            downcast::<LargeListViewArray>(source.as_ref())?.values(),
            child.dtype(),
            budget,
        )?,
        DataType::FixedSizeList(child, _) => reserve_new_dictionary_vocabularies(
            downcast::<FixedSizeListArray>(output.as_ref())?.values(),
            downcast::<FixedSizeListArray>(source.as_ref())?.values(),
            child.dtype(),
            budget,
        )?,
        DataType::Map(map) => {
            let output: ArrayRef =
                Arc::new(downcast::<MapArray>(output.as_ref())?.entries().clone());
            let source: ArrayRef =
                Arc::new(downcast::<MapArray>(source.as_ref())?.entries().clone());
            reserve_new_dictionary_vocabularies(&output, &source, map.entries().dtype(), budget)?;
        }
        DataType::Union(fields, _) => {
            let output = downcast::<UnionArray>(output.as_ref())?;
            let source = downcast::<UnionArray>(source.as_ref())?;
            for (type_id, field) in fields {
                reserve_new_dictionary_vocabularies(
                    output.child(type_id),
                    source.child(type_id),
                    field.dtype(),
                    budget,
                )?;
            }
        }
        DataType::RunEndEncoded(encoded) => {
            macro_rules! reserve_run {
                ($run:ty) => {{
                    let output = downcast::<RunArray<$run>>(output.as_ref())?;
                    let source = downcast::<RunArray<$run>>(source.as_ref())?;
                    reserve_new_dictionary_vocabularies(
                        output.values(),
                        source.values(),
                        encoded.values().dtype(),
                        budget,
                    )?;
                }};
            }
            match encoded.run_ends().dtype() {
                DataType::Int16 => reserve_run!(Int16Type),
                DataType::Int32 => reserve_run!(Int32Type),
                DataType::Int64 => reserve_run!(Int64Type),
                _ => {
                    return Err(Error::IncompatibleSchema(
                        "run-end type is not a supported signed integer".to_owned(),
                    ));
                }
            }
        }
        _ => {}
    }
    Ok(())
}

#[allow(clippy::too_many_lines)] // Mirrors Arrow ArrayData buffers and child trees.
pub(crate) fn reserve_to_data_scratch(
    array: &ArrayRef,
    budget: &mut MaterializationBudget,
) -> Result<()> {
    let buffer_count = match array.data_type() {
        ArrowDataType::Null
        | ArrowDataType::Struct(_)
        | ArrowDataType::FixedSizeList(_, _)
        | ArrowDataType::RunEndEncoded(_, _) => 0,
        ArrowDataType::Binary
        | ArrowDataType::LargeBinary
        | ArrowDataType::Utf8
        | ArrowDataType::LargeUtf8
        | ArrowDataType::ListView(_)
        | ArrowDataType::LargeListView(_)
        | ArrowDataType::Union(_, arrow_schema::UnionMode::Dense) => 2,
        ArrowDataType::BinaryView => downcast::<BinaryViewArray>(array.as_ref())?
            .data_buffers()
            .len()
            .checked_add(1)
            .ok_or_else(|| {
                Error::IncompatibleSchema("BinaryView buffer count exceeds usize".to_owned())
            })?,
        ArrowDataType::Utf8View => downcast::<StringViewArray>(array.as_ref())?
            .data_buffers()
            .len()
            .checked_add(1)
            .ok_or_else(|| {
                Error::IncompatibleSchema("Utf8View buffer count exceeds usize".to_owned())
            })?,
        _ => 1,
    };
    reserve_vec_bytes::<arrow_buffer::Buffer>(budget, buffer_count)?;

    macro_rules! one_child {
        ($child:expr) => {{
            reserve_vec_bytes::<arrow_data::ArrayData>(budget, 1)?;
            reserve_to_data_scratch($child, budget)?;
        }};
    }
    match array.data_type() {
        ArrowDataType::List(_) => one_child!(downcast::<ListArray>(array.as_ref())?.values()),
        ArrowDataType::LargeList(_) => {
            one_child!(downcast::<LargeListArray>(array.as_ref())?.values());
        }
        ArrowDataType::ListView(_) => {
            one_child!(downcast::<ListViewArray>(array.as_ref())?.values());
        }
        ArrowDataType::LargeListView(_) => {
            one_child!(downcast::<LargeListViewArray>(array.as_ref())?.values());
        }
        ArrowDataType::FixedSizeList(_, _) => {
            one_child!(downcast::<FixedSizeListArray>(array.as_ref())?.values());
        }
        ArrowDataType::Struct(fields) => {
            reserve_vec_bytes::<arrow_data::ArrayData>(budget, fields.len())?;
            for child in downcast::<StructArray>(array.as_ref())?.columns() {
                reserve_to_data_scratch(child, budget)?;
            }
        }
        ArrowDataType::Map(_, _) => {
            reserve_vec_bytes::<arrow_data::ArrayData>(budget, 1)?;
            let entries: ArrayRef =
                Arc::new(downcast::<MapArray>(array.as_ref())?.entries().clone());
            reserve_to_data_scratch(&entries, budget)?;
        }
        ArrowDataType::Dictionary(key, _) => {
            reserve_vec_bytes::<arrow_data::ArrayData>(budget, 1)?;
            macro_rules! values {
                ($key:ty) => {
                    reserve_to_data_scratch(
                        downcast::<DictionaryArray<$key>>(array.as_ref())?.values(),
                        budget,
                    )?
                };
            }
            match key.as_ref() {
                ArrowDataType::Int8 => values!(Int8Type),
                ArrowDataType::Int16 => values!(Int16Type),
                ArrowDataType::Int32 => values!(Int32Type),
                ArrowDataType::Int64 => values!(Int64Type),
                ArrowDataType::UInt8 => values!(UInt8Type),
                ArrowDataType::UInt16 => values!(UInt16Type),
                ArrowDataType::UInt32 => values!(UInt32Type),
                ArrowDataType::UInt64 => values!(UInt64Type),
                _ => {
                    return Err(Error::IncompatibleSchema(
                        "dictionary key is not a supported integer".to_owned(),
                    ));
                }
            }
        }
        ArrowDataType::Union(fields, _) => {
            reserve_vec_bytes::<arrow_data::ArrayData>(budget, fields.len())?;
            let array = downcast::<UnionArray>(array.as_ref())?;
            for (type_id, _) in fields.iter() {
                reserve_to_data_scratch(array.child(type_id), budget)?;
            }
        }
        ArrowDataType::RunEndEncoded(run_ends, _) => {
            reserve_vec_bytes::<arrow_data::ArrayData>(budget, 2)?;
            reserve_vec_bytes::<arrow_buffer::Buffer>(budget, 1)?;
            match run_ends.data_type() {
                ArrowDataType::Int16 => {
                    reserve_to_data_scratch(
                        downcast::<Int16RunArray>(array.as_ref())?.values(),
                        budget,
                    )?;
                }
                ArrowDataType::Int32 => {
                    reserve_to_data_scratch(
                        downcast::<Int32RunArray>(array.as_ref())?.values(),
                        budget,
                    )?;
                }
                ArrowDataType::Int64 => {
                    reserve_to_data_scratch(
                        downcast::<Int64RunArray>(array.as_ref())?.values(),
                        budget,
                    )?;
                }
                _ => {
                    return Err(Error::IncompatibleSchema(
                        "run-end type is not a supported signed integer".to_owned(),
                    ));
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn reserve_field_default(
    field: &Field,
    rows: usize,
    budget: &mut MaterializationBudget,
) -> Result<()> {
    if field.is_nullable() {
        budget.add_null_array(field.dtype(), rows)
    } else {
        budget.add_repeated_default(field.dtype(), rows)
    }
}

pub(crate) fn reserve_field_default_scalar(
    field: &Field,
    budget: &mut MaterializationBudget,
) -> Result<()> {
    if field.is_nullable() {
        budget.add_null_scalar_scratch(field.dtype())
    } else {
        budget.add_default_scalar_scratch(field.dtype())
    }
}

pub(crate) fn reserve_missing_output(
    field: &Field,
    exposed: usize,
    hidden: usize,
    budget: &mut MaterializationBudget,
) -> Result<()> {
    reserve_field_default(field, exposed, budget)?;
    budget.add_null_array(field.dtype(), hidden)
}
