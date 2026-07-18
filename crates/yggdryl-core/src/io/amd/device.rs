//! [`AmdDevice`] — the **AMD Radeon** device descriptor and its hardware probe.
//!
//! [`detect`] **adapts to the hardware present** by probing the OS for an AMD Radeon adapter
//! (Windows: the display-adapter registry class), reading its name and VRAM. When no adapter is
//! found the family falls back to [`AmdDevice::host_fallback`] — a present-`false` descriptor
//! sized from host RAM — so an [`AmdHeap`](super::AmdHeap) is always usable, on every OS, whether
//! or not a Radeon card is installed.

use std::sync::LazyLock;

use crate::io::MemoryInfo;

/// A **value description of the AMD compute device** — its human name and total VRAM, plus whether
/// a real Radeon adapter backs it ([`is_present`](AmdDevice::is_present)) or it is the host-memory
/// fallback. A plain value (`Clone`/`Eq`/`Hash`) that keys a map, sits in a set, and travels over a
/// wire; live free-memory is a fresh [`memory_info`](AmdDevice::memory_info) query, not baked in.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AmdDevice {
    name: String,
    total_memory: u64,
    present: bool,
}

impl AmdDevice {
    /// A **detected** AMD device from its name and total VRAM (bytes).
    pub fn new(name: impl Into<String>, total_memory: u64) -> AmdDevice {
        AmdDevice {
            name: name.into(),
            total_memory,
            present: true,
        }
    }

    /// The **host-memory fallback** used when no AMD Radeon adapter is present — total sized from
    /// [`MemoryInfo::system`](crate::io::MemoryInfo::system), [`is_present`](AmdDevice::is_present)
    /// `false`. An [`AmdHeap`](super::AmdHeap) on this device stages entirely through host memory.
    pub fn host_fallback() -> AmdDevice {
        AmdDevice {
            name: "no AMD device (host memory)".to_string(),
            total_memory: MemoryInfo::system().total(),
            present: false,
        }
    }

    /// The human-readable device name (the driver description for a real adapter).
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The total device memory in bytes (VRAM for a real adapter, host RAM for the fallback).
    pub fn total_memory(&self) -> u64 {
        self.total_memory
    }

    /// Whether a **real AMD Radeon adapter** backs this device (vs the host-memory fallback).
    pub fn is_present(&self) -> bool {
        self.present
    }

    /// A **live capacity snapshot** — a present adapter reports its total VRAM (a live free-VRAM
    /// query lands with the hardware queue); the fallback queries host RAM fresh
    /// ([`MemoryInfo::system`](crate::io::MemoryInfo::system)).
    pub fn memory_info(&self) -> MemoryInfo {
        if self.present {
            MemoryInfo::new(self.total_memory, self.total_memory)
        } else {
            MemoryInfo::system()
        }
    }
}

/// Probes for an **AMD Radeon** device, returning its [`AmdDevice`] (name + VRAM) when found, else
/// `None`. Defensive — any platform-query failure yields `None`, never a panic.
///
/// Windows enumerates the display-adapter registry class
/// (`…\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}\000N`), matching a `DriverDesc` naming
/// AMD / Radeon and reading its `HardwareInformation.qwMemorySize` VRAM. Other platforms report
/// `None` until their native route is wired.
pub fn detect() -> Option<AmdDevice> {
    detect_impl()
}

#[cfg(windows)]
fn detect_impl() -> Option<AmdDevice> {
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
            return Some(AmdDevice::new(desc, vram));
        }
    }
    None
}

#[cfg(not(windows))]
fn detect_impl() -> Option<AmdDevice> {
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

/// The process-wide detected **AMD device** (resolved once), or the host-memory fallback when no
/// adapter is present.
pub(crate) fn amd_device() -> AmdDevice {
    static DEVICE: LazyLock<AmdDevice> =
        LazyLock::new(|| detect().unwrap_or_else(AmdDevice::host_fallback));
    DEVICE.clone()
}
