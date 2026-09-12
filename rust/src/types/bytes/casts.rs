//! Binary layout accounting and identity checks for Arrow casts.

use arrow_array::builder::{BinaryBuilder, BinaryViewBuilder, LargeBinaryBuilder};
use arrow_array::types::{
    Int8Type, Int16Type, Int32Type, Int64Type, UInt8Type, UInt16Type, UInt32Type, UInt64Type,
};
use arrow_array::{
    Array, BinaryArray, BinaryViewArray, DictionaryArray, FixedSizeBinaryArray, Int16RunArray,
    Int32RunArray, Int64RunArray, LargeBinaryArray, LargeStringArray, StringArray, StringViewArray,
    UnionArray,
};

use std::sync::Arc;

use arrow_array::ArrayRef;
use arrow_buffer::BooleanBuffer;
use arrow_cast::can_cast_types;
use arrow_schema::DataType as ArrowDataType;
use smol_str::{SmolStr, format_smolstr};

use crate::arrow::{Error, Result};
use crate::types::budget::MaterializationBudget;
use crate::types::bytes::{BytesLayout, BytesParameters};
use crate::types::cast::{arrow_cast_exposed, downcast, internal_target_error, named_cell};
use crate::types::nested::casts::{is_exposed, null_buffers_ptr_eq};
use crate::{DataType, Field};

/// Whether one byte layout reaches another only through Arrow's `Binary`.
///
/// Arrow's kernel reads every variable byte layout as every other one, but it
/// has no direct reading between a fixed binary and text, nor between a binary
/// view and a fixed binary. Both of those are the same payload under two
/// framings, and `Binary` is the framing both sides already convert to, so the
/// reading exists - it just takes two hops instead of one.
pub(crate) fn bridges_through_binary(source: &ArrowDataType, target: &ArrowDataType) -> bool {
    is_byte_layout(source)
        && is_byte_layout(target)
        && !can_cast_types(source, target)
        && can_cast_types(source, &ArrowDataType::Binary)
        && can_cast_types(&ArrowDataType::Binary, target)
}

/// Whether an Arrow layout stores one byte payload per row.
fn is_byte_layout(dtype: &ArrowDataType) -> bool {
    matches!(
        dtype,
        ArrowDataType::Binary
            | ArrowDataType::LargeBinary
            | ArrowDataType::BinaryView
            | ArrowDataType::FixedSizeBinary(_)
            | ArrowDataType::Utf8
            | ArrowDataType::LargeUtf8
            | ArrowDataType::Utf8View
    )
}

/// A byte source under the one variable framing every value reader takes.
///
/// A payload is one payload under all four binary framings; only the offsets
/// differ. A reader that validates values - an ASCII width, a code, a UUID -
/// takes those bytes directly, because pushing them through a text temporary
/// first turns a payload that is not UTF-8 into a null instead of the refusal
/// the value rule owes it. A text source is not bytes and answers `None`, so
/// it keeps the text path it always had, and a fixed binary answers `None` too
/// because its reader already knows the width it carries.
pub(crate) fn variable_binary_source(
    array: &ArrayRef,
    field: &Field,
    exposure: Option<&BooleanBuffer>,
    budget: &mut MaterializationBudget,
) -> Result<Option<ArrayRef>> {
    match array.data_type() {
        ArrowDataType::Binary => Ok(Some(Arc::clone(array))),
        ArrowDataType::LargeBinary | ArrowDataType::BinaryView => Ok(Some(arrow_cast_exposed(
            array,
            &ArrowDataType::Binary,
            false,
            exposure,
            &Field::new(field.name(), DataType::binary(), true),
            budget,
        )?)),
        _ => Ok(None),
    }
}

/// Validates every exposed, non-null value entering a bounded byte datatype
/// and stores it in the target's own layout.
///
/// A maximum is the one thing about bytes Arrow cannot check, so it is the
/// one byte cast that reads cells rather than buffers: a fixed binary is
/// read as it is, every variable layout through the one `Binary` framing,
/// and anything else through the kernel's own reading into that framing. A
/// cell past the maximum is null under `safe` and an error naming the row
/// otherwise; nothing is copied until every cell has been measured.
pub(crate) fn ingest_bytes_array(
    array: &ArrayRef,
    safe: bool,
    field: &Field,
    exposure: Option<&BooleanBuffer>,
    budget: &mut MaterializationBudget,
) -> Result<ArrayRef> {
    let Some(target) = field.dtype().bytes_parameters() else {
        return Err(internal_target_error("bytes"));
    };
    if let ArrowDataType::FixedSizeBinary(_) = array.data_type() {
        let cells = downcast::<FixedSizeBinaryArray>(array.as_ref())?;
        return bytes_storage(
            target,
            field,
            cells.len(),
            safe,
            exposure,
            budget,
            |index| cells.is_valid(index).then(|| cells.value(index)),
        );
    }
    let bytes = match variable_binary_source(array, field, exposure, budget)? {
        Some(bytes) => bytes,
        // The temporary is nullable bytes: the kernel's masked path fills
        // nothing, and the target's own null policy runs after the reading.
        None => arrow_cast_exposed(
            array,
            &ArrowDataType::Binary,
            safe,
            exposure,
            &Field::new(field.name(), DataType::binary(), true),
            budget,
        )?,
    };
    let cells = downcast::<BinaryArray>(bytes.as_ref())?;
    bytes_storage(
        target,
        field,
        cells.len(),
        safe,
        exposure,
        budget,
        |index| cells.is_valid(index).then(|| cells.value(index)),
    )
}

/// Builds the storage of one bounded byte layout from one cell per row.
///
/// Unexposed rows are null: an ancestor hides them, so their bytes are
/// neither measured nor copied.
fn bytes_storage<'a>(
    target: BytesParameters,
    field: &Field,
    rows: usize,
    safe: bool,
    exposure: Option<&BooleanBuffer>,
    budget: &mut MaterializationBudget,
    cell: impl Fn(usize) -> Option<&'a [u8]>,
) -> Result<ArrayRef> {
    // A fixed width is the storage itself, which is why no maximum ever
    // reaches it and why this reader owes it nothing.
    let Some(max) = target.max() else {
        return Err(internal_target_error("bytes"));
    };
    budget.add_array(field.dtype(), rows)?;
    let mut payload = 0_usize;
    let accepted = |index: usize| -> Result<Option<&'a [u8]>> {
        let Some(bytes) = cell(index).filter(|_| is_exposed(exposure, index)) else {
            return Ok(None);
        };
        if bytes.len() <= max as usize {
            return Ok(Some(bytes));
        }
        if safe {
            return Ok(None);
        }
        named_cell(
            field,
            index,
            Err(crate::Error::InvalidRecord {
                path: SmolStr::new_static("$"),
                reason: crate::text::expected_got(
                    format_args!("at most {max} bytes"),
                    format_smolstr!("{} bytes", bytes.len()),
                ),
            }),
        )
    };
    for index in 0..rows {
        payload = payload.saturating_add(accepted(index)?.map_or(0, <[u8]>::len));
    }
    budget.add_bytes(payload)?;
    macro_rules! filled {
        ($builder:expr) => {{
            let mut builder = $builder;
            for index in 0..rows {
                match accepted(index)? {
                    Some(bytes) => builder.append_value(bytes),
                    None => builder.append_null(),
                }
            }
            Arc::new(builder.finish()) as ArrayRef
        }};
    }
    Ok(match target.layout() {
        BytesLayout::Binary => filled!(BinaryBuilder::with_capacity(rows, payload)),
        BytesLayout::LargeBinary => filled!(LargeBinaryBuilder::with_capacity(rows, payload)),
        // Arrow's view layout carries a prefix per cell rather than offsets,
        // so it takes the row count and grows its own payload blocks.
        BytesLayout::BinaryView => filled!(BinaryViewBuilder::with_capacity(rows)),
        BytesLayout::FixedSizeBinary => return Err(internal_target_error("bytes")),
    })
}

pub(crate) fn projected_byte_len(
    array: &dyn Array,
    source_type: &DataType,
    index: usize,
) -> Result<usize> {
    if index >= array.len() {
        return Err(Error::IncompatibleSchema(
            "Arrow byte projection index exceeds its source array".to_owned(),
        ));
    }
    if array.is_null(index)
        && !matches!(
            source_type,
            DataType::Dictionary(_) | DataType::Union(..) | DataType::RunEndEncoded(_)
        )
    {
        return Ok(0);
    }
    let bytes = match source_type {
        // A byte payload is measured by the framing the array is in: a
        // string, a code and a UUID each project onto one of the byte
        // layouts, and the array says which.
        bytes if bytes.kind().is_bytes() || matches!(bytes, DataType::Uuid) => {
            byte_cell_len(array, index)?
        }
        DataType::Dictionary(dictionary) => {
            macro_rules! dictionary_len {
                ($key:ty) => {{
                    let dictionary_array = downcast::<DictionaryArray<$key>>(array)?;
                    if dictionary_array.keys().is_null(index) {
                        0
                    } else {
                        let key = usize::try_from(dictionary_array.keys().value(index)).map_err(
                            |_| {
                                Error::IncompatibleSchema(
                                    "Arrow dictionary key is negative or exceeds usize".to_owned(),
                                )
                            },
                        )?;
                        projected_byte_len(
                            dictionary_array.values().as_ref(),
                            dictionary.value(),
                            key,
                        )?
                    }
                }};
            }
            match dictionary.key() {
                DataType::Int8 => dictionary_len!(Int8Type),
                DataType::Int16 => dictionary_len!(Int16Type),
                DataType::Int32 => dictionary_len!(Int32Type),
                DataType::Int64 => dictionary_len!(Int64Type),
                DataType::UInt8 => dictionary_len!(UInt8Type),
                DataType::UInt16 => dictionary_len!(UInt16Type),
                DataType::UInt32 => dictionary_len!(UInt32Type),
                DataType::UInt64 => dictionary_len!(UInt64Type),
                _ => {
                    return Err(Error::IncompatibleSchema(
                        "Arrow dictionary byte projection key is not an integer".to_owned(),
                    ));
                }
            }
        }
        DataType::Union(fields, _) => {
            let union = downcast::<UnionArray>(array)?;
            let type_id = union.type_id(index);
            let (_, field) = fields
                .iter()
                .find(|(candidate, _)| *candidate == type_id)
                .ok_or_else(|| {
                    Error::IncompatibleSchema(format!(
                        "Arrow union byte projection has unknown type ID {type_id}"
                    ))
                })?;
            projected_byte_len(
                union.child(type_id).as_ref(),
                field.dtype(),
                union.value_offset(index),
            )?
        }
        DataType::RunEndEncoded(encoded) => match encoded.run_ends().dtype() {
            DataType::Int16 => {
                let run = downcast::<Int16RunArray>(array)?;
                projected_byte_len(
                    run.values().as_ref(),
                    encoded.values().dtype(),
                    run.get_physical_index(index),
                )?
            }
            DataType::Int32 => {
                let run = downcast::<Int32RunArray>(array)?;
                projected_byte_len(
                    run.values().as_ref(),
                    encoded.values().dtype(),
                    run.get_physical_index(index),
                )?
            }
            DataType::Int64 => {
                let run = downcast::<Int64RunArray>(array)?;
                projected_byte_len(
                    run.values().as_ref(),
                    encoded.values().dtype(),
                    run.get_physical_index(index),
                )?
            }
            _ => {
                return Err(Error::IncompatibleSchema(
                    "Arrow run-end byte projection type is invalid".to_owned(),
                ));
            }
        },
        DataType::Boolean | DataType::UInt16 => 5,
        DataType::Int8 => 4,
        DataType::UInt8 => 3,
        DataType::Int16 => 6,
        DataType::Int32 | DataType::Decimal32 { .. } => 12,
        DataType::UInt32 => 10,
        DataType::Int64 | DataType::Decimal64 { .. } => 21,
        DataType::UInt64 => 20,
        DataType::Float16 => 16,
        DataType::Float32 => 24,
        DataType::Float64 => 32,
        DataType::Decimal128 { .. } => 41,
        DataType::Decimal256 { .. } => 78,
        DataType::DateTime64 { .. }
        | DataType::Date32
        | DataType::Date64
        | DataType::Time32(_)
        | DataType::Time64(_)
        | DataType::Duration32(_)
        | DataType::Duration64(_)
        | DataType::Interval(_) => 128,
        _ => 0,
    };
    Ok(bytes)
}

/// The bytes one cell holds, under whichever byte framing the array is in.
fn byte_cell_len(array: &dyn Array, index: usize) -> Result<usize> {
    Ok(match array.data_type() {
        ArrowDataType::Binary => downcast::<BinaryArray>(array)?.value(index).len(),
        ArrowDataType::LargeBinary => downcast::<LargeBinaryArray>(array)?.value(index).len(),
        ArrowDataType::BinaryView => downcast::<BinaryViewArray>(array)?.value(index).len(),
        ArrowDataType::FixedSizeBinary(_) => {
            downcast::<FixedSizeBinaryArray>(array)?.value(index).len()
        }
        ArrowDataType::Utf8 => downcast::<StringArray>(array)?.value(index).len(),
        ArrowDataType::LargeUtf8 => downcast::<LargeStringArray>(array)?.value(index).len(),
        ArrowDataType::Utf8View => downcast::<StringViewArray>(array)?.value(index).len(),
        other => {
            return Err(Error::IncompatibleSchema(format!(
                "Arrow byte projection over a {other:?} array, which holds no byte payload"
            )));
        }
    })
}

pub(crate) fn checked_valid_payload_bytes(
    len: usize,
    mut is_valid: impl FnMut(usize) -> bool,
    mut value_len: impl FnMut(usize) -> usize,
) -> Result<usize> {
    (0..len).try_fold(0usize, |bytes, index| {
        if !is_valid(index) {
            return Ok(bytes);
        }
        bytes
            .checked_add(value_len(index))
            .ok_or_else(|| Error::IncompatibleSchema("Arrow payload bytes exceed usize".to_owned()))
    })
}

#[allow(clippy::too_many_lines)] // Mirrors every nested Arrow container layout.
pub(crate) fn byte_array_storage_ptr_eq(
    left: &dyn Array,
    right: &dyn Array,
    dtype: &DataType,
) -> Result<bool> {
    macro_rules! shared {
        ($array:ty) => {{
            let left = downcast::<$array>(left)?;
            let right = downcast::<$array>(right)?;
            left.offsets().ptr_eq(right.offsets())
                && byte_slices_ptr_eq(left.value_data(), right.value_data())
                && null_buffers_ptr_eq(left.nulls(), right.nulls())
        }};
    }
    if !(dtype.kind().is_bytes() || matches!(dtype, DataType::Uuid)) {
        return Ok(false);
    }
    // The framing is the array's: a string, a code and a UUID each project
    // onto one of the byte layouts, and a view layout owns no offsets to
    // compare.
    Ok(match left.data_type() {
        ArrowDataType::Binary => shared!(BinaryArray),
        ArrowDataType::LargeBinary => shared!(LargeBinaryArray),
        ArrowDataType::Utf8 => shared!(StringArray),
        ArrowDataType::LargeUtf8 => shared!(LargeStringArray),
        ArrowDataType::FixedSizeBinary(_) => {
            let left = downcast::<FixedSizeBinaryArray>(left)?;
            let right = downcast::<FixedSizeBinaryArray>(right)?;
            byte_slices_ptr_eq(left.value_data(), right.value_data())
                && null_buffers_ptr_eq(left.nulls(), right.nulls())
        }
        _ => false,
    })
}

fn byte_slices_ptr_eq(left: &[u8], right: &[u8]) -> bool {
    left.len() == right.len() && (left.is_empty() || std::ptr::eq(left.as_ptr(), right.as_ptr()))
}
