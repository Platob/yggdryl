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
    ///
    /// Behind a pointer for the alignment [`finder_for`] states: a reader
    /// holding a `Finder` inline asks to be aligned to 32 bytes, which the
    /// object allocator a binding hands it to does not give.
    finder: Option<Box<Finder<'static>>>,
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
                self.finder.as_deref(),
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
    #[cfg(feature = "internals")]
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

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/text/reader.rs` pins and a caller cannot reach.
    //!
    //! The window is an allocation strategy, not a contract: a caller reads
    //! [`TextLine`](crate::text::TextLine)s and never the page they are ranges
    //! of. Both doors below wrap the real reader instead of publishing it, so
    //! `Lines` and `LinePart` keep their `pub(crate)` visibility and a part is
    //! answered as what a test measures - its size, and whether it ends the
    //! physical line.

    use std::io::Read;

    use crate::Result;
    use crate::text::LineSep;

    /// The fixed window one page of read bytes is bounded to.
    pub const WINDOW_SIZE: usize = super::WINDOW_SIZE;

    /// One piece of a physical line, measured rather than borrowed.
    pub struct LinePart {
        /// Whether this piece ends the physical line.
        pub end: bool,
        size: usize,
    }

    impl LinePart {
        /// How many bytes this piece carries.
        #[must_use]
        pub const fn len(&self) -> usize {
            self.size
        }

        /// Whether this piece carries no bytes at all.
        #[must_use]
        pub const fn is_empty(&self) -> bool {
            self.size == 0
        }
    }

    /// A fixed-window streaming line splitter.
    pub struct Lines<R>(super::Lines<R>);

    impl<R: Read> Lines<R> {
        /// Split `source` into physical lines through one shared window.
        pub fn new(source: R) -> Self {
            Self(super::Lines::new(source))
        }

        /// Yield the next bounded piece of a physical line.
        pub fn next_part(&mut self, linesep: Option<&LineSep>) -> Option<Result<LinePart>> {
            self.0.next_part(linesep).map(|part| {
                part.map(|part| LinePart {
                    end: part.end,
                    size: part.len(),
                })
            })
        }

        /// Yield the next complete line without its terminator.
        pub fn next_line(&mut self, linesep: Option<&LineSep>) -> Option<Result<Vec<u8>>> {
            self.0.next_line(linesep)
        }
    }
}
