//! Reading a text column through one of this crate's own value readers.
//!
//! A text column entering a temporal or a decimal is read here rather than by
//! Arrow, because both families carry a precision the storage may not hold and
//! this crate refuses what it cannot state exactly where Arrow rounds it. The
//! reader is the same function a row value goes through, so a batch and a row
//! cannot answer differently about a spelling this crate knows. Arrow stays
//! behind the reading for the spellings only it takes, so a column still reads
//! everything it used to.

use std::sync::Arc;

use arrow_array::{Array, ArrayRef, BooleanArray, StringArray};
use arrow_buffer::{BooleanBuffer, BooleanBufferBuilder};
use arrow_cast::can_cast_types;
use arrow_schema::DataType as ArrowDataType;
use arrow_select::zip::zip;

use crate::arrow::{Error, Result};
use crate::types::budget::{MaterializationBudget, reserve_vec_bytes};
use crate::types::cast::{arrow_cast_exposed, downcast};
use crate::types::nested::casts::is_exposed;
use crate::{DataType, Field, Scalar};

/// One text cell read into the value a datatype declares.
///
/// [`crate::Error::Parse`] means the reader did not take the spelling at all,
/// so Arrow is allowed to try it; every other refusal is a spelling this crate
/// read and then rejected, and it stands.
pub(crate) type TextReader = fn(&DataType, &str) -> crate::Result<Scalar>;

/// The value a target holds, past whatever layout encodes it.
///
/// The encoding is a layout: the values read as the value they hold and the
/// tail encodes them, so a dictionary column reads like a plain one.
pub(crate) fn encoded_value_of(target: &DataType) -> &DataType {
    match target {
        DataType::Dictionary(dictionary) => encoded_value_of(dictionary.value()),
        DataType::RunEndEncoded(encoded) => encoded_value_of(encoded.values().dtype()),
        other => other,
    }
}

/// Whether a source layout holds text values, however it wraps them.
pub(crate) fn holds_text(source: &ArrowDataType) -> bool {
    match source {
        ArrowDataType::Utf8 | ArrowDataType::LargeUtf8 | ArrowDataType::Utf8View => true,
        ArrowDataType::Dictionary(_, values) => holds_text(values),
        ArrowDataType::RunEndEncoded(_, values) => holds_text(values.data_type()),
        _ => false,
    }
}

/// Reads a column of text through one of this crate's own value readers.
pub(crate) fn ingest_text_values(
    array: &ArrayRef,
    expected: &ArrowDataType,
    safe: bool,
    field: &Field,
    exposure: Option<&BooleanBuffer>,
    budget: &mut MaterializationBudget,
    reader: TextReader,
) -> Result<ArrayRef> {
    // The temporary is nullable text: this leaf owns the failures, and the
    // target's own null policy runs after the reading.
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
    let rows = source.len();
    let dtype = encoded_value_of(field.dtype());
    let read = Field::new(field.name(), dtype.clone(), true);
    budget.add_array(dtype, rows)?;
    reserve_vec_bytes::<Scalar>(budget, rows)?;
    reserve_vec_bytes::<&Scalar>(budget, rows)?;
    let mut values = Vec::with_capacity(rows);
    let mut ours = BooleanBufferBuilder::new(rows);
    let mut refused = false;
    for index in 0..rows {
        let cell = (is_exposed(exposure, index) && source.is_valid(index))
            .then(|| source.value(index))
            .map(|text| reader(dtype, text));
        match cell {
            // An absent value is this reading's own: nothing else reads it.
            None => {
                values.push(Scalar::Null);
                ours.append(true);
            }
            Some(Ok(value)) => {
                values.push(value);
                ours.append(true);
            }
            // A spelling this crate read and then refused - a count its unit
            // or width cannot hold exactly, a digit its scale would drop -
            // stays refused: Arrow would round it, and the row tier does not.
            Some(Err(error)) => {
                let unread = matches!(error, crate::Error::Parse { .. });
                values.push(Scalar::Null);
                ours.append(!unread);
                refused |= unread;
            }
        }
    }
    let mask = BooleanArray::new(ours.finish(), None);
    let read_here =
        crate::arrow::value::array_from_values(&read, &values.iter().collect::<Vec<_>>())?;
    let cast = if refused && can_cast_types(source.data_type(), expected) {
        // Arrow reads what this crate could not at its own risk: a value
        // neither reading takes stays null, and strict mode reports it below.
        // Arrow refuses a whole column whose target zone it cannot name, so
        // its failure leaves this crate's reading standing rather than
        // sinking it.
        match arrow_cast_exposed(&text, expected, true, exposure, &read, budget) {
            Ok(arrow) => zip(&mask, &read_here.as_ref(), &arrow.as_ref())?,
            Err(_) => read_here,
        }
    } else {
        read_here
    };
    if !safe {
        for index in 0..rows {
            let absent = !is_exposed(exposure, index) || source.is_null(index);
            if absent || !cast.is_null(index) {
                continue;
            }
            let cell = source.value(index);
            let reason = match reader(dtype, cell) {
                Err(crate::Error::InvalidRecord { reason, .. }) => reason.to_string(),
                Err(other) => other.to_string(),
                Ok(_) => String::new(),
            };
            return Err(Error::IncompatibleSchema(format!(
                "field {:?} row {index}: {cell:?} does not read as {dtype}: {reason}",
                field.name(),
            )));
        }
    }
    Ok(cast)
}
