//! Binary layout accounting and identity checks for Arrow casts.

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

use crate::arrow::{Error, Result};
use crate::types::budget::MaterializationBudget;
use crate::types::cast::{arrow_cast_exposed, downcast};
use crate::types::nested::casts::null_buffers_ptr_eq;
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
            &Field::new(field.name(), DataType::Binary, true),
            budget,
        )?)),
        _ => Ok(None),
    }
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
        // Variable ASCII stores its own bytes, so it is Arrow's `Binary`.
        DataType::Binary | DataType::Ascii => downcast::<BinaryArray>(array)?.value(index).len(),
        DataType::LargeBinary => downcast::<LargeBinaryArray>(array)?.value(index).len(),
        DataType::BinaryView => downcast::<BinaryViewArray>(array)?.value(index).len(),
        DataType::FixedSizeBinary(_)
        | DataType::FixedAscii(_)
        | DataType::Country
        | DataType::Currency
        | DataType::Mic
        | DataType::Cfi
        | DataType::Isin
        | DataType::Uuid => downcast::<FixedSizeBinaryArray>(array)?.value(index).len(),
        DataType::Utf8 => downcast::<StringArray>(array)?.value(index).len(),
        DataType::LargeUtf8 => downcast::<LargeStringArray>(array)?.value(index).len(),
        DataType::Utf8View => downcast::<StringViewArray>(array)?.value(index).len(),
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
    Ok(match dtype {
        DataType::Binary | DataType::Ascii => shared!(BinaryArray),
        DataType::LargeBinary => shared!(LargeBinaryArray),
        DataType::Utf8 => shared!(StringArray),
        DataType::LargeUtf8 => shared!(LargeStringArray),
        DataType::FixedSizeBinary(_)
        | DataType::FixedAscii(_)
        | DataType::Country
        | DataType::Currency
        | DataType::Mic
        | DataType::Cfi
        | DataType::Isin
        | DataType::Uuid => {
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
