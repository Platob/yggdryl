//! Arrow casts owned by this datatype family.

use std::sync::Arc;

use arrow_array::{
    Array, ArrayRef, FixedSizeBinaryArray, LargeStringArray, StringArray, StringViewArray,
};
use arrow_buffer::{BooleanBuffer, BooleanBufferBuilder};
use arrow_schema::DataType as ArrowDataType;
use smol_str::SmolStr;

use crate::arrow::{Error, Result};
use crate::types::budget::{MaterializationBudget, reserve_vec_bytes};
use crate::types::cast::arrow_cast_exposed;
use crate::types::cast::{downcast, internal_target_error};
use crate::types::nested::casts::is_exposed;
use crate::types::{guid_parse, guid_text};
use crate::{DataType, Field};

/// Validates every exposed, non-null value entering a GUID and stores it as
/// its sixteen bytes.
///
/// Sixteen-byte storage is the same array once validated; anything else first
/// renders as Utf8 through Arrow's kernel, exactly as an ASCII width does.
pub(crate) fn ingest_guid_array(
    array: &ArrayRef,
    expected: &ArrowDataType,
    safe: bool,
    field: &Field,
    exposure: Option<&BooleanBuffer>,
    budget: &mut MaterializationBudget,
) -> Result<ArrayRef> {
    if !matches!(expected, ArrowDataType::FixedSizeBinary(16)) {
        return Err(internal_target_error("guid"));
    }
    if let ArrowDataType::FixedSizeBinary(16) = array.data_type() {
        let source = downcast::<FixedSizeBinaryArray>(array.as_ref())?;
        for index in 0..source.len() {
            if is_exposed(exposure, index) && source.is_valid(index) {
                guid_cell(field, index, source.value(index))?;
            }
        }
        return Ok(Arc::clone(array));
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
    budget.add_array(field.dtype(), source.len())?;
    let mut bytes = vec![0_u8; source.len() * 16];
    let mut validity = BooleanBufferBuilder::new(source.len());
    for index in 0..source.len() {
        let present = is_exposed(exposure, index) && source.is_valid(index);
        if present {
            let stored = guid_cell(field, index, source.value(index).as_bytes())?;
            bytes[index * 16..][..16].copy_from_slice(&stored);
        }
        validity.append(present);
    }
    let nulls = arrow_buffer::NullBuffer::new(validity.finish());
    Ok(Arc::new(FixedSizeBinaryArray::try_new(
        16,
        arrow_buffer::Buffer::from(bytes),
        (nulls.null_count() != 0).then_some(nulls),
    )?))
}

/// Renders a recognized GUID column as its hyphenated spelling.
pub(crate) fn render_guid_text(
    array: &ArrayRef,
    expected: &ArrowDataType,
    field: &Field,
    exposure: Option<&BooleanBuffer>,
    budget: &mut MaterializationBudget,
) -> Result<ArrayRef> {
    let source = downcast::<FixedSizeBinaryArray>(array.as_ref())?;
    budget.add_array(field.dtype(), source.len())?;
    budget.add_bytes(source.len().saturating_mul(36))?;
    reserve_vec_bytes::<Option<SmolStr>>(budget, source.len())?;
    let mut rendered = Vec::new();
    rendered.try_reserve_exact(source.len()).map_err(|error| {
        Error::IncompatibleSchema(format!("GUID text output allocation failed: {error}"))
    })?;
    for index in 0..source.len() {
        rendered.push(if is_exposed(exposure, index) && source.is_valid(index) {
            Some(guid_text(&guid_cell(field, index, source.value(index))?))
        } else {
            None
        });
    }
    let text = rendered.iter().map(|value| value.as_deref());
    Ok(match expected {
        ArrowDataType::Utf8 => Arc::new(text.collect::<StringArray>()) as ArrayRef,
        ArrowDataType::LargeUtf8 => Arc::new(text.collect::<LargeStringArray>()) as ArrayRef,
        ArrowDataType::Utf8View => Arc::new(text.collect::<StringViewArray>()) as ArrayRef,
        _ => return Err(internal_target_error("guid text")),
    })
}

/// Validates one cell as a GUID, naming the field and the row beside the rule.
fn guid_cell(field: &Field, index: usize, bytes: &[u8]) -> Result<[u8; 16]> {
    guid_parse(bytes).map_err(|error| {
        let reason = match error {
            crate::Error::InvalidRecord { reason, .. } => reason.to_string(),
            other => other.to_string(),
        };
        Error::IncompatibleSchema(format!("column {:?} row {index}: {reason}", field.name()))
    })
}
