//! The one estimate of what rows occupy in memory, read by every byte bound.
//!
//! A commit cadence, a write limit, a batch byte target and the Iceberg file
//! rolling all cut a stream by bytes, and they all cut it here, so one batch
//! costs the same whichever bound charges it. The measure is the rows' own
//! extent: a zero-copy slice counts the rows it reaches, never the buffers it
//! shares with its parent, because a bound that charged the parent's buffers
//! per slice would count one batch as many times as it was cut.
//!
//! ```
//! use std::sync::Arc;
//!
//! use arrow_array::{Int64Array, RecordBatch};
//! use arrow_schema::{DataType as ArrowType, Field as ArrowField, Schema};
//! use yggdryl::arrow::memory_size;
//!
//! let schema = Arc::new(Schema::new(vec![ArrowField::new("id", ArrowType::Int64, false)]));
//! let batch = RecordBatch::try_new(
//!     schema,
//!     vec![Arc::new(Int64Array::from_iter_values(0..1024))],
//! )?;
//!
//! // Eight bytes a row, and a slice counts only its own rows.
//! assert_eq!(memory_size(&batch), 8 * 1024);
//! assert_eq!(memory_size(&batch.slice(0, 16)), 8 * 16);
//! # Ok::<(), arrow_schema::ArrowError>(())
//! ```

use std::sync::Arc;

use arrow_array::{Array, ArrayRef, RecordBatch};

use crate::Scalar;

/// A fixed per-row width, standing for the offsets and validity a row costs.
///
/// Every Arrow layout charges something per row beyond the payload - an
/// offset, a validity bit, a null slot - and a running estimate that charged
/// nothing would never close a batch of empty rows. Shared with the producers
/// that measure their own rows, so one row costs the same width whichever
/// statistic charges it.
pub(crate) const ROW_OVERHEAD: usize = 16;

/// The bytes a batch's rows occupy, as its own slices count them.
///
/// [`RecordBatch::get_array_memory_size`] counts every buffer whole, so a
/// zero-copy slice of a large batch reports its parent's allocation; a size
/// estimate spread over the slice's rows then comes out as many times too
/// large as there are slices. This counts each column's sliced extent through
/// [`array_memory_size`], and over flat and struct columns allocates nothing
/// to do so.
#[must_use]
pub fn memory_size(batch: &RecordBatch) -> usize {
    batch.columns().iter().map(array_memory_size).sum()
}

/// The bytes one column's rows occupy, as its own slice counts them - the
/// per-column half of [`memory_size`].
///
/// A flat column counts its sliced buffers. The layouts whose values live
/// elsewhere count what the slice reaches: a view column its sixteen-byte
/// views and the out-of-line bytes they point at, a list or map only the
/// child range its offsets span, a struct its children, a dictionary its keys
/// sliced and its values whole. A layout that cannot be sliced so falls back
/// to its whole buffers. A flat, struct or dictionary column is measured off
/// its buffers with nothing allocated; a list, map or view column allocates
/// the one `Arc` of the child window its offsets span.
#[must_use]
pub fn array_memory_size(column: &ArrayRef) -> usize {
    sliced_size(column.as_ref())
}

/// What one canonicalized row costs once laid out, near enough to batch by.
///
/// The bound a producer measures with this is a target rather than a
/// ceiling. An in-progress builder cannot be measured the way a finished
/// batch can, so a producer accumulates the payload each row carries plus a
/// fixed per-row width standing for its offsets and validity, and the
/// finished batch's own [`memory_size`] is what a caller measures against.
/// Cheap and monotone beats exact and per-row: an exact measure would cost
/// more than the parse that produced the row. A null costs its validity bit
/// alone, text and bytes their length, a held column its buffers as
/// [`array_memory_size`] counts them, and every other leaf the widest fixed
/// width, sixteen bytes.
#[must_use]
pub fn scalar_memory_size(value: &Scalar) -> usize {
    ROW_OVERHEAD + payload_bytes(value)
}

/// The leaf payload one value carries, summed through nesting.
fn payload_bytes(value: &Scalar) -> usize {
    // A null costs a validity bit, not a value. Charging it a leaf's width
    // would make a wide mostly-null row - which a mixed capture's facet
    // columns are - estimate several times what it actually occupies, and the
    // bound would then cut batches far shorter than the caller asked for.
    if value.is_null() {
        return 0;
    }
    if let Some(text) = value.as_str() {
        return text.len();
    }
    if let Some(bytes) = value.as_bytes() {
        return bytes.len();
    }
    if let Some(held) = value.as_serie() {
        return ROW_OVERHEAD
            + match held.into_arrow_array() {
                // A column's cost is its buffers, and no row is built to
                // count it.
                Some(array) => array_memory_size(&array),
                None => held.rows().iter().map(payload_bytes).sum::<usize>(),
            };
    }
    if let Some(held) = value.as_mapping() {
        return held
            .iter()
            .map(|(key, held)| payload_bytes(key) + payload_bytes(held))
            .sum::<usize>()
            + ROW_OVERHEAD;
    }
    if let Some(held) = value.as_struct() {
        return held
            .iter()
            .map(|(key, held)| key.len() + payload_bytes(held))
            .sum::<usize>()
            + ROW_OVERHEAD;
    }
    // Every remaining variant is a fixed-width leaf, and the widest is 16.
    16
}

fn sliced_size(array: &dyn Array) -> usize {
    use arrow_array::cast::AsArray;
    use arrow_schema::DataType as ArrowType;

    let nulls = array.nulls().map_or(0, |nulls| nulls.len().div_ceil(8));
    // A fixed-width leaf is its rows times its width, read off the datatype
    // so no `ArrayData` is built to count it.
    if let Some(width) = array.data_type().primitive_width() {
        return nulls + array.len() * width;
    }
    // A view's low 32 bits are its length; one of at most twelve bytes is
    // stored inline, and a longer one points at a data buffer.
    let viewed = |views: &[u128]| {
        views.len() * 16
            + views
                .iter()
                .map(|view| *view as u32 as usize)
                .filter(|length| *length > 12)
                .sum::<usize>()
    };
    match array.data_type() {
        ArrowType::Null => 0,
        ArrowType::Boolean => nulls + array.len().div_ceil(8),
        ArrowType::Utf8 => nulls + spanned(array.as_string::<i32>().offsets()),
        ArrowType::LargeUtf8 => nulls + spanned(array.as_string::<i64>().offsets()),
        ArrowType::Binary => nulls + spanned(array.as_binary::<i32>().offsets()),
        ArrowType::LargeBinary => nulls + spanned(array.as_binary::<i64>().offsets()),
        ArrowType::FixedSizeBinary(width) => {
            nulls + array.len() * usize::try_from(*width).unwrap_or(0)
        }
        ArrowType::Utf8View => nulls + viewed(array.as_string_view().views()),
        ArrowType::BinaryView => nulls + viewed(array.as_binary_view().views()),
        // A fixed-size list's slice already slices its values to the window.
        ArrowType::FixedSizeList(..) => {
            nulls + sliced_size(array.as_fixed_size_list().values().as_ref())
        }
        ArrowType::List(_) => {
            let list = array.as_list::<i32>();
            nulls + offsets_size(list.offsets(), list.values())
        }
        ArrowType::LargeList(_) => {
            let list = array.as_list::<i64>();
            nulls + offsets_size(list.offsets(), list.values())
        }
        ArrowType::Map(..) => {
            let map = array.as_map();
            let entries: ArrayRef = Arc::new(map.entries().clone());
            nulls + offsets_size(map.offsets(), &entries)
        }
        ArrowType::Struct(_) => {
            nulls
                + array
                    .as_struct()
                    .columns()
                    .iter()
                    .map(|child| sliced_size(child.as_ref()))
                    .sum::<usize>()
        }
        ArrowType::Dictionary(..) => {
            let dictionary = array.as_any_dictionary();
            sliced_size(dictionary.keys()) + dictionary.values().get_array_memory_size()
        }
        _ => {
            let whole = array.get_array_memory_size();
            array
                .to_data()
                .get_slice_memory_size()
                .map_or(whole, |sliced| sliced.min(whole))
        }
    }
}

/// A list's offsets and the child range they span.
fn offsets_size<O: arrow_array::OffsetSizeTrait>(
    offsets: &arrow_buffer::OffsetBuffer<O>,
    values: &ArrayRef,
) -> usize {
    let (first, length) = span(offsets);
    std::mem::size_of_val(offsets.as_ref()) + sliced_size(values.slice(first, length).as_ref())
}

/// A byte column's offsets and the bytes they span.
fn spanned<O: arrow_array::OffsetSizeTrait>(offsets: &arrow_buffer::OffsetBuffer<O>) -> usize {
    std::mem::size_of_val(offsets.as_ref()) + span(offsets).1
}

/// Where a sliced offset buffer's values begin, and how many it reaches.
fn span<O: arrow_array::OffsetSizeTrait>(
    offsets: &arrow_buffer::OffsetBuffer<O>,
) -> (usize, usize) {
    let first = offsets.first().map_or(0, |offset| offset.as_usize());
    let last = offsets.last().map_or(0, |offset| offset.as_usize());
    (first, last.saturating_sub(first))
}
