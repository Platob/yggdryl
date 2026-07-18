//! The `yggdryl.io` namespace's [`MemoryInfo`] — a **capacity snapshot** of a memory or
//! storage backend.
//!
//! Mirrors `yggdryl_core::io::MemoryInfo`: a plain value carrying a backend's `total` size and
//! currently `available` (free) bytes, the one answer to "how much room is there?" across
//! backends — host RAM (`MemoryInfo.system`), the disk under a `LocalIO` (`LocalIO.memoryInfo`),
//! and a GPU device's VRAM (`gpu.GpuDevice.memoryInfo`). Every method is a thin one- or two-line
//! delegation to the core. Byte counts cross as `i64` (a JS number, exact to 2^53), matching the
//! `byteSize()` convention on the `memory` / `local` sources; the ratio crosses as `f64`.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use napi_derive::napi;

use yggdryl_core::io as core;

/// A Java-style `i32` content hash of a value, folding the 64-bit hash halves.
fn java_hash<T: Hash>(value: &T) -> i32 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    let hash = hasher.finish();
    (hash as u32 ^ (hash >> 32) as u32) as i32
}

/// A **capacity snapshot** of a backend: its `total` size and currently `available` (free)
/// bytes. A plain value (equatable), so `used` / `usageRatio` derive from the pair. An
/// `available == 0 && total == 0` value is the portable **unknown** sentinel a backend reports
/// when the platform cannot answer (`isUnknown`).
#[napi(namespace = "io")]
pub struct MemoryInfo {
    pub(crate) inner: core::MemoryInfo,
}

#[napi(namespace = "io")]
impl MemoryInfo {
    /// A snapshot from its `total` and `available` byte counts (`available` is clamped to
    /// `total`). Both cross as `i64` (a JS number, exact to 2^53); a negative input is treated
    /// as `0`.
    #[napi(constructor)]
    pub fn new(total: i64, available: i64) -> Self {
        let total = u64::try_from(total).unwrap_or(0);
        let available = u64::try_from(available).unwrap_or(0);
        Self {
            inner: core::MemoryInfo::new(total, available),
        }
    }

    /// The portable **unknown** snapshot (`0` / `0`) — what a backend reports when the platform
    /// cannot answer.
    #[napi(factory)]
    pub fn unknown() -> MemoryInfo {
        Self {
            inner: core::MemoryInfo::unknown(),
        }
    }

    /// The **host system memory** (physical RAM) snapshot — total and currently available — from
    /// the fastest platform route, else `unknown`. This is the CPU device's memory.
    #[napi(factory)]
    pub fn system() -> MemoryInfo {
        Self {
            inner: core::MemoryInfo::system(),
        }
    }

    /// The total capacity in bytes — an `i64` (a JS number, exact to 2^53).
    #[napi]
    pub fn total(&self) -> i64 {
        self.inner.total() as i64
    }

    /// The currently available (free) bytes — an `i64` (a JS number, exact to 2^53).
    #[napi]
    pub fn available(&self) -> i64 {
        self.inner.available() as i64
    }

    /// The bytes in use — `total - available` — an `i64` (a JS number, exact to 2^53).
    #[napi]
    pub fn used(&self) -> i64 {
        self.inner.used() as i64
    }

    /// The fraction of capacity in use, `0.0..=1.0` (`0.0` when the total is unknown/zero).
    #[napi]
    pub fn usage_ratio(&self) -> f64 {
        self.inner.usage_ratio()
    }

    /// Whether this is the **unknown** snapshot (the platform could not report capacity).
    #[napi]
    pub fn is_unknown(&self) -> bool {
        self.inner.is_unknown()
    }

    /// Content equality — equal iff both `total` and `available` are equal.
    #[napi]
    pub fn equals(&self, other: &MemoryInfo) -> bool {
        self.inner == other.inner
    }

    /// Java-style `i32` content hash — equal snapshots hash equal.
    #[napi]
    pub fn hash_code(&self) -> i32 {
        java_hash(&self.inner)
    }

    /// A short debug string of the form `MemoryInfo(total=<n>, available=<n>)`.
    #[napi(js_name = "toString")]
    pub fn text(&self) -> String {
        format!(
            "MemoryInfo(total={}, available={})",
            self.inner.total(),
            self.inner.available()
        )
    }
}
