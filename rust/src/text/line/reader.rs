//! The byte-level reading: one reused window, records borrowed out of it.
//!
//! [`Window`] is the buffer every record is borrowed from. It is refilled from
//! the decoded stream and grown only when a record does not fit, so the
//! allocation count is a function of the *window*, never of the row count -
//! which is what makes a million-line read cost a constant amount.
//!
//! Nothing here holds a resource whole. The window is bounded, the stream is
//! pulled through it, and a record that straddles a refill boundary is handled
//! explicitly by growing the window rather than by silently buffering the rest
//! of the file.

use std::io::Read;

use crate::{Error, Result};

use super::sep::{LineSep, next_break};

/// The window's starting size, and the step it refills in.
///
/// One decoder output chunk: large enough that a scan amortizes the refill,
/// small enough that a handle addressing a resource larger than memory still
/// costs a constant amount.
const WINDOW: usize = 64 * 1024;

/// A bounded, reused buffer that records are borrowed out of.
///
/// Held in memory deliberately, and bounded on purpose: the whole point of the
/// line surface is that a season of logs larger than memory flows through it, so
/// what is retained is one window's worth plus, transiently, one record that did
/// not fit.
pub(crate) struct Window<R> {
    source: R,
    bytes: Vec<u8>,
    /// How much of `bytes` currently holds stream content.
    filled: usize,
    /// Where the next record starts within `bytes`.
    cursor: usize,
    /// Decoded bytes consumed before `cursor`, so an offset is absolute.
    consumed: u64,
    /// Whether the source has reported end of stream.
    drained: bool,
}

impl<R: Read> Window<R> {
    /// Wrap a decoded byte stream in a reused window.
    pub(crate) fn new(source: R) -> Self {
        Self {
            source,
            bytes: vec![0; WINDOW],
            filled: 0,
            cursor: 0,
            consumed: 0,
            drained: false,
        }
    }

    /// Move whatever is unread to the front and read more after it.
    ///
    /// Returns how far everything slid, so a caller mid-scan can rebase its own
    /// position. Compaction is what keeps the window reusable: the unread tail
    /// slides down rather than the buffer growing on every refill.
    fn refill(&mut self) -> Result<usize> {
        let shift = self.cursor;
        if shift > 0 {
            self.bytes.copy_within(shift..self.filled, 0);
            self.filled -= shift;
            self.cursor = 0;
        }
        if self.filled == self.bytes.len() {
            // The record under construction fills the window, so it straddles
            // every boundary this size can offer. Growing is the one case that
            // copies, and doubling keeps it amortized rather than per record.
            self.bytes.resize(self.bytes.len() * 2, 0);
        }
        let read = self
            .source
            .read(&mut self.bytes[self.filled..])
            .map_err(Error::Io)?;
        if read == 0 {
            self.drained = true;
        }
        self.filled += read;
        Ok(shift)
    }
}

/// One multi-line record, borrowed from the window.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Record<'window> {
    /// The record's bytes: its lines and their interior terminators, with the
    /// final terminator excluded.
    ///
    /// Interior terminators are kept exactly as they were read rather than
    /// normalized, because the record is one contiguous borrow of the window -
    /// normalizing would mean building a copy, and a round trip through
    /// [`Text`](crate::text::Text) reproduces the record either way.
    pub(crate) bytes: &'window [u8],
    /// The byte offset of the record's first line in the decoded stream.
    pub(crate) offset: u64,
    /// How many lines the record spans.
    pub(crate) lines: i32,
}

impl<R: Read> Window<R> {
    /// The next record: an opening line and every line until the next opener.
    ///
    /// `opens` decides what starts a record - a pattern match, a timestamp
    /// parse - and is asked about a line only when a record is already under
    /// construction, so the very first line always opens one. Lines before the
    /// first opener therefore form a preamble record rather than being dropped:
    /// a rotated file often opens mid-entry and the bytes are still the
    /// caller's data.
    ///
    /// The whole record plus one lookahead line is in the window at once, which
    /// is what lets the record be one contiguous borrow. A record larger than
    /// the window grows it - the one case that copies.
    pub(crate) fn next_record(
        &mut self,
        linesep: Option<&LineSep>,
        opens: &mut dyn FnMut(&[u8]) -> bool,
    ) -> Option<Result<Record<'_>>> {
        let offset = self.consumed;
        // Where the next line to examine begins, and where the record's
        // content ended - the two walk forward together.
        let mut scan = self.cursor;
        let mut content_end = self.cursor;
        let mut lines = 0_i32;
        let mut first = true;

        loop {
            let available = &self.bytes[scan..self.filled];
            match next_break(available, linesep, self.drained) {
                Some(found) => {
                    let line = &self.bytes[scan..scan + found.at];
                    let line = if first { strip_bom(line) } else { line };
                    if lines > 0 && opens(line) {
                        break;
                    }
                    first = false;
                    lines = lines.saturating_add(1);
                    content_end = scan + found.at;
                    scan += found.end();
                }
                None if self.drained => {
                    if scan < self.filled {
                        let line = &self.bytes[scan..self.filled];
                        let line = if first { strip_bom(line) } else { line };
                        // The final record needs no terminator.
                        if lines > 0 && opens(line) {
                            break;
                        }
                        lines = lines.saturating_add(1);
                        content_end = self.filled;
                        scan = self.filled;
                    }
                    break;
                }
                None => {
                    // Rebase the scan against the compaction the refill does.
                    match self.refill() {
                        Ok(shift) => {
                            scan -= shift;
                            content_end -= shift;
                        }
                        Err(error) => {
                            self.drained = true;
                            return Some(Err(error));
                        }
                    }
                }
            }
        }

        if lines == 0 {
            return None;
        }
        let start = self.cursor;
        self.consumed += (scan - start) as u64;
        self.cursor = scan;
        Some(Ok(Record {
            bytes: strip_bom_at(&self.bytes[start..content_end], offset),
            offset,
            lines,
        }))
    }
}

/// Strip the byte-order mark, but only from the resource's very first record.
fn strip_bom_at(bytes: &[u8], offset: u64) -> &[u8] {
    if offset == 0 { strip_bom(bytes) } else { bytes }
}

/// Strip a UTF-8 byte-order mark opening the resource.
///
/// A BOM is an encoding signature rather than content, so it is removed from
/// the first line and an anchored pattern still opens the file's first record.
/// The byte offsets keep counting it, so a seek stays exact.
pub(crate) fn strip_bom(bytes: &[u8]) -> &[u8] {
    bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes)
}
