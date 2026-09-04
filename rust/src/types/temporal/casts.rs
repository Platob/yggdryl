//! Arrow casts owned by this datatype family.

use std::sync::Arc;

use arrow_array::{Array, ArrayRef, BooleanArray, StringArray};
use arrow_buffer::{BooleanBuffer, BooleanBufferBuilder};
use arrow_cast::can_cast_types;
use arrow_schema::DataType as ArrowDataType;
use arrow_select::zip::zip;

use crate::arrow::{Error, Result};
use crate::types::budget::{MaterializationBudget, reserve_vec_bytes};
use crate::types::cast::arrow_cast_exposed;
use crate::types::cast::downcast;
use crate::types::nested::casts::is_exposed;
use crate::{DataType, Field, Scalar};

/// Whether a source Arrow type holds temporals with a classic spelling.
///
/// The interval layouts are temporal too and have no such spelling, so they
/// keep Arrow's rendering.
pub(crate) fn is_temporal_arrow(source: &ArrowDataType) -> bool {
    match source {
        ArrowDataType::Date32
        | ArrowDataType::Date64
        | ArrowDataType::Time32(_)
        | ArrowDataType::Time64(_)
        | ArrowDataType::Timestamp(..)
        | ArrowDataType::Duration(_) => true,
        ArrowDataType::Dictionary(_, values) => is_temporal_arrow(values),
        ArrowDataType::RunEndEncoded(_, values) => is_temporal_arrow(values.data_type()),
        _ => false,
    }
}

/// Renders a temporal column as the classic text this crate spells.
///
/// [`Scalar::into_temporal_text`] is what an expression literal and the row
/// evaluator spell with, and it reads the zone rules this crate owns, so a
/// zoned instant renders here where Arrow's formatter refuses one. A value
/// with no classic spelling keeps Arrow's rendering.
pub(crate) fn render_temporal_text(
    array: &ArrayRef,
    safe: bool,
    field: &Field,
    exposure: Option<&BooleanBuffer>,
    budget: &mut MaterializationBudget,
) -> Result<ArrayRef> {
    let source_type = DataType::from_arrow(array.data_type())?;
    let rows = array.len();
    budget.add_array(field.dtype(), rows)?;
    reserve_vec_bytes::<Option<smol_str::SmolStr>>(budget, rows)?;
    let mut spelled = Vec::with_capacity(rows);
    let mut ours = BooleanBufferBuilder::new(rows);
    let mut unspelled = false;
    for index in 0..rows {
        let text = if is_exposed(exposure, index) && array.is_valid(index) {
            crate::arrow::value::value_from_array(&source_type, array.as_ref(), index)?
                .into_temporal_text()
        } else {
            None
        };
        let absent = !is_exposed(exposure, index) || array.is_null(index);
        ours.append(absent || text.is_some());
        unspelled |= !absent && text.is_none();
        spelled.push(text);
    }
    // The reservation above charges the offsets a text array carries; the
    // spellings themselves are the payload this loop built.
    budget.add_bytes(spelled.iter().flatten().map(smol_str::SmolStr::len).sum())?;
    let mask = BooleanArray::new(ours.finish(), None);
    let read_here: ArrayRef = Arc::new(StringArray::from_iter(
        spelled
            .iter()
            .map(|text| text.as_ref().map(smol_str::SmolStr::as_str)),
    ));
    if !unspelled {
        return Ok(read_here);
    }
    // Arrow's formatter keeps the readings this crate has no spelling for,
    // such as a date outside four-digit years; where it has none either, this
    // crate's nulls stand.
    let cast = if can_cast_types(array.data_type(), &ArrowDataType::Utf8) {
        let rendered = arrow_cast_exposed(
            array,
            &ArrowDataType::Utf8,
            true,
            exposure,
            &Field::new(field.name(), DataType::Utf8, true),
            budget,
        );
        match rendered {
            Ok(arrow) => zip(&mask, &read_here.as_ref(), &arrow.as_ref())?,
            Err(_) => read_here,
        }
    } else {
        read_here
    };
    if !safe {
        for index in 0..rows {
            if !mask.value(index) && cast.is_null(index) {
                return Err(Error::IncompatibleSchema(format!(
                    "field {:?} row {index}: this {source_type} reading has no classic spelling",
                    field.name(),
                )));
            }
        }
    }
    Ok(cast)
}

/// Whether a target datatype holds temporals, however it encodes them.
pub(crate) fn holds_temporal(target: &DataType) -> bool {
    match target {
        DataType::Date32
        | DataType::Date64
        | DataType::Time32(_)
        | DataType::Time64(_)
        | DataType::DateTime64 { .. }
        | DataType::Duration32(_)
        | DataType::Duration64(_) => true,
        DataType::Dictionary(dictionary) => holds_temporal(dictionary.value()),
        DataType::RunEndEncoded(encoded) => holds_temporal(encoded.values().dtype()),
        _ => false,
    }
}

/// The temporal a target holds, past whatever layout encodes it.
fn temporal_of(target: &DataType) -> &DataType {
    match target {
        DataType::Dictionary(dictionary) => temporal_of(dictionary.value()),
        DataType::RunEndEncoded(encoded) => temporal_of(encoded.values().dtype()),
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

/// Reads a column of temporal text through this crate's own spellings.
///
/// [`Scalar::from_temporal_text`] is what the row evaluator reads one value
/// with, so a batch and a row cannot answer differently about a spelling this
/// crate knows - its refusals included: a reading this crate takes but its
/// declared unit or width cannot hold is null here as it is there, never
/// Arrow's rounded one. Arrow's kernel answers only what this crate cannot
/// read at all - a bare date entering a timestamp, a twelve-hour clock, a
/// compact `YYYYMMDD` - so the column still reads everything it used to.
pub(crate) fn ingest_temporal_text(
    array: &ArrayRef,
    expected: &ArrowDataType,
    safe: bool,
    field: &Field,
    exposure: Option<&BooleanBuffer>,
    budget: &mut MaterializationBudget,
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
            &Field::new(field.name(), DataType::Utf8, true),
            budget,
        )?
    };
    let source = downcast::<StringArray>(text.as_ref())?;
    let rows = source.len();
    // The encoding is a layout: the values read as the temporal they hold and
    // the tail encodes them, so a dictionary column reads like a plain one.
    let dtype = temporal_of(field.dtype());
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
            .map(|text| Scalar::from_temporal_text(dtype, text));
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
            // or width cannot hold exactly - stays refused: Arrow would round
            // it, and the row tier does not.
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
            let reason = match Scalar::from_temporal_text(dtype, cell) {
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
