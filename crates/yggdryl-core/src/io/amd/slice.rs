//! [`AmdSlice`] — a **bounded window over an [`AmdHeap`]**.
//!
//! Like [`AmdCursor`](super::AmdCursor), the AMD family reuses the crate's one shared window,
//! [`IOSlice`](crate::io::memory::IOSlice), instantiated over [`AmdHeap`]: it owns the device heap
//! and presents a sub-range addressed from its own `0`, staying on the same **zero-copy** contiguous
//! fast path (`AmdHeap` forwards [`as_bytes`](crate::io::memory::IOBase::as_bytes) to its store). One
//! shared optimization across every memory type, not a per-family reimplementation.

use crate::io::amd::AmdHeap;
use crate::io::memory::IOSlice;

/// A **bounded window** over an [`AmdHeap`] — the AMD device-memory instantiation of the shared
/// [`IOSlice`](crate::io::memory::IOSlice). Construct it over a heap and a byte range with
/// [`IOSlice::new`](crate::io::memory::IOSlice::new).
///
/// ```
/// use yggdryl_core::io::amd::{AmdHeap, AmdSlice};
/// use yggdryl_core::io::memory::IOBase;
///
/// let win = AmdSlice::new(AmdHeap::from_host(b"radeon payload"), 7, 7).unwrap();
/// assert_eq!(win.byte_size(), 7);
/// assert_eq!(win.pread_vec(0, 7), b"payload");
/// ```
pub type AmdSlice = IOSlice<AmdHeap>;
