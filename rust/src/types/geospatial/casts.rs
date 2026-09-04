//! Arrow casts owned by this datatype family.

use std::sync::Arc;

use arrow_array::{Array, ArrayRef, BinaryArray, LargeStringArray, StringArray, StringViewArray};
use arrow_buffer::BooleanBuffer;
use arrow_schema::DataType as ArrowDataType;

use super::wkb;
use crate::Field;
use crate::arrow::{Error, Result};
use crate::types::budget::{MaterializationBudget, reserve_vec_bytes};
use crate::types::cast::{downcast, internal_target_error};
use crate::types::nested::casts::is_exposed;

/// Validates every exposed, non-null payload of a Binary array as WKB on its
/// way into a geospatial field, naming the field, the row, and the byte
/// position the reader stopped at.
///
/// The streaming bounds scan walks the whole payload without materializing a
/// geometry, so validation holds nothing per row.
pub(crate) fn validate_wkb_ingest(
    array: &dyn Array,
    field: &Field,
    exposure: Option<&BooleanBuffer>,
) -> Result<()> {
    let source = downcast::<BinaryArray>(array)?;
    for index in 0..source.len() {
        if !is_exposed(exposure, index) || source.is_null(index) {
            continue;
        }
        wkb::bounding_box(source.value(index)).map_err(|error| {
            Error::IncompatibleSchema(format!(
                "field {:?} row {index}: expected WKB bytes for a {} value, got {error}",
                field.name(),
                field.dtype().name(),
            ))
        })?;
    }
    Ok(())
}

/// Renders a recognized geospatial Binary column as WKT text.
///
/// Two passes keep the materialization honest: the first parses and renders
/// each exposed value once to count the exact output payload against the
/// budget - holding one rendered value at a time - and the second builds the
/// target text array under that reservation.
pub(crate) fn render_wkt_array(
    array: &ArrayRef,
    expected: &ArrowDataType,
    field: &Field,
    exposure: Option<&BooleanBuffer>,
    budget: &mut MaterializationBudget,
) -> Result<ArrayRef> {
    let source = downcast::<BinaryArray>(array.as_ref())?;
    let mut total = 0usize;
    let mut maximum = 0usize;
    for index in 0..source.len() {
        if !is_exposed(exposure, index) || source.is_null(index) {
            continue;
        }
        let text = wkt_for_cell(field, index, source.value(index))?;
        total = total.checked_add(text.len()).ok_or_else(|| {
            Error::IncompatibleSchema("WKT output payload exceeds usize".to_owned())
        })?;
        maximum = maximum.max(text.len());
    }
    budget.add_array(field.dtype(), source.len())?;
    budget.add_bytes(total)?;
    // View outputs keep the largest logical value alive while the final view
    // buffers are appended.
    if matches!(expected, ArrowDataType::Utf8View) {
        budget.add_bytes(maximum)?;
    }
    let mut rendered: Vec<Option<String>> = Vec::new();
    reserve_vec_bytes::<Option<String>>(budget, source.len())?;
    rendered.try_reserve_exact(source.len()).map_err(|error| {
        Error::IncompatibleSchema(format!("WKT output allocation failed: {error}"))
    })?;
    for index in 0..source.len() {
        if !is_exposed(exposure, index) || source.is_null(index) {
            rendered.push(None);
        } else {
            rendered.push(Some(wkt_for_cell(field, index, source.value(index))?));
        }
    }
    Ok(match expected {
        ArrowDataType::Utf8 => Arc::new(rendered.into_iter().collect::<StringArray>()) as ArrayRef,
        ArrowDataType::LargeUtf8 => {
            Arc::new(rendered.into_iter().collect::<LargeStringArray>()) as ArrayRef
        }
        ArrowDataType::Utf8View => {
            Arc::new(rendered.into_iter().collect::<StringViewArray>()) as ArrayRef
        }
        _ => return Err(internal_target_error("geospatial WKT")),
    })
}

/// Renders one WKB cell as WKT, naming the field and row when the bytes are
/// not one well-formed geometry.
fn wkt_for_cell(field: &Field, index: usize, bytes: &[u8]) -> Result<String> {
    wkb::into_wkt(bytes).map_err(|error| {
        Error::IncompatibleSchema(format!(
            "field {:?} row {index}: expected WKB bytes to render as WKT, got {error}",
            field.name(),
        ))
    })
}
