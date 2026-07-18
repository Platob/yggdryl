//! [`AmdHeap`] — the **AMD Radeon device-memory heap**, a full [`IOBase`] + [`AmdMemory`].
//!
//! It implements the whole `IOBase` byte + vectorized-bulk surface (forwarded to its resident
//! staging store, so the bulk kernels stay on the fast contiguous path), plus the `AmdMemory`
//! host↔device transfer. [`AmdCursor`](super::AmdCursor) / [`AmdSlice`](super::AmdSlice) wrap it for
//! streamed and windowed access, inheriting the same zero-copy fast path.
//!
//! **Status:** the resident store is host memory (a [`Heap`]) for now — correct and usable
//! everywhere the feature builds — with the VRAM queue (device upload/download, compute kernels) as
//! the next increment behind the `amd` feature. The type, the [`AmdMemory`] contract, and the probe
//! are stable now, so wiring the hardware path does not change a caller.

use super::device::amd_device;
use super::{AmdDevice, AmdMemory};
use crate::headers::Headers;
use crate::io::memory::{Heap, IOBase, IoError, NoChildren};
use crate::io::{IOKind, IOMode};
use crate::uri::Uri;

/// An **AMD Radeon device-memory heap** — a full [`AmdMemory`] over the detected AMD device,
/// implementing the whole [`IOBase`] byte + vectorized-bulk surface plus `upload` / `download`.
///
/// **Status:** the resident store is host memory (a [`Heap`]) for now — correct and usable
/// everywhere — with the VRAM queue as the next increment. The API is stable, so wiring the
/// hardware path does not change a caller.
///
/// ```
/// use yggdryl_core::io::amd::{AmdHeap, AmdMemory};
/// use yggdryl_core::io::memory::IOBase;
///
/// let mut buf = AmdHeap::new();
/// buf.upload(b"radeon payload").unwrap();
/// buf.pwrite_i32_array(16, &[1, -2, 3]).unwrap();  // vectorized bulk op on device memory
/// assert_eq!(&buf.download_vec()[..14], b"radeon payload");
/// // A present adapter runs on "amd"; with none installed it stages through host memory.
/// assert_eq!(buf.device().is_present(), buf.device().name() != "no AMD device (host memory)");
/// ```
#[derive(Clone, Debug)]
pub struct AmdHeap {
    store: Heap,
    device: AmdDevice,
}

impl Default for AmdHeap {
    fn default() -> Self {
        AmdHeap {
            store: Heap::new(),
            device: amd_device(),
        }
    }
}

impl AmdHeap {
    /// An empty AMD device heap on the detected AMD device (or the host fallback when none).
    pub fn new() -> Self {
        Self::default()
    }

    /// An empty heap with room for `capacity` bytes before reallocating.
    pub fn with_capacity(capacity: usize) -> Self {
        AmdHeap {
            store: Heap::with_capacity(capacity),
            device: amd_device(),
        }
    }

    /// A heap initialized by **uploading** `data` (host → device).
    pub fn from_host(data: &[u8]) -> Self {
        AmdHeap {
            store: Heap::from_slice(data),
            device: amd_device(),
        }
    }

    /// The device bytes as a host-visible slice (zero-copy for the host-staged store).
    pub fn as_slice(&self) -> &[u8] {
        self.store.as_slice()
    }
}

impl AmdMemory for AmdHeap {
    fn device(&self) -> &AmdDevice {
        &self.device
    }
}

impl IOBase for AmdHeap {
    fn byte_size(&self) -> u64 {
        self.store.byte_size()
    }

    fn capacity(&self) -> u64 {
        self.store.capacity()
    }

    fn reserve(&mut self, additional: u64) {
        self.store.reserve(additional);
    }

    fn try_reserve(&mut self, additional: u64) -> Result<(), IoError> {
        self.store.try_reserve(additional)
    }

    fn shrink_to_fit(&mut self) {
        self.store.shrink_to_fit();
    }

    fn pread_byte_array(&self, offset: u64, buf: &mut [u8]) -> usize {
        self.store.pread_byte_array(offset, buf)
    }

    fn pwrite_byte_array(&mut self, offset: u64, data: &[u8]) -> usize {
        self.store.pwrite_byte_array(offset, data)
    }

    #[inline]
    fn as_bytes(&self) -> Option<&[u8]> {
        self.store.as_bytes()
    }

    // Forward every typed bulk array + repeat to the resident store's fast contiguous kernels.
    crate::io::memory::forward_bulk_ops!(store);

    fn truncate(&mut self, len: u64) -> Result<(), IoError> {
        self.store.truncate(len)
    }

    fn uri(&self) -> Uri {
        self.store.uri()
    }

    fn headers(&self) -> &Headers {
        self.store.headers()
    }

    fn headers_mut(&mut self) -> &mut Headers {
        self.store.headers_mut()
    }

    fn mode(&self) -> IOMode {
        self.store.mode()
    }

    fn kind(&self) -> IOKind {
        self.store.kind()
    }

    fn exists(&self) -> bool {
        self.store.exists()
    }

    type Children = NoChildren<Self>;
    type Walk = NoChildren<Self>;

    fn ls(&self) -> Result<Self::Children, IoError> {
        Ok(std::iter::empty())
    }

    fn ls_recursive(&self) -> Result<Self::Walk, IoError> {
        Ok(std::iter::empty())
    }
}
