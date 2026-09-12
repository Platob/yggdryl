//! Arrow casts owned by this datatype family.

use std::borrow::Cow;
use std::sync::Arc;

use arrow_array::builder::{
    BinaryBuilder, BinaryViewBuilder, LargeBinaryBuilder, LargeStringBuilder, StringBuilder,
    StringViewBuilder,
};
use arrow_array::{Array, ArrayRef, BinaryArray, FixedSizeBinaryArray, StringArray};
use arrow_buffer::{BooleanBuffer, BooleanBufferBuilder};
use arrow_schema::DataType as ArrowDataType;

use crate::arrow::{Error, Result};
use crate::types::budget::{MaterializationBudget, reserve_vec_bytes};
use crate::types::bytes::casts::variable_binary_source;
use crate::types::cast::arrow_cast_exposed;
use crate::types::cast::{downcast, internal_target_error, named_cell};
use crate::types::nested::casts::is_exposed;
use crate::types::string::arrow_storage;
use crate::types::{Str, StringParameters, code_cell_text, code_text, trim_padding};
use crate::{Charset, DataType, Field};

/// What the planner learned about the column a string reads from.
///
/// A recognized source declares how its bytes are read; a bare one is read
/// as text where Arrow calls it text, and as bytes already in the target's
/// charset everywhere else.
#[derive(Clone, Debug)]
pub(crate) enum StringSource {
    /// A `yggdryl.string` column: every cell is read under its own
    /// parameters before it is restated under the target's.
    String(StringParameters),
    /// A registered code: validated ASCII, trimmed of its padding.
    Code(DataType),
    /// Storage with no extension identity.
    Bare,
}

/// One cell of the temporary every string source is read through.
#[derive(Clone, Copy)]
enum Cell<'a> {
    Text(&'a str),
    Bytes(&'a [u8]),
}

/// Validates every exposed, non-null value entering a string datatype and
/// stores it in the target's own storage.
///
/// Every source is read through one of three temporaries - text, variable
/// bytes, or the fixed binary it already is - so the per-cell reading is
/// stated once per source kind rather than once per Arrow layout. A fixed
/// binary cell is trimmed of its padding on the way in: the padding is the
/// slot's, never the value's. A failing cell is null under `safe` and an
/// error naming the row otherwise.
pub(crate) fn ingest_string_array(
    array: &ArrayRef,
    source: &StringSource,
    safe: bool,
    field: &Field,
    exposure: Option<&BooleanBuffer>,
    budget: &mut MaterializationBudget,
) -> Result<ArrayRef> {
    let Some(target) = field.dtype().string_parameters() else {
        return Err(internal_target_error("string"));
    };
    let read = |cell: Cell<'_>| -> crate::Result<Str> {
        match (source, cell) {
            (StringSource::String(parameters), Cell::Text(text)) => {
                Str::from_storage(text, *parameters).try_with_parameters(target)
            }
            (StringSource::String(parameters), Cell::Bytes(bytes)) => {
                Str::from_bytes(bytes, *parameters)?.try_with_parameters(target)
            }
            (StringSource::Code(code), Cell::Bytes(bytes)) => {
                Str::from_storage(code_cell_text(code, bytes)?, StringParameters::default())
                    .try_with_parameters(target)
            }
            (StringSource::Code(_) | StringSource::Bare, Cell::Text(text)) => {
                Str::from_storage(text, StringParameters::default()).try_with_parameters(target)
            }
            (StringSource::Bare, Cell::Bytes(bytes)) => Str::from_bytes(bytes, target),
        }
    };
    if let ArrowDataType::FixedSizeBinary(_) = array.data_type() {
        let cells = downcast::<FixedSizeBinaryArray>(array.as_ref())?;
        return string_storage(
            target,
            field,
            cells.len(),
            safe,
            exposure,
            budget,
            |index| {
                cells
                    .is_valid(index)
                    .then(|| read(Cell::Bytes(trim_padding(cells.value(index)))))
            },
        );
    }
    if let Some(bytes) = variable_binary_source(array, field, exposure, budget)? {
        let cells = downcast::<BinaryArray>(bytes.as_ref())?;
        return string_storage(
            target,
            field,
            cells.len(),
            safe,
            exposure,
            budget,
            |index| {
                cells
                    .is_valid(index)
                    .then(|| read(Cell::Bytes(cells.value(index))))
            },
        );
    }
    let text = if array.data_type() == &ArrowDataType::Utf8 {
        Arc::clone(array)
    } else {
        // The temporary is nullable text: the kernel's masked path fills
        // nothing, and the target's own null policy runs after the reading.
        arrow_cast_exposed(
            array,
            &ArrowDataType::Utf8,
            safe,
            exposure,
            &Field::new(field.name(), DataType::utf8(), true),
            budget,
        )?
    };
    let cells = downcast::<StringArray>(text.as_ref())?;
    string_storage(
        target,
        field,
        cells.len(),
        safe,
        exposure,
        budget,
        |index| {
            cells
                .is_valid(index)
                .then(|| read(Cell::Text(cells.value(index))))
        },
    )
}

/// Builds the storage of one string datatype from one read cell per row.
///
/// Unexposed rows are null: an ancestor hides them, so their bytes are
/// neither validated nor copied. Every value is held before a byte of the
/// output exists, so the payload is measured once and the buffer sized to
/// it. The repertoire was settled when the value was restated - US-ASCII is
/// judged there, UTF-8 is what a value holds - so text storage takes the
/// characters as they are; every other charset writes its bytes here, and a
/// scalar it cannot spell fails at the write, as in every other writer.
fn string_storage(
    target: StringParameters,
    field: &Field,
    rows: usize,
    safe: bool,
    exposure: Option<&BooleanBuffer>,
    budget: &mut MaterializationBudget,
    cell: impl Fn(usize) -> Option<crate::Result<Str>>,
) -> Result<ArrayRef> {
    budget.add_array(field.dtype(), rows)?;
    reserve_vec_bytes::<Option<Str>>(budget, rows)?;
    let mut values = Vec::new();
    values.try_reserve_exact(rows).map_err(|error| {
        Error::IncompatibleSchema(format!("string output allocation failed: {error}"))
    })?;
    let mut payload = 0_usize;
    for index in 0..rows {
        let value = match is_exposed(exposure, index).then(|| cell(index)).flatten() {
            Some(Ok(value)) => Some(value),
            Some(Err(_)) if safe => None,
            Some(read @ Err(_)) => Some(named_cell(field, index, read)?),
            None => None,
        };
        payload = payload.saturating_add(value.as_ref().map_or(0, Str::encoded_len));
        values.push(value);
    }
    budget.add_bytes(payload)?;
    let charset = target.charset();
    macro_rules! filled {
        ($builder:expr, $stored:expr) => {{
            let mut builder = $builder;
            for (index, value) in values.iter().enumerate() {
                match value.as_ref().map($stored) {
                    Some(Ok(stored)) => builder.append_value(stored),
                    Some(Err(_)) if safe => builder.append_null(),
                    Some(Err(error)) => return named_cell(field, index, Err(error)),
                    None => builder.append_null(),
                }
            }
            Arc::new(builder.finish()) as ArrayRef
        }};
    }
    Ok(match arrow_storage(target)? {
        ArrowDataType::FixedSizeBinary(width) => {
            let slot = usize::try_from(width).map_err(|_| internal_target_error("string"))?;
            // The reservation bounds `rows * slot`, so the product cannot overflow.
            let mut padded = vec![0_u8; rows * slot];
            let mut validity = BooleanBufferBuilder::new(rows);
            for (index, value) in values.iter().enumerate() {
                let present = match value.as_ref().map(|value| encoded(charset, value)) {
                    Some(Ok(encoded)) => {
                        padded[index * slot..][..encoded.len()].copy_from_slice(&encoded);
                        true
                    }
                    Some(Err(_)) if safe => false,
                    Some(Err(error)) => return named_cell(field, index, Err(error)),
                    None => false,
                };
                validity.append(present);
            }
            let nulls = arrow_buffer::NullBuffer::new(validity.finish());
            Arc::new(FixedSizeBinaryArray::try_new(
                width,
                arrow_buffer::Buffer::from(padded),
                (nulls.null_count() != 0).then_some(nulls),
            )?)
        }
        ArrowDataType::Utf8 => filled!(StringBuilder::with_capacity(rows, payload), characters),
        ArrowDataType::LargeUtf8 => {
            filled!(LargeStringBuilder::with_capacity(rows, payload), characters)
        }
        // Arrow's view layout carries a prefix per cell rather than offsets,
        // so it takes the row count and grows its own payload blocks.
        ArrowDataType::Utf8View => filled!(StringViewBuilder::with_capacity(rows), characters),
        ArrowDataType::Binary => filled!(BinaryBuilder::with_capacity(rows, payload), |value| {
            encoded(charset, value)
        }),
        ArrowDataType::LargeBinary => {
            filled!(LargeBinaryBuilder::with_capacity(rows, payload), |value| {
                encoded(charset, value)
            })
        }
        ArrowDataType::BinaryView => filled!(BinaryViewBuilder::with_capacity(rows), |value| {
            encoded(charset, value)
        }),
        _ => return Err(internal_target_error("string")),
    })
}

/// What text storage stores: the characters themselves.
fn characters(value: &Str) -> crate::Result<&str> {
    Ok(value.as_str())
}

/// What binary storage stores: the characters written in the charset.
fn encoded(charset: Charset, value: &Str) -> crate::Result<Cow<'_, [u8]>> {
    charset.encode(value.as_str())
}

/// Validates every exposed, non-null value entering a registered code and
/// pads it into that code's fixed storage.
///
/// A fixed binary of the code's own width is the same array once validated,
/// another fixed width re-pads each trimmed value, and anything else first
/// renders as Utf8 through Arrow's kernel. What the constant width buys is
/// the inner loop: the length check, the slot arithmetic and the padding
/// copy are all fixed-size, so a currency column ingests three bytes a row
/// with no width to read.
pub(crate) fn ingest_code_array<const WIDTH: usize>(
    array: &ArrayRef,
    safe: bool,
    field: &Field,
    exposure: Option<&BooleanBuffer>,
    budget: &mut MaterializationBudget,
) -> Result<ArrayRef> {
    if let ArrowDataType::FixedSizeBinary(source_width) = array.data_type() {
        let source = downcast::<FixedSizeBinaryArray>(array.as_ref())?;
        if usize::try_from(*source_width).is_ok_and(|width| width == WIDTH) {
            // The array is the target's own storage once every cell passes;
            // a cell that fails is an error when strict, and under `safe` a
            // null the rebuild below writes.
            let mut every_cell_passes = true;
            for index in 0..source.len() {
                if is_exposed(exposure, index)
                    && source.is_valid(index)
                    && code_cell::<WIDTH>(field, index, source.value(index), safe)?.is_none()
                {
                    every_cell_passes = false;
                    break;
                }
            }
            if every_cell_passes {
                return Ok(Arc::clone(array));
            }
        }
        return padded_code_array::<WIDTH>(field, source.len(), safe, exposure, budget, |index| {
            source.is_valid(index).then(|| source.value(index))
        });
    }
    if let Some(bytes) = variable_binary_source(array, field, exposure, budget)? {
        let source = downcast::<BinaryArray>(bytes.as_ref())?;
        return padded_code_array::<WIDTH>(field, source.len(), safe, exposure, budget, |index| {
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
    padded_code_array::<WIDTH>(field, source.len(), safe, exposure, budget, |index| {
        source
            .is_valid(index)
            .then(|| source.value(index).as_bytes())
    })
}

/// Builds the fixed storage of one registered code from one cell per row.
///
/// Unexposed rows are null, and so is a refused cell under `safe`, exactly
/// as a refused string cell is: the two families answer one cast the same
/// way.
fn padded_code_array<'a, const WIDTH: usize>(
    field: &Field,
    rows: usize,
    safe: bool,
    exposure: Option<&BooleanBuffer>,
    budget: &mut MaterializationBudget,
    cell: impl Fn(usize) -> Option<&'a [u8]>,
) -> Result<ArrayRef> {
    // The reservation bounds `rows * WIDTH`, so the product cannot overflow.
    budget.add_array(field.dtype(), rows)?;
    let mut bytes = vec![0_u8; rows * WIDTH];
    let mut validity = BooleanBufferBuilder::new(rows);
    for (index, slot) in bytes.chunks_exact_mut(WIDTH).enumerate() {
        let value = cell(index)
            .filter(|_| is_exposed(exposure, index))
            .map(|raw| code_cell::<WIDTH>(field, index, raw, safe))
            .transpose()?
            .flatten();
        if let Some(text) = value {
            slot[..text.len()].copy_from_slice(text.as_bytes());
        }
        validity.append(value.is_some());
    }
    let nulls = arrow_buffer::NullBuffer::new(validity.finish());
    Ok(Arc::new(FixedSizeBinaryArray::try_new(
        WIDTH as i32,
        arrow_buffer::Buffer::from(bytes),
        (nulls.null_count() != 0).then_some(nulls),
    )?))
}

/// Validates one code cell at the code's constant width.
///
/// `None` is a refused cell under `safe`, which the caller stores as null;
/// strict, the refusal names the row and the column.
fn code_cell<'a, const WIDTH: usize>(
    field: &Field,
    index: usize,
    bytes: &'a [u8],
    safe: bool,
) -> Result<Option<&'a str>> {
    let refused = |reason: String| match safe {
        true => Ok(None),
        false => Err(Error::IncompatibleSchema(format!(
            "row {index} of column {name}: {reason}",
            name = field.name()
        ))),
    };
    let text = match code_text::<WIDTH>(bytes) {
        Ok(text) => text,
        Err(error) => return refused(error.to_string()),
    };
    // A securities number carries its own check, and a column of them
    // holds the canonical spelling: what a cast lets in is what a read
    // answers, so the check digit and the case are settled here rather
    // than on every read of the cell.
    if matches!(field.dtype(), DataType::Isin) && !crate::types::Isin::is_canonical(text) {
        return refused(format!(
            "expected a securities number in its canonical spelling, got {text:?}"
        ));
    }
    Ok(Some(text))
}
