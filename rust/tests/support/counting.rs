//! The measuring instrument every handle-wrapper suite asserts against.
//!
//! A cache, a codec and a transcriber are invisible except through the traffic
//! they do *not* send to the handle underneath, so the suites count inner
//! calls rather than time them. One fixture, because three themes need the
//! same one.

#![allow(dead_code)]

use std::sync::atomic::{AtomicUsize, Ordering};

use yggdryl::IOBase;
use yggdryl::holder::Buffer;

/// A handle that mirrors a [`Buffer`] and counts what reaches it.
///
/// It is the whole measuring instrument for this module: a cache is invisible
/// except through the reads it does *not* perform, so the tests assert inner
/// call counts rather than timings.
#[derive(Debug, Default)]
pub struct Counting {
    handle: Buffer,
    reads: AtomicUsize,
    writes: AtomicUsize,
    sizes: AtomicUsize,
}

impl Counting {
    /// Wrap bytes in a counting handle.
    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        Self {
            handle: Buffer::from_bytes(bytes),
            reads: AtomicUsize::new(0),
            writes: AtomicUsize::new(0),
            sizes: AtomicUsize::new(0),
        }
    }

    /// Return how many `pread` calls have reached the buffer.
    pub fn reads(&self) -> usize {
        self.reads.load(Ordering::Relaxed)
    }

    /// Return how many `pwrite` calls have reached the buffer.
    pub fn writes(&self) -> usize {
        self.writes.load(Ordering::Relaxed)
    }

    /// Return how many `size` calls have reached the buffer.
    ///
    /// A `Buffer` answers this from a field, but the backends the cache
    /// exists for do not: an `fs` handle answers it with one metadata
    /// call through the foreign filesystem's vtable, which over an object
    /// store is a round trip. Counting it is how the tests keep a cache hit
    /// from quietly costing one.
    pub fn sizes(&self) -> usize {
        self.sizes.load(Ordering::Relaxed)
    }
}

impl yggdryl::IOMedia for Counting {
    yggdryl::impl_default_iomedia!();
}

impl IOBase for Counting {
    yggdryl::delegate_iobase!(handle: capacity, reserve, truncate, url,
        media_type, set_media_type, flush, parent, child_by_path, ls, kind);

    fn size(&self) -> u64 {
        self.sizes.fetch_add(1, Ordering::Relaxed);
        self.handle.size()
    }

    fn pread(&self, offset: u64, buffer: &mut [u8]) -> yggdryl::Result<usize> {
        self.reads.fetch_add(1, Ordering::Relaxed);
        self.handle.pread(offset, buffer)
    }

    fn pwrite(&mut self, offset: u64, bytes: &[u8]) -> yggdryl::Result<usize> {
        self.writes.fetch_add(1, Ordering::Relaxed);
        self.handle.pwrite(offset, bytes)
    }
}
