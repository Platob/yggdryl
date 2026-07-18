//! [`MemoryInfo`] — a **capacity snapshot** (total / available bytes) of a memory or storage
//! backend, and the platform-native accessors that fill it.
//!
//! One value type answers "how much room is there?" uniformly across backends: host RAM behind a
//! device (`MemoryInfo::system`), the disk under a [`LocalIO`](crate::io::local::LocalIO)
//! (`LocalIO::memory_info`), a GPU device's VRAM, and — later — an object store's quota. Each
//! source resolves it through the **fastest platform route** behind one cross-platform surface
//! (Windows `GlobalMemoryStatusEx` / `GetDiskFreeSpaceExW`, Linux `/proc/meminfo`), with a portable
//! `unknown` fallback so the accessor is total on every OS.

/// A **capacity snapshot** of a backend: its `total` size and currently `available` (free) bytes,
/// both in bytes. A plain value (`Clone`/`Eq`/`Hash`), so it keys a map, sits in a set, and
/// travels over a wire. `available == 0 && total == 0` is the portable **unknown** sentinel a
/// backend reports when the platform cannot answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct MemoryInfo {
    total: u64,
    available: u64,
}

impl MemoryInfo {
    /// A snapshot from its `total` and `available` byte counts (`available` is clamped to `total`).
    pub fn new(total: u64, available: u64) -> MemoryInfo {
        MemoryInfo {
            total,
            available: available.min(total),
        }
    }

    /// The portable **unknown** snapshot (`0` / `0`) — what a backend reports when the platform
    /// cannot answer.
    pub fn unknown() -> MemoryInfo {
        MemoryInfo::default()
    }

    /// The total capacity in bytes.
    pub fn total(&self) -> u64 {
        self.total
    }

    /// The currently available (free) bytes.
    pub fn available(&self) -> u64 {
        self.available
    }

    /// The bytes in use — `total - available`.
    pub fn used(&self) -> u64 {
        self.total.saturating_sub(self.available)
    }

    /// The fraction of capacity in use, `0.0..=1.0` (`0.0` when the total is unknown/zero).
    pub fn usage_ratio(&self) -> f64 {
        if self.total == 0 {
            0.0
        } else {
            self.used() as f64 / self.total as f64
        }
    }

    /// Whether this is the **unknown** snapshot (the platform could not report capacity).
    pub fn is_unknown(&self) -> bool {
        self.total == 0 && self.available == 0
    }

    /// The **host system memory** (physical RAM) snapshot — total and currently available — from
    /// the fastest platform route (Windows `GlobalMemoryStatusEx`, Linux `/proc/meminfo`), else
    /// [`unknown`](MemoryInfo::unknown). This is the CPU device's memory.
    ///
    /// ```
    /// use yggdryl_core::io::MemoryInfo;
    ///
    /// let sys = MemoryInfo::system();
    /// // On a real host RAM is reported; the API is total on every platform regardless.
    /// assert!(sys.total() >= sys.available());
    /// ```
    pub fn system() -> MemoryInfo {
        system_memory()
    }
}

// -------------------------------------------------------------------------------------
// Platform routes — each #[cfg] arm covers a target; the last is the portable fallback.
// -------------------------------------------------------------------------------------

#[cfg(windows)]
fn system_memory() -> MemoryInfo {
    #[repr(C)]
    struct MemoryStatusEx {
        length: u32,
        memory_load: u32,
        total_phys: u64,
        avail_phys: u64,
        total_page_file: u64,
        avail_page_file: u64,
        total_virtual: u64,
        avail_virtual: u64,
        avail_extended_virtual: u64,
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GlobalMemoryStatusEx(buffer: *mut MemoryStatusEx) -> i32;
    }
    // SAFETY: a zeroed MemoryStatusEx with its `length` set is the documented input; the call only
    // writes the struct.
    let mut status: MemoryStatusEx = unsafe { core::mem::zeroed() };
    status.length = core::mem::size_of::<MemoryStatusEx>() as u32;
    if unsafe { GlobalMemoryStatusEx(&mut status) } != 0 {
        MemoryInfo::new(status.total_phys, status.avail_phys)
    } else {
        MemoryInfo::unknown()
    }
}

#[cfg(target_os = "linux")]
fn system_memory() -> MemoryInfo {
    // Parse /proc/meminfo — no FFI: MemTotal + MemAvailable, reported in kibibytes.
    let Ok(text) = std::fs::read_to_string("/proc/meminfo") else {
        return MemoryInfo::unknown();
    };
    let kib = |key: &str| -> Option<u64> {
        text.lines()
            .find(|line| line.starts_with(key))
            .and_then(|line| line.split_whitespace().nth(1))
            .and_then(|value| value.parse::<u64>().ok())
            .map(|kib| kib * 1024)
    };
    match (kib("MemTotal:"), kib("MemAvailable:")) {
        (Some(total), Some(available)) => MemoryInfo::new(total, available),
        _ => MemoryInfo::unknown(),
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
fn system_memory() -> MemoryInfo {
    // Portable fallback (macOS/BSD/wasm): capacity is not queried without a platform route yet.
    MemoryInfo::unknown()
}

/// The **disk** capacity of the volume backing `path` (total + free), from the platform route —
/// the value [`LocalIO::memory_info`](crate::io::local::LocalIO::memory_info) reports. Windows
/// uses `GetDiskFreeSpaceExW`; other platforms report [`unknown`](MemoryInfo::unknown) until a
/// native route lands.
pub(crate) fn disk_memory(path: &std::path::Path) -> MemoryInfo {
    disk_memory_impl(path)
}

#[cfg(windows)]
fn disk_memory_impl(path: &std::path::Path) -> MemoryInfo {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    extern "system" {
        fn GetDiskFreeSpaceExW(
            directory: *const u16,
            free_bytes_available: *mut u64,
            total_bytes: *mut u64,
            total_free_bytes: *mut u64,
        ) -> i32;
    }
    // GetDiskFreeSpaceExW accepts any existing directory on the volume; walk up to the nearest
    // existing ancestor so a not-yet-created LocalIO path still resolves its volume.
    let mut probe = path;
    while !probe.exists() {
        match probe.parent() {
            Some(parent) => probe = parent,
            None => return MemoryInfo::unknown(),
        }
    }
    let wide: Vec<u16> = probe
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let (mut free_avail, mut total, mut total_free) = (0u64, 0u64, 0u64);
    // SAFETY: `wide` is a valid null-terminated path; the three out-pointers are live locals.
    let ok =
        unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &mut free_avail, &mut total, &mut total_free) };
    if ok != 0 {
        MemoryInfo::new(total, free_avail)
    } else {
        MemoryInfo::unknown()
    }
}

#[cfg(not(windows))]
fn disk_memory_impl(_path: &std::path::Path) -> MemoryInfo {
    // Portable fallback: free-space needs a platform route (statvfs) not yet wired without libc.
    MemoryInfo::unknown()
}
