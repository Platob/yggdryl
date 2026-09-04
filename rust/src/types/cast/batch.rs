//! Exact and preflight record-batch boundaries.

use arrow_array::RecordBatch;

use super::cast_record_batch;
use crate::Field;
use crate::arrow::{Error, Result, arrow_schema_from_field};

/// Validates an exact source batch against one declared Struct root Field.
///
/// This low-level hook is public for native runtime bindings that already own
/// Arrow arrays but intentionally hidden from their user-facing APIs. It uses
/// IPC-compatible schema comparison and rejects recursive logical values that
/// would require default filling or another canonical repair.
///
/// # Errors
///
/// Returns an error unless `field` is a valid non-null Struct root and the
/// batch has a compatible schema plus valid recursive values.
#[doc(hidden)]
pub fn validate_arrow_batch(field: &Field, batch: &RecordBatch) -> Result<()> {
    field.validate_bounded()?;
    let root_field = field.clone();
    root_field.validate_struct_root()?;

    // Valid means "needs no repair": casting an exact batch returns the very
    // arrays it was given, so a changed column is a validation failure.
    let cast = cast_record_batch(field, batch.clone(), true)?;
    for (index, (before, after)) in batch.columns().iter().zip(cast.columns()).enumerate() {
        if !std::sync::Arc::ptr_eq(before, after) {
            let name = batch.schema().field(index).name().clone();
            return Err(Error::IncompatibleSchema(format!(
                "field {name:?} requires canonical repair and is not valid as stored"
            )));
        }
    }
    Ok(())
}

/// Preflights an empty source-to-target batch cast for runtime readers.
///
/// This binding hook validates both Struct roots and constructs the recursive
/// cast plan from the canonical source schema without materializing arrays. It
/// lets an empty backend reject an invalid target before a lazy checked read is
/// attempted, without maintaining another schema table.
///
/// # Errors
///
/// Returns an error when either root Field is invalid/nullable/non-Struct or
/// the recursive source-to-target cast plan cannot be constructed.
#[doc(hidden)]
pub fn preflight_arrow_batch_cast(
    source: &Field,
    target: Option<&Field>,
    safe: bool,
) -> Result<()> {
    let schema = arrow_schema_from_field(source)?;
    let target = target.unwrap_or(source);
    // An empty batch of the source schema exercises the whole recursive plan
    // without materializing a row.
    let empty = RecordBatch::new_empty(schema);
    cast_record_batch(target, empty, safe)?;
    Ok(())
}
