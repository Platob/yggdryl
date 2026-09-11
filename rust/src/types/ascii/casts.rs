//! Arrow casts owned by this datatype family.

use std::sync::Arc;

use arrow_array::{Array, ArrayRef, BinaryArray, FixedSizeBinaryArray, StringArray};
use arrow_buffer::BooleanBuffer;
use arrow_schema::DataType as ArrowDataType;

use crate::arrow::{Error, Result};
use crate::types::budget::{MaterializationBudget, reserve_vec_bytes};
use crate::types::bytes::casts::variable_binary_source;
use crate::types::cast::arrow_cast_exposed;
use crate::types::cast::{downcast, internal_target_error, named_cell};
use crate::types::nested::casts::is_exposed;
use crate::types::{ascii_text, ascii_value_text, code_text, code_value_text};
use crate::{DataType, Field};

/// Validates every exposed, non-null value entering an ASCII datatype and
/// stores it as the text it is.
///
/// Every shape in this family stores `Utf8`, so a text source that passes the
/// value rule *is* the target column: the cast validates and hands back the
/// buffers it was given, which is the whole of the common case. A byte source
/// is padded storage instead, so each cell is read through the width's rule -
/// trailing NUL trimmed - and the column is rebuilt as the text it holds.
/// Anything else renders as `Utf8` through Arrow's kernel first.
pub(crate) fn ingest_ascii_array(
    array: &ArrayRef,
    safe: bool,
    field: &Field,
    exposure: Option<&BooleanBuffer>,
    budget: &mut MaterializationBudget,
) -> Result<ArrayRef> {
    let width = field.dtype().ascii_width();
    if let Some(bytes) = padded_source(array, field, exposure, budget)? {
        return stored_text(field, &bytes, exposure, budget, |index, padded| {
            named_cell(field, index, ascii_padded_text(width, padded))
        });
    }
    let bound = ascii_bound(width)?;
    validated_text(array, safe, field, exposure, budget, |index, text| {
        named_cell(field, index, ascii_value_text(bound, text.as_bytes()))
    })
}

/// [`ingest_ascii_array`] at one code's constant width.
///
/// The same three sources and the same answers; what the constant buys is the
/// inner loop, where the length check folds and no row reads a width out of
/// the datatype. A securities number carries one check beyond its width, so
/// the code path is where that check runs.
pub(crate) fn ingest_code_array<const WIDTH: usize>(
    array: &ArrayRef,
    safe: bool,
    field: &Field,
    exposure: Option<&BooleanBuffer>,
    budget: &mut MaterializationBudget,
) -> Result<ArrayRef> {
    let canonical = matches!(field.dtype(), DataType::Isin);
    if let Some(bytes) = padded_source(array, field, exposure, budget)? {
        return stored_text(field, &bytes, exposure, budget, |index, padded| {
            let text = named_cell(field, index, code_text::<WIDTH>(padded))?;
            canonical_cell(canonical, field, index, text)
        });
    }
    validated_text(array, safe, field, exposure, budget, |index, text| {
        let text = named_cell(field, index, code_value_text::<WIDTH>(text.as_bytes()))?;
        canonical_cell(canonical, field, index, text)
    })
}

/// The padded byte source one column arrived as, `None` when it is text.
///
/// `FixedSizeBinary` pads by construction and `Binary` may carry the padding
/// it was written with, so both are read through the width's own rule. Every
/// other layout is text, and text carries no padding.
fn padded_source(
    array: &ArrayRef,
    field: &Field,
    exposure: Option<&BooleanBuffer>,
    budget: &mut MaterializationBudget,
) -> Result<Option<ArrayRef>> {
    match array.data_type() {
        ArrowDataType::FixedSizeBinary(_) => Ok(Some(Arc::clone(array))),
        _ => variable_binary_source(array, field, exposure, budget),
    }
}

/// Rebuilds one padded byte column as the trimmed text its cells hold.
fn stored_text(
    field: &Field,
    bytes: &ArrayRef,
    exposure: Option<&BooleanBuffer>,
    budget: &mut MaterializationBudget,
    cell: impl for<'a> Fn(usize, &'a [u8]) -> Result<&'a str>,
) -> Result<ArrayRef> {
    let fixed = match bytes.data_type() {
        ArrowDataType::FixedSizeBinary(_) => {
            Some(downcast::<FixedSizeBinaryArray>(bytes.as_ref())?)
        }
        _ => None,
    };
    let variable = match fixed {
        Some(_) => None,
        None => Some(downcast::<BinaryArray>(bytes.as_ref())?),
    };
    let rows = bytes.len();
    text_array(field, rows, budget, |index| {
        let raw = match (fixed, variable) {
            (Some(source), _) => source.is_valid(index).then(|| source.value(index)),
            (_, Some(source)) => source.is_valid(index).then(|| source.value(index)),
            _ => None,
        };
        match raw.filter(|_| is_exposed(exposure, index)) {
            Some(raw) => cell(index, raw).map(Some),
            None => Ok(None),
        }
    })
}

/// Validates a text column in place and hands back the array it already is.
///
/// The target storage is the source storage, so a column that passes keeps
/// its offsets and its bytes: nothing is copied and nothing is rebuilt.
fn validated_text(
    array: &ArrayRef,
    safe: bool,
    field: &Field,
    exposure: Option<&BooleanBuffer>,
    budget: &mut MaterializationBudget,
    cell: impl for<'a> Fn(usize, &'a str) -> Result<&'a str>,
) -> Result<ArrayRef> {
    let text = if array.data_type() == &ArrowDataType::Utf8 {
        Arc::clone(array)
    } else {
        // The temporary is nullable text: the kernel's masked path fills
        // nothing, and the ASCII target's own null policy runs after this.
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
    for index in 0..source.len() {
        if is_exposed(exposure, index) && source.is_valid(index) {
            cell(index, source.value(index))?;
        }
    }
    Ok(text)
}

/// Builds the `Utf8` storage of an ASCII column from one cell per row.
///
/// Unexposed rows are null: an ancestor hides them, so their bytes are
/// neither validated nor copied.
fn text_array<'a>(
    field: &Field,
    rows: usize,
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
        let text = cell(index)?;
        payload = payload.saturating_add(text.map_or(0, str::len));
        values.push(text);
    }
    budget.add_bytes(payload)?;
    Ok(Arc::new(values.into_iter().collect::<StringArray>()))
}

/// The declared width as the bound the value rule takes.
fn ascii_bound(width: Option<i32>) -> Result<Option<usize>> {
    width
        .map(|width| usize::try_from(width).map_err(|_| internal_target_error("ascii")))
        .transpose()
}

/// One padded cell read under a declared width, or under none at all.
fn ascii_padded_text(width: Option<i32>, padded: &[u8]) -> crate::Result<&str> {
    match width {
        Some(width) => ascii_text(width, padded),
        None => crate::types::ascii_free_text(padded),
    }
}

/// The one check a securities number carries beyond its width.
///
/// A column of them holds the canonical spelling: what a cast lets in is what
/// a read answers, so the check digit and the case are settled here rather
/// than on every read of the cell.
fn canonical_cell<'a>(
    canonical: bool,
    field: &Field,
    index: usize,
    text: &'a str,
) -> Result<&'a str> {
    if canonical && !crate::types::Isin::is_canonical(text) {
        return Err(Error::IncompatibleSchema(format!(
            "row {index} of column {name}: expected a securities number in its canonical \
             spelling, got {text:?}",
            name = field.name()
        )));
    }
    Ok(text)
}
