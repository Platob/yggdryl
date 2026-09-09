//! Positional record and cell access, reading and writing.
//!
//! A record is reached through the sparse index: one positional read at the
//! nearest anchor, then a bounded forward scan. A write is a byte splice - the
//! cell or record's exact range replaced by its new spelling - so changing one
//! value costs the bytes after it, not a re-encode of the whole resource, and
//! a same-width replacement costs one positional write.
//!
//! Positional writes need an uncoded resource. A coding's output offset is not
//! addressable in its input, so splicing compressed bytes at a decoded offset
//! would corrupt the stream; that is refused by name rather than attempted.

use smol_str::{SmolStr, format_smolstr};

use crate::{Codec, Error, IOBase, Result};

use super::reader::Records;
use super::scan::{Dialect, Span};

/// One record read positionally, owned by whoever asked for it.
#[derive(Clone, Debug)]
pub(crate) struct OwnedRecord {
    /// The record's bytes, its terminator excluded.
    pub(crate) bytes: Vec<u8>,
    /// Where each cell sits inside those bytes.
    pub(crate) spans: Vec<Span>,
    /// Decoded byte offset of the record's first byte.
    pub(crate) offset: u64,
    /// Decoded bytes from that offset to the start of the next record.
    pub(crate) stride: u64,
}

impl OwnedRecord {
    /// Return where one cell sits in the resource, not in the record.
    pub(crate) fn cell_range(&self, column: usize) -> Option<(u64, u64)> {
        let span = self.spans.get(column)?;
        Some((
            self.offset + span.start as u64,
            self.offset + span.end as u64,
        ))
    }
}

/// Read the record `skip` records past `offset`, or `None` past the end.
///
/// # Errors
///
/// Returns a positional read or decoding failure.
pub(crate) fn read_record(
    handle: &(impl IOBase + ?Sized),
    dialect: &Dialect,
    offset: u64,
    skip: u64,
) -> Result<Option<OwnedRecord>> {
    let source = crate::media::stream::decoded_reader_at(handle, offset)?;
    let mut records = Records::new(source, dialect.clone(), offset);
    for _ in 0..skip {
        match records.next_record() {
            None => return Ok(None),
            Some(Err(error)) => return Err(error),
            Some(Ok(_)) => {}
        }
    }
    match records.next_record() {
        None => Ok(None),
        Some(Err(error)) => Err(error),
        Some(Ok(record)) => Ok(Some(OwnedRecord {
            bytes: record.bytes.to_vec(),
            spans: record.spans.to_vec(),
            offset: record.offset,
            stride: record.stride,
        })),
    }
}

/// Replace the bytes in `start..end` with `payload`, moving what follows.
///
/// A replacement of the same width is one positional write. A different width
/// moves the tail in bounded chunks - backwards when the value grows, forwards
/// when it shrinks - so the resource is never held in memory.
///
/// # Errors
///
/// Returns a refusal for a coded resource, or a positional read or write
/// failure.
pub(crate) fn splice(
    handle: &mut (impl IOBase + ?Sized),
    start: u64,
    end: u64,
    payload: &[u8],
) -> Result<()> {
    require_positional(handle)?;
    let size = handle.size();
    let end = end.min(size);
    let start = start.min(end);
    let removed = end - start;
    let added = payload.len() as u64;
    if added == removed {
        handle.pwrite_all(start, payload)?;
        return handle.flush();
    }
    if added < removed {
        handle.pwrite_all(start, payload)?;
        shift_left(handle, end, start + added, size)?;
        handle.truncate(size - (removed - added))?;
        return handle.flush();
    }
    shift_right(handle, end, start + added, size)?;
    handle.pwrite_all(start, payload)?;
    handle.flush()
}

/// Move `from..size` down to `to`, forwards, one bounded window at a time.
fn shift_left(handle: &mut (impl IOBase + ?Sized), from: u64, to: u64, size: u64) -> Result<()> {
    let mut window = vec![0_u8; crate::DEFAULT_STREAM_BATCH_SIZE];
    let mut read_at = from;
    let mut write_at = to;
    while read_at < size {
        let want = usize::try_from(size - read_at)
            .unwrap_or(window.len())
            .min(window.len());
        let read = handle.pread(read_at, &mut window[..want])?;
        if read == 0 {
            break;
        }
        handle.pwrite_all(write_at, &window[..read])?;
        read_at += read as u64;
        write_at += read as u64;
    }
    Ok(())
}

/// Move `from..size` up to `to`, backwards, so the copy never overwrites
/// bytes it has not read yet.
fn shift_right(handle: &mut (impl IOBase + ?Sized), from: u64, to: u64, size: u64) -> Result<()> {
    let mut window = vec![0_u8; crate::DEFAULT_STREAM_BATCH_SIZE];
    let mut left = size - from;
    while left > 0 {
        let want = usize::try_from(left)
            .unwrap_or(window.len())
            .min(window.len());
        let read_at = from + left - want as u64;
        handle.pread_exact(read_at, &mut window[..want])?;
        handle.pwrite_all(to + left - want as u64, &window[..want])?;
        left -= want as u64;
    }
    Ok(())
}

/// Refuse a positional write a coding would corrupt.
fn require_positional(handle: &(impl IOBase + ?Sized)) -> Result<()> {
    let codec = handle.codec();
    if codec == Codec::Identity {
        return Ok(());
    }
    Err(Error::InvalidRecord {
        path: SmolStr::new_static("$.encoding"),
        reason: format_smolstr!(
            "expected an uncoded resource to write positionally, got {codec} - \
             a coding's output offset is not addressable in its input, so rewrite \
             the rows instead"
        ),
    })
}
