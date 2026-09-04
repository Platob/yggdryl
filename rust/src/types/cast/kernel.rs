//! Bounded execution of Arrow kernel casts.

use std::sync::Arc;

use arrow_array::{ArrayRef, BooleanArray, Scalar as ArrowScalar, UInt32Array};
use arrow_buffer::BooleanBuffer;
use arrow_cast::{CastOptions, cast_with_options};
use arrow_schema::DataType as ArrowDataType;
use arrow_select::{take::take, zip::zip};

use crate::arrow::{Error, Result};
use crate::types::budget::{
    MaterializationBudget, SourceSelection, reserve_cast_output_payload,
    reserve_new_dictionary_vocabularies, reserve_selected_source_take, reserve_source_selection,
    reserve_vec_bytes,
};
use crate::types::nested::casts::{align_nested_dictionaries, contains_dictionary, default_array};
use crate::{DataType, Field};

fn arrow_cast(array: &ArrayRef, expected: &ArrowDataType, safe: bool) -> Result<ArrayRef> {
    let options = CastOptions {
        safe,
        ..CastOptions::default()
    };
    cast_with_options(array.as_ref(), expected, &options).map_err(Into::into)
}

pub(crate) fn arrow_cast_exposed(
    array: &ArrayRef,
    expected: &ArrowDataType,
    safe: bool,
    exposure: Option<&BooleanBuffer>,
    target: &Field,
    budget: &mut MaterializationBudget,
) -> Result<ArrayRef> {
    let Some(exposure) = exposure else {
        if array.data_type() == expected {
            return Ok(Arc::clone(array));
        }
        budget.add_array(target.dtype(), array.len())?;
        let source_type = DataType::from_arrow(array.data_type())?;
        let full = [(0, array.len())];
        reserve_cast_output_payload(
            array.as_ref(),
            &source_type,
            target.dtype(),
            SourceSelection::Ranges(&full),
            budget,
        )?;
        return arrow_cast(array, expected, safe);
    };
    let selected_count = exposure.count_set_bits();
    if selected_count == 0 {
        return default_array(target, array.len(), Some(exposure), budget);
    }
    let phase = budget.mark();
    let output = (|| -> Result<ArrayRef> {
        // The masked kernel retains several arrays at once: selection/scatter
        // indices, a compact source, its compact cast, the scattered target, and
        // (for a required Field) the placeholder plus final zip output. Charge
        // every target-shaped buffer plus the selected source's actual physical
        // layout and copied payload before invoking Arrow's take kernel.
        budget.add_array(&DataType::UInt32, selected_count)?;
        budget.add_array(&DataType::UInt32, array.len())?;
        budget.add_array(target.dtype(), selected_count)?;
        budget.add_array(target.dtype(), array.len())?;
        if !target.is_nullable() {
            budget.add_array(target.dtype(), 1)?;
            budget.add_array(target.dtype(), array.len())?;
        }
        let mut selected = Vec::new();
        selected
            .try_reserve_exact(selected_count)
            .map_err(|error| {
                Error::IncompatibleSchema(format!(
                    "masked Arrow cast index allocation failed: {error}"
                ))
            })?;
        for index in 0..array.len() {
            if exposure.value(index) {
                let source = u32::try_from(index).map_err(|_| {
                    Error::IncompatibleSchema(
                        "masked Arrow cast source index exceeds UInt32".to_owned(),
                    )
                })?;
                selected.push(source);
            }
        }
        reserve_selected_source_take(
            array.as_ref(),
            &selected,
            target.dtype(),
            if target.is_nullable() { 2 } else { 3 },
            budget,
        )?;

        reserve_vec_bytes::<Option<u32>>(budget, array.len())?;
        let mut scatter = Vec::new();
        scatter.try_reserve_exact(array.len()).map_err(|error| {
            Error::IncompatibleSchema(format!(
                "masked Arrow cast scatter allocation failed: {error}"
            ))
        })?;
        let mut target_index = 0usize;
        for index in 0..array.len() {
            if exposure.value(index) {
                scatter.push(Some(u32::try_from(target_index).map_err(|_| {
                    Error::IncompatibleSchema(
                        "masked Arrow cast target index exceeds UInt32".to_owned(),
                    )
                })?));
                target_index += 1;
            } else {
                scatter.push(None);
            }
        }
        let compact = take(array.as_ref(), &UInt32Array::from(selected), None)?;
        let compact = arrow_cast(&compact, expected, safe)?;
        let scattered = take(compact.as_ref(), &UInt32Array::from(scatter), None)?;
        if target.is_nullable() {
            return Ok(scattered);
        }
        let mask = BooleanArray::new(exposure.clone(), None);
        let placeholder = crate::arrow::value::physical_placeholder_for_field(target)?;
        let placeholder = crate::arrow::value::array_from_values(target, &[&placeholder])?;
        let (scattered, placeholder) = if contains_dictionary(target.dtype()) {
            align_nested_dictionaries(
                target,
                &scattered,
                &placeholder,
                Some(exposure),
                None,
                budget,
            )?
        } else {
            (scattered, placeholder)
        };
        let placeholder = ArrowScalar::new(placeholder);
        zip(&mask, &scattered.as_ref(), &placeholder).map_err(Into::into)
    })()?;

    // Compact sources, index arrays, scatter output, and placeholders are
    // phase-local. Retain only the returned target array before sibling
    // columns continue against the shared operation budget.
    budget.restore(phase);
    let full = [(0, output.len())];
    reserve_source_selection(
        output.as_ref(),
        target.dtype(),
        SourceSelection::Ranges(&full),
        budget,
    )?;
    if contains_dictionary(target.dtype()) {
        reserve_new_dictionary_vocabularies(&output, array, target.dtype(), budget)?;
    }
    Ok(output)
}
