//! One shared byte window yielding bounded physical-line parts.
//!
//! The window is the page every line of it is a range of. It is filled while
//! nothing points into it and handed out only once the fill is complete, so
//! sharing it costs a reference count per line rather than a copy of the
//! line's own bytes - which is what [`TextBytes`](super::TextBytes) means by
//! a page. A window a line still holds is never written over: the refill
//! takes a fresh one and moves the open tail into it, so the cost of a
//! retained line is one page per window and never one page per line.

use std::io::Read;
use std::ops::Range;
use std::sync::Arc;

use memchr::memmem::Finder;

use crate::{Error, Result};

use super::sep::{LineSep, finder_for, next_break};

const WINDOW_SIZE: usize = crate::DEFAULT_STREAM_BATCH_SIZE;

/// One piece of a physical line, as a range of the page that holds it.
pub(crate) struct LinePart {
    /// Exact source bytes excluding the physical terminator, as a range of
    /// the page [`Lines::page`] answers with for this part.
    pub(crate) range: Range<usize>,
    /// Whether this piece ends the physical line.
    pub(crate) end: bool,
}

impl LinePart {
    /// How many bytes this piece carries.
    pub(crate) const fn len(&self) -> usize {
        self.range.end - self.range.start
    }
}

/// A fixed-window streaming line splitter.
pub(crate) struct Lines<R> {
    source: R,
    /// The window, shared with every line cut from it.
    ///
    /// Sealed by construction: it is written only through
    /// [`Arc::get_mut`], which answers nothing once a line points into it,
    /// and the refill then takes a fresh window instead.
    page: Arc<Vec<u8>>,
    filled: usize,
    cursor: usize,
    drained: bool,
    failed: bool,
    line_open: bool,
    /// The searcher a pinned multi-byte terminator is scanned with, built
    /// from the configuration on the first pull and kept for the read.
    finder: Option<Finder<'static>>,
    /// Whether that searcher has been asked for yet.
    searched: bool,
}

impl<R: Read> Lines<R> {
    pub(crate) fn new(source: R) -> Self {
        Self {
            source,
            page: Arc::new(vec![0; WINDOW_SIZE]),
            filled: 0,
            cursor: 0,
            drained: false,
            failed: false,
            line_open: false,
            finder: None,
            searched: false,
        }
    }

    /// The page the ranges of the most recent part index into.
    ///
    /// Valid until the next [`Self::next_part`], which may retire it.
    pub(crate) const fn page(&self) -> &Arc<Vec<u8>> {
        &self.page
    }

    /// The window's bytes, for a reader working in the ranges it yields.
    pub(crate) fn window(&self) -> &[u8] {
        &self.page
    }

    /// Take a window of `size` bytes holding the open tail at its front.
    ///
    /// In place where nothing points into the current one, and into a fresh
    /// page otherwise: a window a line is a range of is that line's bytes,
    /// so moving them under it would rewrite what the line already read.
    fn rewind(&mut self, size: usize) {
        let open = self.filled - self.cursor;
        let grows = size > self.page.len();
        if !grows && Arc::get_mut(&mut self.page).is_some() {
            if self.cursor > 0 {
                if let Some(page) = Arc::get_mut(&mut self.page) {
                    page.copy_within(self.cursor..self.filled, 0);
                }
            }
        } else {
            let mut page = vec![0; size];
            page[..open].copy_from_slice(&self.page[self.cursor..self.filled]);
            self.page = Arc::new(page);
        }
        self.filled = open;
        self.cursor = 0;
    }

    fn refill(&mut self) -> Result<()> {
        self.rewind(self.page.len());
        let filled = self.filled;
        let read = match Arc::get_mut(&mut self.page) {
            // `rewind` leaves the window unshared, so this is the arm every
            // refill takes. The other is stated rather than assumed: a
            // window that somehow stayed shared must refuse rather than
            // report a short read, which would silently truncate the object.
            Some(page) => self.source.read(&mut page[filled..]).map_err(Error::Io)?,
            None => {
                return Err(Error::Io(std::io::Error::other(
                    "the text window was still shared when it came to be refilled",
                )));
            }
        };
        if read == 0 {
            self.drained = true;
        }
        self.filled += read;
        Ok(())
    }

    /// Yield the next bounded part, without retaining earlier parts.
    pub(crate) fn next_part(&mut self, linesep: Option<&LineSep>) -> Option<Result<LinePart>> {
        if self.failed {
            return None;
        }
        if !self.searched {
            self.finder = finder_for(linesep);
            self.searched = true;
        }
        loop {
            if let Some(found) = next_break(
                &self.page[self.cursor..self.filled],
                linesep,
                self.finder.as_ref(),
                self.drained,
            ) {
                let start = self.cursor;
                let end = start + found.at;
                self.cursor = start + found.end();
                self.line_open = false;
                return Some(Ok(LinePart {
                    range: start..end,
                    end: true,
                }));
            }
            if self.drained {
                if self.cursor < self.filled {
                    let start = self.cursor;
                    self.cursor = self.filled;
                    self.line_open = false;
                    return Some(Ok(LinePart {
                        range: start..self.filled,
                        end: true,
                    }));
                }
                if std::mem::take(&mut self.line_open) {
                    return Some(Ok(LinePart {
                        range: 0..0,
                        end: true,
                    }));
                }
                return None;
            }
            if self.cursor > 0 || self.filled < self.page.len() {
                if let Err(error) = self.refill() {
                    self.failed = true;
                    return Some(Err(error));
                }
                continue;
            }

            let overlap = linesep.map_or_else(
                || usize::from(self.page[self.filled - 1] == b'\r'),
                |linesep| linesep.len().saturating_sub(1),
            );
            let available = self.filled - self.cursor;
            let safe = available.saturating_sub(overlap);
            if safe == 0 {
                // A configured terminator may itself exceed the ordinary
                // window. Its length is explicit configuration, so retaining
                // one such candidate is the bound needed to recognize it.
                let size = self.page.len().saturating_add(WINDOW_SIZE).max(overlap + 1);
                self.rewind(size);
                continue;
            }
            let start = self.cursor;
            let end = start + safe;
            self.cursor = end;
            self.line_open = true;
            return Some(Ok(LinePart {
                range: start..end,
                end: false,
            }));
        }
    }

    /// Yield the next complete line without its terminator.
    #[cfg(test)]
    fn next_line(&mut self, linesep: Option<&LineSep>) -> Option<Result<Vec<u8>>> {
        let mut line = Vec::new();
        loop {
            let part = self.next_part(linesep)?;
            match part {
                Ok(part) => {
                    let end = part.end;
                    line.extend_from_slice(&self.window()[part.range]);
                    if end {
                        return Some(Ok(line));
                    }
                }
                Err(error) => return Some(Err(error)),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    use super::{Lines, WINDOW_SIZE};
    use crate::media::text::LineSep;

    struct Chunked {
        bytes: std::io::Cursor<Vec<u8>>,
        size: usize,
    }

    impl Read for Chunked {
        fn read(&mut self, target: &mut [u8]) -> std::io::Result<usize> {
            let size = target.len().min(self.size);
            self.bytes.read(&mut target[..size])
        }
    }

    fn read(input: &[u8], linesep: Option<&LineSep>) -> Vec<Vec<u8>> {
        let mut lines = Lines::new(std::io::Cursor::new(input));
        let mut values = Vec::new();
        while let Some(line) = lines.next_line(linesep) {
            values.push(line.unwrap());
        }
        values
    }

    #[test]
    fn flexible_and_pinned_terminators_stream_lines() {
        assert_eq!(
            read(b"a\r\nb\nc\rd", None),
            [b"a".to_vec(), b"b".to_vec(), b"c".to_vec(), b"d".to_vec()]
        );
        assert_eq!(
            read(b"a\nb\r\nc", Some(&LineSep::CRLF)),
            [b"a\nb".to_vec(), b"c".to_vec()]
        );
        assert_eq!(read(b"\xef\xbb\xbfa\n", None), [b"\xef\xbb\xbfa".to_vec()]);
    }

    #[test]
    fn terminators_can_cross_short_source_reads() {
        let mut flexible = Lines::new(Chunked {
            bytes: std::io::Cursor::new(b"a\r\nb\rc\nlast".to_vec()),
            size: 1,
        });
        let mut values = Vec::new();
        while let Some(line) = flexible.next_line(None) {
            values.push(line.unwrap());
        }
        assert_eq!(
            values,
            [
                b"a".to_vec(),
                b"b".to_vec(),
                b"c".to_vec(),
                b"last".to_vec()
            ]
        );

        let mut pinned = Lines::new(Chunked {
            bytes: std::io::Cursor::new(b"a\r\nb\r\nc".to_vec()),
            size: 1,
        });
        let mut values = Vec::new();
        while let Some(line) = pinned.next_line(Some(&LineSep::CRLF)) {
            values.push(line.unwrap());
        }
        assert_eq!(values, [b"a".to_vec(), b"b".to_vec(), b"c".to_vec()]);
    }

    #[test]
    fn a_line_larger_than_the_window_is_returned_as_bounded_parts() {
        let mut input = vec![b'x'; WINDOW_SIZE * 3 + 7];
        input.extend_from_slice(b"\nnext");
        let mut lines = Lines::new(std::io::Cursor::new(input));
        let mut sizes = Vec::new();
        loop {
            let part = lines.next_part(None).unwrap().unwrap();
            sizes.push(part.len());
            if part.end {
                break;
            }
        }
        assert!(sizes.len() >= 3);
        assert!(sizes.iter().all(|size| *size <= WINDOW_SIZE));
        assert_eq!(lines.next_line(None).unwrap().unwrap(), b"next");
    }
}
