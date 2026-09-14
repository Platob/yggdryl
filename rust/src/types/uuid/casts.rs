//! Arrow casts owned by this datatype family.

use std::sync::Arc;

use crate::arrow::{Error, Result};
use crate::types::budget::MaterializationBudget;
use crate::types::bytes::casts::variable_binary_source;
use crate::types::cast::arrow_cast_exposed;
use crate::types::cast::{downcast, internal_target_error};
use crate::types::nested::casts::is_exposed;
use crate::types::uuid_parse;
use crate::{DataType, Field};
use arrow_array::{Array, ArrayRef, BinaryArray, FixedSizeBinaryArray, StringArray};
use arrow_buffer::{BooleanBuffer, BooleanBufferBuilder};
use arrow_schema::DataType as ArrowDataType;

/// Validates every exposed, non-null value entering a UUID and stores it as
/// its sixteen bytes.
///
/// Sixteen-byte storage is the same array once validated. Any other fixed
/// width is a slot holding one of the two text spellings, so the padding is
/// taken off and the spelling read; variable bytes are read as they are; and
/// anything else first renders as Utf8 through Arrow's kernel, exactly as an
/// ASCII width does.
pub(crate) fn ingest_uuid_array(
    array: &ArrayRef,
    expected: &ArrowDataType,
    safe: bool,
    field: &Field,
    exposure: Option<&BooleanBuffer>,
    budget: &mut MaterializationBudget,
) -> Result<ArrayRef> {
    if !matches!(expected, ArrowDataType::FixedSizeBinary(16)) {
        return Err(internal_target_error("uuid"));
    }
    if let ArrowDataType::FixedSizeBinary(width) = array.data_type() {
        let source = downcast::<FixedSizeBinaryArray>(array.as_ref())?;
        if *width == 16 {
            for index in 0..source.len() {
                if is_exposed(exposure, index) && source.is_valid(index) {
                    uuid_cell(field, index, source.value(index))?;
                }
            }
            return Ok(Arc::clone(array));
        }
        // Sixteen bytes are an identifier, in which every byte carries
        // identity and a trailing NUL is one of them. Any other width is a
        // text slot, so its trailing NUL is the slot's padding.
        return uuid_storage(field, source.len(), exposure, budget, |index| {
            source
                .is_valid(index)
                .then(|| crate::types::trim_padding(source.value(index)))
        });
    }
    if let Some(bytes) = variable_binary_source(array, field, exposure, budget)? {
        let source = downcast::<BinaryArray>(bytes.as_ref())?;
        return uuid_storage(field, source.len(), exposure, budget, |index| {
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
            &Field::new(field.name(), DataType::utf8(), true),
            budget,
        )?
    };
    let source = downcast::<StringArray>(text.as_ref())?;
    uuid_storage(field, source.len(), exposure, budget, |index| {
        source
            .is_valid(index)
            .then(|| source.value(index).as_bytes())
    })
}

/// Builds the sixteen stored bytes of a UUID from one cell per row.
///
/// The cell is whatever spelling the source carried - the hyphenated text, the
/// bare hex, or the sixteen bytes themselves - and the one UUID rule reads all
/// three, so the reading is stated once for every source layout.
fn uuid_storage<'a>(
    field: &Field,
    rows: usize,
    exposure: Option<&BooleanBuffer>,
    budget: &mut MaterializationBudget,
    cell: impl Fn(usize) -> Option<&'a [u8]>,
) -> Result<ArrayRef> {
    budget.add_array(field.dtype(), rows)?;
    let mut bytes = vec![0_u8; rows * 16];
    let mut validity = BooleanBufferBuilder::new(rows);
    for index in 0..rows {
        let present = is_exposed(exposure, index).then(|| cell(index)).flatten();
        if let Some(value) = present {
            bytes[index * 16..][..16].copy_from_slice(&uuid_cell(field, index, value)?);
        }
        validity.append(present.is_some());
    }
    let nulls = arrow_buffer::NullBuffer::new(validity.finish());
    Ok(Arc::new(FixedSizeBinaryArray::try_new(
        16,
        arrow_buffer::Buffer::from(bytes),
        (nulls.null_count() != 0).then_some(nulls),
    )?))
}

/// Validates one cell as a UUID, naming the field and the row beside the rule.
fn uuid_cell(field: &Field, index: usize, bytes: &[u8]) -> Result<[u8; 16]> {
    uuid_parse(bytes).map_err(|error| {
        let reason = match error {
            crate::Error::InvalidRecord { reason, .. } => reason.to_string(),
            other => other.to_string(),
        };
        Error::IncompatibleSchema(format!("column {:?} row {index}: {reason}", field.name()))
    })
}
