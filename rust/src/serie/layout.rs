//! The buffer edits every column leaf shares: a validity bitmap and an
//! offsets cut, each spliced in place where the leaf owns the bytes and
//! rebuilt once where it does not.
//!
//! Arrow hands a buffer back mutable only when nothing else holds it and its
//! pointer was never advanced, which is what `into_mutable` checks; every
//! helper here tries that first and falls back to one rebuild through the
//! matching builder. A leaf never splices through a kernel: a kernel-made
//! buffer is 64-byte aligned and refuses `into_vec`, so every later edit
//! would copy again.

use std::ops::Range;

use arrow_array::OffsetSizeTrait;
use arrow_buffer::{
    BooleanBuffer, BooleanBufferBuilder, MutableBuffer, NullBuffer, OffsetBuffer, ScalarBuffer,
};

use crate::Result;

/// The validity bitmap `nulls` is with `range` replaced by `present`, or
/// `None` when no row is absent afterwards.
///
/// `len` is the row count `nulls` describes (a bitmap absent because every
/// row is present still has a length). Appending at `len` writes into the
/// bitmap's own bytes when the leaf holds them alone: `sliced()` byte-slices
/// an aligned bitmap and copies an unaligned one, so a bitmap whose bit
/// offset is not zero never reaches `new_from_buffer`, and `into_mutable`
/// refuses a shared or pointer-advanced buffer. Anything else is one rebuild
/// from prefix, replacement and suffix.
pub(crate) fn splice_nulls(
    nulls: Option<NullBuffer>,
    len: usize,
    range: Range<usize>,
    present: &[bool],
) -> Option<NullBuffer> {
    let kept_absent = nulls.as_ref().map_or(0, |held| {
        held.null_count() - held.slice(range.start, range.len()).null_count()
    });
    if kept_absent == 0 && present.iter().all(|row| *row) {
        return None;
    }
    let new_len = len - range.len() + present.len();
    let mut builder = match nulls {
        Some(held) if range.start == len => match held.into_inner().sliced().into_mutable() {
            Ok(owned) => BooleanBufferBuilder::new_from_buffer(owned, len),
            Err(shared) => {
                let mut builder = BooleanBufferBuilder::new(new_len);
                builder.append_buffer(&BooleanBuffer::new(shared, 0, len));
                builder
            }
        },
        Some(held) => {
            let mut builder = BooleanBufferBuilder::new(new_len);
            builder.append_buffer(&held.inner().slice(0, range.start));
            builder.append_slice(present);
            builder.append_buffer(&held.inner().slice(range.end, len - range.end));
            return Some(NullBuffer::new(builder.finish()));
        }
        None => {
            let mut builder = BooleanBufferBuilder::new(new_len);
            builder.append_n(range.start, true);
            builder.append_slice(present);
            builder.append_n(len - range.end, true);
            return Some(NullBuffer::new(builder.finish()));
        }
    };
    builder.append_slice(present);
    Some(NullBuffer::new(builder.finish()))
}

/// The values bitmap `bits` is with `range` replaced by `replacement`.
///
/// `len` is the row count `bits` describes. One bit over one bit is one bit
/// write, and appending at `len` writes into the bitmap's own bytes, both
/// where the leaf holds them alone (the rule [`splice_nulls`] states);
/// anything else is one rebuild from prefix, replacement and suffix.
pub(crate) fn splice_bits(
    bits: BooleanBuffer,
    len: usize,
    range: Range<usize>,
    replacement: &BooleanBuffer,
) -> BooleanBuffer {
    let new_len = len - range.len() + replacement.len();
    if range.start == len || (range.len() == 1 && replacement.len() == 1) {
        let one_bit = range.start != len;
        let mut builder = match bits.sliced().into_mutable() {
            Ok(owned) => BooleanBufferBuilder::new_from_buffer(owned, len),
            Err(shared) => {
                let mut builder = BooleanBufferBuilder::new(new_len);
                builder.append_buffer(&BooleanBuffer::new(shared, 0, len));
                builder
            }
        };
        if one_bit {
            builder.set_bit(range.start, replacement.value(0));
        } else {
            builder.append_buffer(replacement);
        }
        return builder.finish();
    }
    let mut builder = BooleanBufferBuilder::new(new_len);
    builder.append_buffer(&bits.slice(0, range.start));
    builder.append_buffer(replacement);
    builder.append_buffer(&bits.slice(range.end, len - range.end));
    builder.finish()
}

/// The offsets cut with rows `range` replaced by rows of `lengths` items,
/// and the item range the replaced rows occupied.
///
/// Infallible because `check` already ran [`require_offset`] on the total
/// the cut will reach. Appending at the end writes into the cut's own bytes
/// when the leaf holds them alone; a sliced cut has an advanced pointer, so
/// `into_mutable` refuses it and the rebuild runs - which the door's
/// rebasing guarantees never happens for a column it built.
pub(crate) fn splice_offsets<O: OffsetSizeTrait>(
    offsets: OffsetBuffer<O>,
    range: Range<usize>,
    lengths: &[usize],
) -> (OffsetBuffer<O>, Range<usize>) {
    let len = offsets.len() - 1;
    let start = offsets[range.start].as_usize();
    let end = offsets[range.end].as_usize();
    let replaced = start..end;
    if range.start == len {
        let mut last = offsets[len];
        let mut bytes = match offsets.into_inner().into_inner().into_mutable() {
            Ok(owned) => owned,
            Err(shared) => {
                let mut owned = MutableBuffer::with_capacity(
                    (len + 1 + lengths.len()) * std::mem::size_of::<O>(),
                );
                owned.extend_from_slice(shared.typed_data::<O>());
                owned
            }
        };
        for items in lengths {
            last += O::usize_as(*items);
            bytes.push(last);
        }
        return (OffsetBuffer::new(ScalarBuffer::from(bytes)), replaced);
    }
    let mut rebuilt: Vec<O> = Vec::with_capacity(len + 1 - range.len() + lengths.len());
    rebuilt.extend_from_slice(&offsets[..=range.start]);
    let mut last = offsets[range.start];
    for items in lengths {
        last += O::usize_as(*items);
        rebuilt.push(last);
    }
    let shift = last - offsets[range.end];
    rebuilt.extend(
        offsets[range.end + 1..]
            .iter()
            .map(|offset| *offset + shift),
    );
    (OffsetBuffer::new(ScalarBuffer::from(rebuilt)), replaced)
}

/// Refuse an item total the offset type cannot reach, naming the column.
///
/// # Errors
///
/// Returns an error when `total` is past `O::MAX`.
pub(crate) fn require_offset<O: OffsetSizeTrait>(name: &str, total: usize) -> Result<()> {
    if O::from_usize(total).is_some() {
        return Ok(());
    }
    Err(crate::Error::InvalidRecord {
        path: smol_str::SmolStr::new(name),
        reason: smol_str::format_smolstr!(
            "{total} items are past the {} an offset of {} bytes reaches",
            O::MAX_OFFSET,
            std::mem::size_of::<O>()
        ),
    })
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/serie/layout.rs` pins and a caller cannot reach.

    use std::ops::Range;

    use arrow_array::OffsetSizeTrait;
    use arrow_buffer::{BooleanBuffer, NullBuffer, OffsetBuffer};

    use crate::Result;

    pub fn splice_nulls(
        nulls: Option<NullBuffer>,
        len: usize,
        range: Range<usize>,
        present: &[bool],
    ) -> Option<NullBuffer> {
        super::splice_nulls(nulls, len, range, present)
    }

    pub fn splice_offsets<O: OffsetSizeTrait>(
        offsets: OffsetBuffer<O>,
        range: Range<usize>,
        lengths: &[usize],
    ) -> (OffsetBuffer<O>, Range<usize>) {
        super::splice_offsets(offsets, range, lengths)
    }

    pub fn require_offset<O: OffsetSizeTrait>(name: &str, total: usize) -> Result<()> {
        super::require_offset::<O>(name, total)
    }

    pub fn splice_bits(
        bits: BooleanBuffer,
        len: usize,
        range: Range<usize>,
        replacement: &BooleanBuffer,
    ) -> BooleanBuffer {
        super::splice_bits(bits, len, range, replacement)
    }
}
