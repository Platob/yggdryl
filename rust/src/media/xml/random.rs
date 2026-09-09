//! Reading and writing one row at a time, by position.
//!
//! A row's bytes are contiguous, so its address is its span: a read is one
//! `pread` bounded by that span, and a write is one patch of it. A replacement
//! of the same length lands exactly where the old row was and touches nothing
//! else in the document; a different length moves only the bytes after the
//! row, in bounded chunks, so a document larger than memory is never held
//! whole. Every positional write restates the index rather than reading it
//! again, so a run of them costs one scan in total.

use std::borrow::Borrow;

use smol_str::SmolStr;

use crate::text::Limits;
use crate::{Error, IOBase, Result, Scalar};

use super::index::{RowIndex, RowSpan};
use super::options::XmlOptions;
use super::writer::{self, RowLayout};

/// How many bytes one shift moves at a time.
const SHIFT_CHUNK: u64 = crate::DEFAULT_STREAM_BATCH_SIZE as u64;

/// How far before a row its own layout whitespace is looked for.
const LAYOUT_LOOKBACK: u64 = 64;

/// Read one row's value out of the bytes its span names.
pub(crate) fn read_row_scalar<H: IOBase + ?Sized>(
    handle: &H,
    index: &RowIndex,
    row: u64,
) -> Result<Scalar> {
    let span = index.require(row)?;
    row_scalar(&read_span(handle, span)?, row)
}

/// Read a bounded run of rows, each out of its own span.
pub(crate) fn read_range_scalars<H: IOBase + ?Sized>(
    handle: &H,
    index: &RowIndex,
    offset: u64,
    count: usize,
) -> Result<Vec<Scalar>> {
    let end = offset.saturating_add(count as u64).min(index.len());
    let mut rows = Vec::with_capacity(usize::try_from(end.saturating_sub(offset)).unwrap_or(0));
    for row in offset..end {
        rows.push(read_row_scalar(handle, index, row)?);
    }
    Ok(rows)
}

/// Replace `count` rows at `offset` with `values`.
///
/// This is the one positional write: replacing a row is a splice of one,
/// removing it is a splice of none, and appending is a splice at the end. The
/// bytes that move are the rows being replaced plus whatever follows them, and
/// nothing before `offset` is read or written.
pub(crate) fn splice_rows<H, I>(
    handle: &mut H,
    index: &mut RowIndex,
    options: &XmlOptions,
    offset: u64,
    count: usize,
    values: I,
) -> Result<u64>
where
    H: IOBase + ?Sized,
    I: IntoIterator,
    I::Item: Borrow<Scalar>,
{
    open_document(handle, index, options)?;
    if offset > index.len() {
        return Err(Error::InvalidRecord {
            path: smol_str::format_smolstr!("$[{offset}]"),
            reason: crate::text::expected_got(
                format_args!("a row at or below {}", index.len()),
                offset,
            ),
        });
    }
    let position = usize::try_from(offset).unwrap_or(usize::MAX);
    let removed = count.min(index.len().saturating_sub(offset) as usize);
    let layout = RowLayout::from(options.formatting());
    let name = index
        .row()
        .unwrap_or_else(|| options.write_row())
        .to_owned();

    // The range starts at the whitespace that only introduces the first
    // replaced row, so a replacement re-emits its own layout and a removal
    // takes it away instead of leaving a blank line behind.
    let floor = position
        .checked_sub(1)
        .and_then(|previous| index.get(previous as u64))
        .map_or(0, |previous| previous.end);
    let anchor = index
        .get(offset)
        .map_or_else(|| index.content_end(), |span| span.start);
    let start = layout_start(handle, anchor, floor)?;
    let end = match removed.checked_sub(1) {
        Some(last) => index.require(offset + last as u64)?.end,
        None => start,
    };

    let mut rendered = Vec::new();
    let mut spans = Vec::new();
    let mut added = 0_u64;
    for value in values {
        let element = rendered.len() + layout.row_prefix_size();
        writer::write_row(&mut rendered, &name, value.borrow(), layout)?;
        spans.push((element, rendered.len()));
        added += 1;
    }

    index.set_row(&name);
    let delta = patch(handle, RowSpan { start, end }, &rendered)?;
    index.spliced(
        position,
        removed,
        spans.into_iter().map(|(from, to)| RowSpan {
            start: start + from as u64,
            end: start + to as u64,
        }),
        delta,
    );
    handle.flush()?;
    Ok(added)
}

/// Give the document a start and end tag a row can live between.
///
/// A handle holding nothing becomes an empty document, and a document element
/// written `<rows/>` becomes the pair `<rows></rows>`, so every later write is
/// the ordinary splice rather than a special case of its own.
fn open_document<H: IOBase + ?Sized>(
    handle: &mut H,
    index: &mut RowIndex,
    options: &XmlOptions,
) -> Result<()> {
    if index.root().is_some() && !index.is_self_closed() {
        return Ok(());
    }
    let root = index
        .root()
        .unwrap_or_else(|| options.write_root())
        .to_owned();
    let layout = RowLayout::from(options.formatting());
    let mut rendered = Vec::new();
    writer::write_document_start(&mut rendered, &root)?;
    let content_end = rendered.len() + layout.close_prefix_size();
    writer::write_document_end(&mut rendered, &root, layout)?;

    let start = index.content_end();
    let end = if index.is_self_closed() {
        index.byte_size()
    } else {
        start
    };
    patch(handle, RowSpan { start, end }, &rendered)?;
    index.opened(&root, start + content_end as u64, handle.size());
    Ok(())
}

/// Overwrite `span` with `bytes`, returning how far everything after it moved.
fn patch<H: IOBase + ?Sized>(handle: &mut H, span: RowSpan, bytes: &[u8]) -> Result<i64> {
    let size = handle.size();
    let current = i64::try_from(span.byte_size()).map_err(|_| addressing_error())?;
    let replacement = i64::try_from(bytes.len()).map_err(|_| addressing_error())?;
    let delta = replacement
        .checked_sub(current)
        .ok_or_else(addressing_error)?;

    if delta == 0 {
        // The write this whole design exists for: the replacement is the same
        // length, so it lands exactly where the old bytes were and no byte
        // after it is read or moved.
        if !bytes.is_empty() {
            handle.pwrite_all(span.start, bytes)?;
        }
        return Ok(0);
    }
    let tail = size.saturating_sub(span.end);
    if delta > 0 {
        // Growing: the tail moves right from its own end, so no byte is
        // overwritten before it has been read.
        let mut remaining = tail;
        while remaining > 0 {
            let chunk = remaining.min(SHIFT_CHUNK);
            let from = span.end + remaining - chunk;
            let moved = handle.read_range_bytes(from, chunk as usize)?;
            handle.pwrite_all(from + delta.unsigned_abs(), &moved)?;
            remaining -= chunk;
        }
        handle.pwrite_all(span.start, bytes)?;
        return Ok(delta);
    }
    // Shrinking: the replacement is written first and the tail follows it
    // forward, then the value ends where the last moved byte does.
    if !bytes.is_empty() {
        handle.pwrite_all(span.start, bytes)?;
    }
    let mut moved = 0_u64;
    while moved < tail {
        let chunk = (tail - moved).min(SHIFT_CHUNK);
        let from = span.end + moved;
        let bytes = handle.read_range_bytes(from, chunk as usize)?;
        handle.pwrite_all(from - delta.unsigned_abs(), &bytes)?;
        moved += chunk;
    }
    handle.truncate(size - delta.unsigned_abs())?;
    Ok(delta)
}

/// Return where a row's own introducing whitespace begins.
fn layout_start<H: IOBase + ?Sized>(handle: &H, start: u64, floor: u64) -> Result<u64> {
    let floor = floor.max(start.saturating_sub(LAYOUT_LOOKBACK));
    if floor >= start {
        return Ok(start);
    }
    let length = usize::try_from(start - floor).unwrap_or(usize::MAX);
    let bytes = handle.read_range_bytes(floor, length)?;
    let kept = bytes
        .iter()
        .rposition(|byte| !matches!(byte, b' ' | b'\t' | b'\r' | b'\n'))
        .map_or(0, |index| index + 1);
    Ok(floor + kept as u64)
}

/// Read the bytes one span names.
fn read_span<H: IOBase + ?Sized>(handle: &H, span: RowSpan) -> Result<Vec<u8>> {
    let length = usize::try_from(span.byte_size()).map_err(|_| addressing_error())?;
    handle.read_range_bytes(span.start, length)
}

/// Read one row element's bytes as the record a row is.
fn row_scalar(bytes: &[u8], row: u64) -> Result<Scalar> {
    let text = std::str::from_utf8(bytes).map_err(|error| Error::Codec {
        format: crate::text::xml::FORMAT,
        position: error.valid_up_to(),
        reason: "one row's bytes are not valid UTF-8".into(),
    })?;
    let document =
        crate::text::xml::parser::parse(text, Limits::default()).map_err(|error| match error {
            Error::Codec {
                format,
                position,
                reason,
            } => Error::InvalidRecord {
                path: smol_str::format_smolstr!("$[{row}]"),
                reason: smol_str::format_smolstr!("{format} at byte {position}: {reason}"),
            },
            other => other,
        })?;
    super::reader::row_record(crate::text::xml::content(document))
}

fn addressing_error() -> Error {
    Error::InvalidRecord {
        path: SmolStr::new_static("$"),
        reason: SmolStr::new_static("row byte size exceeds what one document can address"),
    }
}
