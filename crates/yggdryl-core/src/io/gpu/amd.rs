//! The **AMD Radeon** architecture (feature `gpu-amd`).
//!
//! Two pieces: [`detect`] **adapts to the hardware present** by probing the OS for an AMD Radeon
//! adapter (Windows: the display-adapter registry class), and [`AmdBuffer`] is the device-memory
//! buffer over that adapter, a full [`GpuMemory`] implementing the whole [`IOBase`] byte + bulk
//! surface.
//!
//! **Status:** detection is live; the device-side allocation currently **stages through host
//! memory** (the buffer is correct and usable everywhere the feature builds), with the device
//! queue (upload/download → VRAM, compute kernels) as the next increment behind this feature. The
//! type, the `GpuMemory` contract, and the probe are stable now so the hardware path is a drop-in.

use super::{GpuBackend, GpuDevice, GpuMemory};
use crate::headers::Headers;
use crate::io::memory::{Heap, IOBase, IoError, NoChildren};
use crate::io::{IOKind, IOMode};
use crate::uri::Uri;

/// Probes for an **AMD Radeon** device, returning its [`GpuDevice`] (name + VRAM) when found, else
/// `None` (the caller falls back to the CPU device). Defensive — any platform-query failure yields
/// `None`, never a panic.
///
/// Windows enumerates the display-adapter registry class
/// (`…\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}\000N`), matching a `DriverDesc` naming
/// AMD / Radeon and reading its `HardwareInformation.qwMemorySize` VRAM. Other platforms report
/// `None` until their native route is wired.
pub fn detect() -> Option<GpuDevice> {
    detect_impl()
}

#[cfg(windows)]
fn detect_impl() -> Option<GpuDevice> {
    // Scan the first handful of adapter subkeys for one whose driver description names AMD/Radeon.
    for index in 0..8u32 {
        let subkey = format!(
            "SYSTEM\\CurrentControlSet\\Control\\Class\\\
             {{4d36e968-e325-11ce-bfc1-08002be10318}}\\{index:04}"
        );
        let Some(desc) = reg_read_string(&subkey, "DriverDesc") else {
            continue;
        };
        let upper = desc.to_ascii_uppercase();
        if upper.contains("AMD") || upper.contains("RADEON") {
            let vram = reg_read_u64(&subkey, "HardwareInformation.qwMemorySize").unwrap_or(0);
            return Some(GpuDevice::new(GpuBackend::Amd, desc, vram));
        }
    }
    None
}

#[cfg(not(windows))]
fn detect_impl() -> Option<GpuDevice> {
    None
}

#[cfg(windows)]
const HKEY_LOCAL_MACHINE: isize = 0x8000_0002u32 as i32 as isize;

// One declaration of `RegGetValueW` with an untyped byte sink; the readers cast their own buffer.
#[cfg(windows)]
#[link(name = "advapi32")]
extern "system" {
    fn RegGetValueW(
        hkey: isize,
        sub_key: *const u16,
        value: *const u16,
        flags: u32,
        kind: *mut u32,
        data: *mut u8,
        data_len: *mut u32,
    ) -> i32;
}

#[cfg(windows)]
fn reg_read_string(subkey: &str, value: &str) -> Option<String> {
    const RRF_RT_REG_SZ: u32 = 0x0000_0002;
    let sub = wide(subkey);
    let val = wide(value);
    let mut buf = [0u16; 256];
    let mut len = (buf.len() * 2) as u32; // capacity in bytes
                                          // SAFETY: null-terminated wide inputs; `buf`/`len` are a live sink sized in bytes as documented.
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            sub.as_ptr(),
            val.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            buf.as_mut_ptr().cast::<u8>(),
            &mut len,
        )
    };
    if status != 0 {
        return None;
    }
    let chars = (len as usize / 2).saturating_sub(1).min(buf.len()); // drop the trailing NUL
    Some(String::from_utf16_lossy(&buf[..chars]))
}

#[cfg(windows)]
fn reg_read_u64(subkey: &str, value: &str) -> Option<u64> {
    const RRF_RT_REG_QWORD: u32 = 0x0000_0040;
    let sub = wide(subkey);
    let val = wide(value);
    let mut data = 0u64;
    let mut len = 8u32;
    // SAFETY: null-terminated wide inputs; `data`/`len` describe a live 8-byte QWORD sink.
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            sub.as_ptr(),
            val.as_ptr(),
            RRF_RT_REG_QWORD,
            std::ptr::null_mut(),
            std::ptr::from_mut(&mut data).cast::<u8>(),
            &mut len,
        )
    };
    (status == 0).then_some(data)
}

#[cfg(windows)]
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// The process-wide detected **AMD device**, or the CPU fallback when none is present.
fn amd_device() -> GpuDevice {
    static DEVICE: std::sync::LazyLock<GpuDevice> =
        std::sync::LazyLock::new(|| detect().unwrap_or_else(GpuDevice::cpu));
    DEVICE.clone()
}

/// An **AMD Radeon device-memory buffer** — a full [`GpuMemory`] over the detected AMD device.
/// It implements the whole [`IOBase`] byte + vectorized-bulk surface (forwarded to its resident
/// staging store, so the bulk kernels stay on the fast contiguous path), plus `upload` / `download`.
///
/// **Status:** the resident store is host memory (a [`Heap`]) for now — correct and usable
/// everywhere — with the VRAM queue (device upload/download, compute) as the next increment. The
/// API is stable, so wiring the hardware path does not change a caller.
///
/// ```
/// use yggdryl_core::io::gpu::{AmdBuffer, GpuMemory};
/// use yggdryl_core::io::memory::IOBase;
///
/// let mut buf = AmdBuffer::new();
/// buf.upload(b"radeon payload").unwrap();
/// buf.pwrite_i32_array(16, &[1, -2, 3]).unwrap();  // vectorized bulk op on device memory
/// assert_eq!(&buf.download_vec()[..14], b"radeon payload");
/// assert_eq!(buf.device().backend().as_str(), if buf.device().is_cpu() { "cpu" } else { "amd" });
/// ```
#[derive(Clone, Debug)]
pub struct AmdBuffer {
    store: Heap,
    device: GpuDevice,
}

impl Default for AmdBuffer {
    fn default() -> Self {
        AmdBuffer {
            store: Heap::new(),
            device: amd_device(),
        }
    }
}

impl AmdBuffer {
    /// An empty AMD device buffer on the detected AMD device (or the CPU fallback when none).
    pub fn new() -> Self {
        Self::default()
    }

    /// An empty buffer with room for `capacity` bytes before reallocating.
    pub fn with_capacity(capacity: usize) -> Self {
        AmdBuffer {
            store: Heap::with_capacity(capacity),
            device: amd_device(),
        }
    }

    /// A buffer initialized by **uploading** `data` (host → device).
    pub fn from_host(data: &[u8]) -> Self {
        AmdBuffer {
            store: Heap::from_slice(data),
            device: amd_device(),
        }
    }

    /// The device bytes as a host-visible slice (zero-copy for the host-staged store).
    pub fn as_slice(&self) -> &[u8] {
        self.store.as_slice()
    }
}

impl GpuMemory for AmdBuffer {
    fn device(&self) -> &GpuDevice {
        &self.device
    }
}

impl IOBase for AmdBuffer {
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
