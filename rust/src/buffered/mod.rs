//! A page cache that makes any [`IOBase`] handle buffered.
//!
//! [`Buffered`] wraps one handle and serves its reads from fixed-size pages.
//! A miss fetches the covering pages with page-aligned reads from the inner
//! handle and keeps them; a hit copies out of memory and touches nothing. It
//! is a wrapping handle like [`Coded`](crate::io::Coded): it owns its inner
//! handle, mirrors everything it does not change, and is invisible except for
//! speed - `size`, `url`, `media_type`, `kind`, `parent`, `child_by`, and `ls`
//! all answer exactly what the wrapped handle answers.
//!
//! ```
//! use yggdryl::buffered::{Buffered, BufferedOptions};
//! use yggdryl::io::{Buffer, IOBase};
//!
//! # fn main() -> yggdryl::Result<()> {
//! let options = BufferedOptions::default().with_page_size(256);
//! let mut handle = Buffered::new(Buffer::new(), options);
//! handle.write_all_bytes(&vec![7_u8; 1_024])?;
//!
//! // Per the laziness contract, nothing is cached until a read asks for it.
//! assert_eq!(handle.cached_pages(), 0);
//!
//! // The first read fetches the page holding the range; the second is served
//! // from it, and a read spanning two pages caches both.
//! assert_eq!(handle.read_range(0, 16)?.len(), 16);
//! assert_eq!(handle.cached_pages(), 1);
//! assert_eq!(handle.read_range(0, 16)?.len(), 16);
//! assert_eq!(handle.cached_pages(), 1);
//! assert_eq!(handle.read_range(250, 12)?.len(), 12);
//! assert_eq!(handle.cached_pages(), 2);
//! # Ok(())
//! # }
//! ```
//!
//! # What the cache promises
//!
//! - **Reads are the only thing it changes.** A [`IOBase::pwrite`] goes
//!   straight through to the inner handle and then patches or drops every page
//!   it overlapped, so a read after a write through the same wrapper is never
//!   stale. [`IOBase::truncate`] delegates and invalidates everything at or
//!   past the new size, [`IOBase::flush`] delegates, and [`IOBase::close`]
//!   flushes and drops the whole cache.
//! - **Absence stays absence.** A read past the end returns `0` without
//!   touching the inner handle, a short read at the true end is remembered as
//!   short, and a handle with no bytes yet caches nothing - the laziness
//!   contract in [`crate::io`] holds unchanged.
//! - **Both ends of the value are pinned.** The first page and the page
//!   holding the last byte are exempt from eviction and from expiry, because
//!   that is where discovery lives: magic bytes at the head, a Parquet footer
//!   or an Arrow IPC schema block at the tail. They are still filled lazily -
//!   pinning is a retention guarantee, never a prefetch.
//!
//! ```
//! use yggdryl::buffered::{Buffered, BufferedOptions};
//! use yggdryl::io::{Buffer, IOBase};
//!
//! # fn main() -> yggdryl::Result<()> {
//! // Four pages of budget over a sixteen-page value: the middle cannot all
//! // stay, but the two ends never leave.
//! let options = BufferedOptions::default()
//!     .with_page_size(64)
//!     .with_max_bytes(4 * 64);
//! let mut handle = Buffered::new(Buffer::from_bytes(vec![1_u8; 16 * 64]), options);
//!
//! // Footer first, then the header - the shape a container is opened with.
//! handle.read_range(16 * 64 - 8, 8)?;
//! handle.read_range(0, 8)?;
//!
//! // Then a scan of the middle, far past what the budget can hold.
//! for page in 1..15 {
//!     handle.read_range(page * 64, 8)?;
//! }
//!
//! assert!(handle.cached_bytes() <= handle.options().max_bytes());
//! assert!(handle.has_cached_page(0));
//! assert!(handle.has_cached_page(15));
//! # Ok(())
//! # }
//! ```
//!
//! # Wrapping is idempotent
//!
//! [`IOBase::buffered`] wraps any handle, and [`Buffered`] shadows it with an
//! inherent method of its own, so buffering an already-buffered handle
//! re-wraps the handle it holds instead of stacking a second cache:
//!
//! ```
//! use yggdryl::buffered::BufferedOptions;
//! use yggdryl::io::{Buffer, IOBase};
//!
//! # fn main() -> yggdryl::Result<()> {
//! let once = Buffer::new().buffered(BufferedOptions::default());
//! // The inherent method wins method resolution, so this is the same type.
//! let twice = once.buffered(BufferedOptions::default().with_page_size(4_096));
//! assert_eq!(twice.options().page_size(), 4_096);
//!
//! let inner: Buffer = twice.into_handle();
//! assert_eq!(inner.size(), 0);
//! # Ok(())
//! # }
//! ```

mod options;
mod page;

use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Instant;

pub use options::BufferedOptions;
use page::{PageTable, Pinned};

use crate::Result;
use crate::io::IOBase;

/// A page cache over one handle.
///
/// Reads are served from pages of [`BufferedOptions::page_size`] bytes, kept
/// under a byte budget with least-recently-used eviction and discarded once
/// they outlive [`BufferedOptions::ttl`] measured from their last access. The
/// first page and the page holding the last byte are pinned: they survive both.
///
/// The cache state sits behind a `Mutex` because [`IOBase::pread`] takes
/// `&self` while a hit has to record the access and a miss has to insert. One
/// operation takes the lock exactly once.
#[derive(Debug)]
pub struct Buffered<H: IOBase> {
    handle: H,
    options: BufferedOptions,
    pages: Mutex<PageTable>,
}

impl<H: IOBase> Buffered<H> {
    /// Wrap a handle in a page cache without touching it.
    pub fn new(handle: H, options: BufferedOptions) -> Self {
        Self {
            handle,
            options,
            pages: Mutex::new(PageTable::default()),
        }
    }

    /// Re-wrap the held handle with different options.
    ///
    /// This shadows [`IOBase::buffered`], which is the point: an inherent
    /// method wins method resolution, so buffering a handle that is already
    /// buffered replaces this cache rather than stacking a second one on top
    /// of it.
    #[must_use]
    pub fn buffered(self, options: BufferedOptions) -> Self {
        Self::new(self.into_handle(), options)
    }

    /// Return how this cache is configured.
    pub const fn options(&self) -> &BufferedOptions {
        &self.options
    }

    /// Borrow the wrapped handle.
    pub const fn handle(&self) -> &H {
        &self.handle
    }

    /// Borrow the wrapped handle mutably, dropping the cache first.
    ///
    /// Bytes written straight to the inner handle are invisible to this
    /// wrapper, so every cached page is discarded here rather than left to go
    /// stale. The borrow ends before this handle can be read again, so the
    /// next read re-fetches whatever the caller left behind.
    pub fn handle_mut(&mut self) -> &mut H {
        self.clear_cache();
        &mut self.handle
    }

    /// Consume the wrapper and return the handle it held.
    pub fn into_handle(self) -> H {
        self.handle
    }

    /// Return the bytes the cached pages occupy together.
    ///
    /// Pinned pages count, which is why [`BufferedOptions`] keeps the budget
    /// at two pages or more.
    pub fn cached_bytes(&self) -> u64 {
        self.table().bytes()
    }

    /// Return how many pages are cached.
    pub fn cached_pages(&self) -> usize {
        self.table().len()
    }

    /// Return whether the page at `index` is cached, without touching it.
    ///
    /// Reading this does not count as an access, so it never keeps a page
    /// alive that the time to live would otherwise have taken.
    pub fn has_cached_page(&self, index: u64) -> bool {
        self.table().contains(index)
    }

    /// Discard every cached page, pinned ones included.
    pub fn clear_cache(&mut self) {
        self.table().clear();
    }

    /// [`IOBase::pread`] against an explicit clock.
    ///
    /// The public read stamps `Instant::now()`; taking the instant as an
    /// argument is what lets the expiry rules be tested by advancing a clock
    /// rather than by sleeping.
    pub(crate) fn read_at(&self, offset: u64, buffer: &mut [u8], now: Instant) -> Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        let size = self.handle.size();
        if offset >= size {
            // Past the end is emptiness, and it is answered without reaching
            // the inner handle at all.
            return Ok(0);
        }
        let page_size = self.options.page_size_u64();
        let wanted = (buffer.len() as u64).min(size - offset);
        let pinned = Pinned::for_size(size, page_size);
        let ttl = self.options.ttl();

        let mut table = self.table();
        let mut filled: u64 = 0;
        while filled < wanted {
            let position = offset + filled;
            let index = self.options.page_index(position);
            let page_start = self.options.page_start(index);
            let within = slot(position - page_start);
            let take = slot(wanted - filled).min(self.options.page_size() - within);

            // The copy happens inside the lookup so the borrow of the table
            // ends before the miss path needs it again.
            let hit = table
                .get(index, now, ttl, pinned)
                .map(|page| copy_out(page, within, take, &mut buffer[slot(filled)..]));
            let copied = match hit {
                Some(copied) => copied,
                None => {
                    let fetched = self.fetch(page_start)?;
                    let copied = copy_out(&fetched, within, take, &mut buffer[slot(filled)..]);
                    // An empty fetch is the end of the value; caching it would
                    // record an absence the next write has to undo.
                    if !fetched.is_empty() {
                        table.insert(index, fetched, now, &self.options, pinned);
                    }
                    copied
                }
            };
            filled += copied as u64;
            if copied < take {
                // The page ended before the request did, which only happens at
                // the end of the value: a short read, exactly as `pread` says.
                break;
            }
        }
        Ok(slot(filled))
    }

    /// Read one page-aligned page from the inner handle.
    ///
    /// The result is shorter than a page only at the end of the value, which
    /// is what makes a short page the record that the value ends there.
    fn fetch(&self, start: u64) -> Result<Vec<u8>> {
        let mut page = vec![0_u8; self.options.page_size()];
        let mut filled = 0;
        while filled < page.len() {
            let read = self
                .handle
                .pread(start + filled as u64, &mut page[filled..])?;
            if read == 0 {
                break;
            }
            filled += read;
        }
        if filled < page.len() {
            page.truncate(filled);
            // The budget counts what a page holds, so a short page gives back
            // the allocation it did not need.
            page.shrink_to_fit();
        }
        Ok(page)
    }

    /// Borrow the cache, treating a poisoned lock as the state it holds.
    ///
    /// A panic elsewhere must not turn every later read into an error: the
    /// pages are plain bytes, and the worst a poisoned lock can mean here is a
    /// page that was mid-insert, which the next miss re-fetches.
    fn table(&self) -> MutexGuard<'_, PageTable> {
        self.pages.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Copy the part of `page` at `within` into `target`, returning what fit.
fn copy_out(page: &[u8], within: usize, take: usize, target: &mut [u8]) -> usize {
    let Some(available) = page.len().checked_sub(within) else {
        return 0;
    };
    let count = take.min(available).min(target.len());
    target[..count].copy_from_slice(&page[within..within + count]);
    count
}

/// Narrow a byte count to an index, saturating rather than wrapping.
fn slot(value: u64) -> usize {
    usize::try_from(value).unwrap_or(usize::MAX)
}

impl<H: IOBase> IOBase for Buffered<H> {
    // Everything the cache does not change is the wrapped handle's answer,
    // expanded from the one delegation macro. What the list leaves out is
    // exactly what this wrapper owns: the two positional primitives, the
    // resize that invalidates, and the open/close pair that holds the cache.
    crate::delegate_iobase!(handle: size, capacity, reserve, url, media_type,
        set_media_type, flush, parent, child_by, ls, kind);

    /// Serve the range from the pages holding it, fetching what is missing.
    ///
    /// A read spanning several pages copies each of them straight into the
    /// caller's buffer, so nothing is assembled in between.
    fn pread(&self, offset: u64, buffer: &mut [u8]) -> Result<usize> {
        self.read_at(offset, buffer, Instant::now())
    }

    /// Write through to the inner handle, then fold the write into the cache.
    ///
    /// Every page the write overlapped is patched with the new bytes, or
    /// dropped when the write extends past what that page cached. Pinned pages
    /// are patched like any other: a pin is about retention, never about
    /// staleness.
    fn pwrite(&mut self, offset: u64, bytes: &[u8]) -> Result<usize> {
        let previous = self.handle.size();
        let written = self.handle.pwrite(offset, bytes)?;
        let landed = &bytes[..written.min(bytes.len())];
        let options = self.options;
        self.table().apply_write(offset, landed, previous, &options);
        Ok(written)
    }

    /// Resize the inner value and drop every page at or past the new size.
    fn truncate(&mut self, size: u64) -> Result<()> {
        let previous = self.handle.size();
        self.handle.truncate(size)?;
        let options = self.options;
        self.table().retain_below(previous.min(size), &options);
        Ok(())
    }

    fn open(&mut self) -> Result<()> {
        // Opening caches what repeated calls would re-derive; it never fills
        // pages, because a page nobody asked for is a read nobody wanted.
        self.handle.open()
    }

    fn is_open(&self) -> bool {
        self.handle.is_open()
    }

    /// Flush the inner handle and release every cached page.
    ///
    /// Pinned pages go too: `close` releases cached state, and the handle
    /// stays usable - the next read simply fetches again.
    fn close(&mut self) -> Result<()> {
        let closed = self.handle.close();
        self.table().clear();
        closed
    }
}

#[cfg(test)]
pub(crate) mod tests;
