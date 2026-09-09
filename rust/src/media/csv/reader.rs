//! Logical CSV records over the shared physical line splitter.
//!
//! One record is one physical line, except when a quoted cell holds the record
//! terminator: then the reader keeps joining physical lines - with the exact
//! bytes that separated them - until the cell scan reports every quote closed.
//! Nothing is retained beyond the current record and the splitter's own window.

use std::io::Read;

use smol_str::format_smolstr;

use crate::media::text::LineSep;
use crate::media::text::reader::Lines;
use crate::{Error, Result};

use super::scan::{Dialect, Span, split_cells};

/// One logical record, borrowed from the reader that produced it.
pub(crate) struct Record<'record> {
    /// The record's bytes, its terminator excluded and joined lines included.
    pub(crate) bytes: &'record [u8],
    /// Where each cell sits inside those bytes.
    pub(crate) spans: &'record [Span],
    /// Decoded byte offset of the record's first byte.
    pub(crate) offset: u64,
    /// Decoded bytes from that offset to the start of the next record.
    pub(crate) stride: u64,
}

/// A streaming logical-record splitter over any decoded byte source.
pub(crate) struct Records<R> {
    lines: Lines<R>,
    dialect: Dialect,
    /// The record being assembled. It grows only while a quote is open, and
    /// only up to `max_record_byte_size`, which is the bound that keeps an
    /// unclosed quote in an adversarial resource from joining the whole of it
    /// into one value.
    bytes: Vec<u8>,
    spans: Vec<Span>,
    terminator: Vec<u8>,
    position: u64,
    offset: u64,
    stride: u64,
    done: bool,
}

impl<R: Read> Records<R> {
    /// Split `source` from decoded byte offset `position`.
    pub(crate) fn new(source: R, dialect: Dialect, position: u64) -> Self {
        Self {
            lines: Lines::new(source),
            dialect,
            bytes: Vec::new(),
            spans: Vec::new(),
            terminator: Vec::new(),
            position,
            offset: position,
            stride: 0,
            done: false,
        }
    }

    /// Yield the next record, or `None` once the source is drained.
    ///
    /// Blank lines and comment lines carry no record: they advance the byte
    /// position and are skipped, so a record's offset always names a record.
    pub(crate) fn next_record(&mut self) -> Option<Result<Record<'_>>> {
        if self.done {
            return None;
        }
        loop {
            self.bytes.clear();
            self.terminator.clear();
            let start = self.position;
            match read_line(
                &mut self.lines,
                self.dialect.linesep.as_ref(),
                &mut self.bytes,
                &mut self.terminator,
            ) {
                None => {
                    self.done = true;
                    return None;
                }
                Some(Err(error)) => {
                    self.done = true;
                    return Some(Err(error));
                }
                Some(Ok(())) => {}
            }
            self.position = start + self.bytes.len() as u64 + self.terminator.len() as u64;
            let skip = self.bytes.is_empty()
                || self
                    .dialect
                    .comment
                    .is_some_and(|comment| self.bytes[0] == comment);
            if skip {
                continue;
            }
            // A quoted cell may hold the terminator, so the record is complete
            // only when the scan says every quote it opened also closed. The
            // terminator that ended the joined line becomes content, which is
            // why the splitter reports the bytes it found rather than the ones
            // a write would have produced.
            while !split_cells(&self.bytes, &self.dialect, &mut self.spans) {
                if let Some(bound) = self.dialect.max_record_byte_size {
                    if self.bytes.len() as u64 > bound {
                        self.done = true;
                        return Some(Err(Error::InvalidRecord {
                            path: format_smolstr!("$[{start}]"),
                            reason: format_smolstr!(
                                "expected a record within {bound} bytes, got one still open \
                                 after {} from byte {start}",
                                self.bytes.len()
                            ),
                        }));
                    }
                }
                self.bytes.extend_from_slice(&self.terminator);
                self.terminator.clear();
                match read_line(
                    &mut self.lines,
                    self.dialect.linesep.as_ref(),
                    &mut self.bytes,
                    &mut self.terminator,
                ) {
                    // The resource ended inside a quoted cell. What is there is
                    // the record: refusing it would lose every complete row a
                    // truncated file still holds.
                    None => break,
                    Some(Err(error)) => {
                        self.done = true;
                        return Some(Err(error));
                    }
                    Some(Ok(())) => {}
                }
            }
            self.offset = start;
            self.stride = self.bytes.len() as u64 + self.terminator.len() as u64;
            self.position = start + self.stride;
            return Some(Ok(Record {
                bytes: &self.bytes,
                spans: &self.spans,
                offset: self.offset,
                stride: self.stride,
            }));
        }
    }
}

/// Append one complete physical line, reporting the terminator that ended it.
///
/// A line longer than the splitter's window arrives in parts; they are joined
/// here so a record scan always sees whole lines.
fn read_line<R: Read>(
    lines: &mut Lines<R>,
    linesep: Option<&LineSep>,
    bytes: &mut Vec<u8>,
    terminator: &mut Vec<u8>,
) -> Option<Result<()>> {
    let mut opened = false;
    loop {
        match lines.next_part(linesep) {
            None => {
                return if opened { Some(Ok(())) } else { None };
            }
            Some(Err(error)) => return Some(Err(error)),
            Some(Ok(part)) => {
                opened = true;
                bytes.extend_from_slice(part.bytes);
                if part.end {
                    terminator.extend_from_slice(part.terminator);
                    return Some(Ok(()));
                }
            }
        }
    }
}
