//! Arrow casts owned by this datatype family.

use std::sync::Arc;

use arrow_array::{
    Array, ArrayRef, BinaryArray, FixedSizeBinaryArray, LargeStringArray, StringArray,
    StringViewArray,
};
use arrow_buffer::{BooleanBuffer, BooleanBufferBuilder};
use arrow_schema::DataType as ArrowDataType;

use crate::arrow::{Error, Result};
use crate::types::budget::{MaterializationBudget, reserve_vec_bytes};
use crate::types::cast::arrow_cast_exposed;
use crate::types::cast::{downcast, internal_target_error, named_cell};
use crate::types::nested::casts::is_exposed;
use crate::types::{ascii_free_text, ascii_padded, ascii_text, code_text};
use crate::{DataType, Field};

/// Validates every exposed, non-null value entering an ASCII datatype and
/// stores it as the target's own bytes.
///
/// A fixed binary of the target width is the same array once validated,
/// like WKB entering a geospatial column; another fixed width re-pads each
/// trimmed value; anything else first renders as Utf8 through Arrow's
/// kernel, so a dictionary or a view layout costs one temporary text array
/// on top of the fixed-width target. The variable form pads nothing and is
/// the same three sources under [`variable_ascii_array`].
pub(crate) fn ingest_ascii_array(
    array: &ArrayRef,
    expected: &ArrowDataType,
    safe: bool,
    field: &Field,
    exposure: Option<&BooleanBuffer>,
    budget: &mut MaterializationBudget,
) -> Result<ArrayRef> {
    let width = match *expected {
        ArrowDataType::FixedSizeBinary(width) => width,
        ArrowDataType::Binary => {
            return ingest_variable_ascii_array(array, safe, field, exposure, budget);
        }
        _ => return Err(internal_target_error("ascii")),
    };
    if let ArrowDataType::FixedSizeBinary(source_width) = array.data_type() {
        let source = downcast::<FixedSizeBinaryArray>(array.as_ref())?;
        if *source_width == width {
            for index in 0..source.len() {
                if is_exposed(exposure, index) && source.is_valid(index) {
                    ascii_cell(field, index, width, source.value(index))?;
                }
            }
            return Ok(Arc::clone(array));
        }
        return padded_ascii_array(field, width, source.len(), exposure, budget, |index| {
            source.is_valid(index).then(|| source.value(index))
        });
    }
    let text = if array.data_type() == &ArrowDataType::Utf8 {
        Arc::clone(array)
    } else {
        // The temporary is nullable text: the kernel's masked path fills
        // nothing, and the ASCII target's own null policy runs after padding.
        arrow_cast_exposed(
            array,
            &ArrowDataType::Utf8,
            safe,
            exposure,
            &Field::new(field.name(), DataType::Utf8, true),
            budget,
        )?
    };
    let source = downcast::<StringArray>(text.as_ref())?;
    padded_ascii_array(field, width, source.len(), exposure, budget, |index| {
        source
            .is_valid(index)
            .then(|| source.value(index).as_bytes())
    })
}
/// Builds the fixed storage of an ASCII width from one cell per row.
///
/// Unexposed rows are null: an ancestor hides them, so their bytes are
/// neither validated nor copied.
fn padded_ascii_array<'a>(
    field: &Field,
    width: i32,
    rows: usize,
    exposure: Option<&BooleanBuffer>,
    budget: &mut MaterializationBudget,
    cell: impl Fn(usize) -> Option<&'a [u8]>,
) -> Result<ArrayRef> {
    let slot = usize::try_from(width).map_err(|_| internal_target_error("ascii"))?;
    // The reservation bounds `rows * slot`, so the product cannot overflow.
    budget.add_array(field.dtype(), rows)?;
    let mut bytes = vec![0_u8; rows * slot];
    let mut validity = BooleanBufferBuilder::new(rows);
    for index in 0..rows {
        let value = cell(index).filter(|_| is_exposed(exposure, index));
        if let Some(raw) = value {
            let text = ascii_cell(field, index, width, raw)?;
            ascii_padded(&mut bytes[index * slot..][..slot], text);
        }
        validity.append(value.is_some());
    }
    let nulls = arrow_buffer::NullBuffer::new(validity.finish());
    Ok(Arc::new(FixedSizeBinaryArray::try_new(
        width,
        arrow_buffer::Buffer::from(bytes),
        (nulls.null_count() != 0).then_some(nulls),
    )?))
}

/// Validates every exposed, non-null value entering variable ASCII and stores
/// it as the bytes it is.
///
/// The same three sources as a width, minus the padding: a `Binary` column is
/// the same array once validated, a fixed width is trimmed of the NUL its
/// storage added, and anything else renders as Utf8 through Arrow's kernel
/// first.
fn ingest_variable_ascii_array(
    array: &ArrayRef,
    safe: bool,
    field: &Field,
    exposure: Option<&BooleanBuffer>,
    budget: &mut MaterializationBudget,
) -> Result<ArrayRef> {
    match array.data_type() {
        ArrowDataType::Binary => {
            let source = downcast::<BinaryArray>(array.as_ref())?;
            for index in 0..source.len() {
                if is_exposed(exposure, index) && source.is_valid(index) {
                    ascii_free_cell(field, index, source.value(index))?;
                }
            }
            return Ok(Arc::clone(array));
        }
        // The width's padding is storage, so a fixed cell is trimmed by the
        // width's own rule before it is stored as the bytes it is.
        ArrowDataType::FixedSizeBinary(width) => {
            let width = *width;
            let source = downcast::<FixedSizeBinaryArray>(array.as_ref())?;
            return variable_ascii_array(
                field,
                source.len(),
                exposure,
                budget,
                |index| match source.is_valid(index).then(|| source.value(index)) {
                    Some(padded) => ascii_cell(field, index, width, padded).map(Some),
                    None => Ok(None),
                },
            );
        }
        _ => {}
    }
    let text = if array.data_type() == &ArrowDataType::Utf8 {
        Arc::clone(array)
    } else {
        arrow_cast_exposed(
            array,
            &ArrowDataType::Utf8,
            safe,
            exposure,
            &Field::new(field.name(), DataType::Utf8, true),
            budget,
        )?
    };
    let source = downcast::<StringArray>(text.as_ref())?;
    variable_ascii_array(
        field,
        source.len(),
        exposure,
        budget,
        |index| match source.is_valid(index) {
            true => ascii_free_cell(field, index, source.value(index).as_bytes()).map(Some),
            false => Ok(None),
        },
    )
}

/// Builds the variable storage of ASCII text from one cell per row.
///
/// Unexposed rows are null: an ancestor hides them, so their bytes are
/// neither validated nor copied. A fixed-width cell arrives padded and is
/// stored trimmed, which is the only shortening this does.
fn variable_ascii_array<'a>(
    field: &Field,
    rows: usize,
    exposure: Option<&BooleanBuffer>,
    budget: &mut MaterializationBudget,
    cell: impl Fn(usize) -> Result<Option<&'a str>>,
) -> Result<ArrayRef> {
    budget.add_array(field.dtype(), rows)?;
    reserve_vec_bytes::<Option<&str>>(budget, rows)?;
    let mut values = Vec::new();
    values.try_reserve_exact(rows).map_err(|error| {
        Error::IncompatibleSchema(format!("ASCII output allocation failed: {error}"))
    })?;
    let mut payload = 0usize;
    for index in 0..rows {
        let text = if is_exposed(exposure, index) {
            cell(index)?
        } else {
            None
        };
        payload = payload.saturating_add(text.map_or(0, str::len));
        values.push(text);
    }
    budget.add_bytes(payload)?;
    Ok(Arc::new(
        values
            .into_iter()
            .map(|text| text.map(str::as_bytes))
            .collect::<BinaryArray>(),
    ))
}

/// Renders a recognized ASCII column as trimmed text.
///
/// Storage pads with NUL and every string rendering trims, so the text
/// payload is bounded by the fixed payload: the reservation charges the
/// target rows plus that bound, and a view target its one largest value.
pub(crate) fn render_ascii_text(
    array: &ArrayRef,
    expected: &ArrowDataType,
    field: &Field,
    exposure: Option<&BooleanBuffer>,
    budget: &mut MaterializationBudget,
) -> Result<ArrayRef> {
    // The variable form stores no padding, so its own bytes are the text and
    // the payload bound is the source buffer itself.
    if let ArrowDataType::Binary = array.data_type() {
        let source = downcast::<BinaryArray>(array.as_ref())?;
        return render_ascii_cells(
            expected,
            field,
            source.len(),
            source.value_data().len(),
            budget,
            |index| match is_exposed(exposure, index) && source.is_valid(index) {
                true => ascii_free_cell(field, index, source.value(index)).map(Some),
                false => Ok(None),
            },
        );
    }
    let source = downcast::<FixedSizeBinaryArray>(array.as_ref())?;
    let width = source.value_length();
    let slot = usize::try_from(width).map_err(|_| internal_target_error("ascii text"))?;
    render_ascii_cells(
        expected,
        field,
        source.len(),
        source.len().saturating_mul(slot),
        budget,
        |index| match is_exposed(exposure, index) && source.is_valid(index) {
            true => ascii_cell(field, index, width, source.value(index)).map(Some),
            false => Ok(None),
        },
    )
}

/// Collects one ASCII cell per row into the text layout the target names.
fn render_ascii_cells<'a>(
    expected: &ArrowDataType,
    field: &Field,
    rows: usize,
    payload: usize,
    budget: &mut MaterializationBudget,
    cell: impl Fn(usize) -> Result<Option<&'a str>>,
) -> Result<ArrayRef> {
    budget.add_array(field.dtype(), rows)?;
    budget.add_bytes(payload)?;
    if matches!(expected, ArrowDataType::Utf8View) {
        budget.add_bytes(payload)?;
    }
    reserve_vec_bytes::<Option<&str>>(budget, rows)?;
    let mut rendered = Vec::new();
    rendered.try_reserve_exact(rows).map_err(|error| {
        Error::IncompatibleSchema(format!("ASCII text output allocation failed: {error}"))
    })?;
    for index in 0..rows {
        rendered.push(cell(index)?);
    }
    Ok(match expected {
        ArrowDataType::Utf8 => Arc::new(rendered.into_iter().collect::<StringArray>()) as ArrayRef,
        ArrowDataType::LargeUtf8 => {
            Arc::new(rendered.into_iter().collect::<LargeStringArray>()) as ArrayRef
        }
        ArrowDataType::Utf8View => {
            Arc::new(rendered.into_iter().collect::<StringViewArray>()) as ArrayRef
        }
        _ => return Err(internal_target_error("ascii text")),
    })
}

/// Validates every exposed, non-null value entering a registered code and
/// pads it into that code's fixed storage.
///
/// The same shape as [`ingest_ascii_array`] with the width a constant: a
/// fixed binary of the code's own width is the same array once validated,
/// another fixed width re-pads each trimmed value, and anything else first
/// renders as Utf8 through Arrow's kernel. What the constant buys is the
/// inner loop: the length check, the slot arithmetic and the padding copy
/// are all fixed-size, so a currency column ingests three bytes a row with
/// no width to read.
pub(crate) fn ingest_code_array<const WIDTH: usize>(
    array: &ArrayRef,
    safe: bool,
    field: &Field,
    exposure: Option<&BooleanBuffer>,
    budget: &mut MaterializationBudget,
) -> Result<ArrayRef> {
    if let ArrowDataType::FixedSizeBinary(source_width) = array.data_type() {
        let source = downcast::<FixedSizeBinaryArray>(array.as_ref())?;
        if usize::try_from(*source_width).is_ok_and(|width| width == WIDTH) {
            for index in 0..source.len() {
                if is_exposed(exposure, index) && source.is_valid(index) {
                    code_cell::<WIDTH>(field, index, source.value(index))?;
                }
            }
            return Ok(Arc::clone(array));
        }
        return padded_code_array::<WIDTH>(field, source.len(), exposure, budget, |index| {
            source.is_valid(index).then(|| source.value(index))
        });
    }
    let text = if array.data_type() == &ArrowDataType::Utf8 {
        Arc::clone(array)
    } else {
        arrow_cast_exposed(
            array,
            &ArrowDataType::Utf8,
            safe,
            exposure,
            &Field::new(field.name(), DataType::Utf8, true),
            budget,
        )?
    };
    let source = downcast::<StringArray>(text.as_ref())?;
    padded_code_array::<WIDTH>(field, source.len(), exposure, budget, |index| {
        source
            .is_valid(index)
            .then(|| source.value(index).as_bytes())
    })
}

/// Builds the fixed storage of one registered code from one cell per row.
fn padded_code_array<'a, const WIDTH: usize>(
    field: &Field,
    rows: usize,
    exposure: Option<&BooleanBuffer>,
    budget: &mut MaterializationBudget,
    cell: impl Fn(usize) -> Option<&'a [u8]>,
) -> Result<ArrayRef> {
    // The reservation bounds `rows * WIDTH`, so the product cannot overflow.
    budget.add_array(field.dtype(), rows)?;
    let mut bytes = vec![0_u8; rows * WIDTH];
    let mut validity = BooleanBufferBuilder::new(rows);
    for (index, slot) in bytes.chunks_exact_mut(WIDTH).enumerate() {
        let value = cell(index).filter(|_| is_exposed(exposure, index));
        if let Some(raw) = value {
            let text = code_cell::<WIDTH>(field, index, raw)?;
            slot[..text.len()].copy_from_slice(text.as_bytes());
        }
        validity.append(value.is_some());
    }
    let nulls = arrow_buffer::NullBuffer::new(validity.finish());
    Ok(Arc::new(FixedSizeBinaryArray::try_new(
        WIDTH as i32,
        arrow_buffer::Buffer::from(bytes),
        (nulls.null_count() != 0).then_some(nulls),
    )?))
}

/// Validates one code cell at the code's constant width.
fn code_cell<'a, const WIDTH: usize>(
    field: &Field,
    index: usize,
    bytes: &'a [u8],
) -> Result<&'a str> {
    code_text::<WIDTH>(bytes).map_err(|error| {
        Error::IncompatibleSchema(format!(
            "row {index} of column {name}: {error}",
            name = field.name()
        ))
    })
}
/// Validates one cell under an ASCII width, naming the field and the row
/// beside the width the rule itself names.
fn ascii_cell<'a>(field: &Field, index: usize, width: i32, bytes: &'a [u8]) -> Result<&'a str> {
    named_cell(field, index, ascii_text(width, bytes))
}

/// One variable ASCII cell: the same value rule, with no width to fit.
fn ascii_free_cell<'a>(field: &Field, index: usize, bytes: &'a [u8]) -> Result<&'a str> {
    named_cell(field, index, ascii_free_text(bytes))
}
