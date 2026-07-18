//! [`AmdCursor`] — a **streamed cursor over an [`AmdHeap`]**.
//!
//! The AMD family reuses the crate's one shared cursor, [`IOCursor`](crate::io::memory::IOCursor),
//! instantiated over [`AmdHeap`]: it owns the device heap and a moving position, and — because
//! `AmdHeap` forwards [`as_bytes`](crate::io::memory::IOBase::as_bytes) to its contiguous store —
//! every read/write and the vectorized bulk kernels stay on the same **zero-copy** fast path a CPU
//! `Heap` cursor uses. One optimization, shared across memory types; no per-family reimplementation.

use crate::io::amd::AmdHeap;
use crate::io::memory::IOCursor;

/// A **cursor** (a moving position with `read` / `write` / `seek`) over an [`AmdHeap`] — the AMD
/// device-memory instantiation of the shared [`IOCursor`](crate::io::memory::IOCursor). Construct
/// it from a heap with [`IOCursor::new`](crate::io::memory::IOCursor::new).
///
/// ```
/// use yggdryl_core::io::amd::{AmdCursor, AmdHeap};
///
/// let mut cur = AmdCursor::new(AmdHeap::from_host(b"radeon"));
/// let mut head = [0u8; 3];
/// assert_eq!(cur.read(&mut head), 3);
/// assert_eq!(&head, b"rad");
/// ```
pub type AmdCursor = IOCursor<AmdHeap>;
