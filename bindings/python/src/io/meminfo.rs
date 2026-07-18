//! The `yggdryl.io` [`MemoryInfo`] capacity snapshot — total / available bytes of a memory or
//! storage backend, with the platform-native accessors that fill it.
//!
//! Mirrors [`yggdryl_core::io::MemoryInfo`]: a value type (`total()` / `available()` / `used()`
//! / `usage_ratio()` / `is_unknown()`), the portable [`unknown`](MemoryInfo::unknown) sentinel,
//! and the host-RAM [`system`](MemoryInfo::system) snapshot. It is immutable, so it is equal,
//! hashable, and picklable through its `(total, available)` pair. The same value type answers
//! "how much room is there?" for a [`LocalIO`](crate::io::local::LocalIO) disk
//! (`memory_info()`) and an [`AmdDevice`](crate::io::amd::AmdDevice) uniformly.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use pyo3::prelude::*;

use yggdryl_core::io::MemoryInfo as CoreMemoryInfo;

/// A **capacity snapshot** of a backend: its `total` size and currently `available` (free)
/// bytes. An immutable value — equal, hashable, and picklable through its `(total, available)`
/// pair. `available == 0 and total == 0` is the portable **unknown** sentinel a backend reports
/// when the platform cannot answer.
#[pyclass(module = "yggdryl.io")]
#[derive(Clone)]
pub struct MemoryInfo {
    pub(crate) inner: CoreMemoryInfo,
}

#[pymethods]
impl MemoryInfo {
    /// A snapshot from its `total` and `available` byte counts (`available` is clamped to
    /// `total`).
    #[new]
    fn new(total: u64, available: u64) -> Self {
        Self {
            inner: CoreMemoryInfo::new(total, available),
        }
    }

    /// The portable **unknown** snapshot (`0` / `0`) — what a backend reports when the platform
    /// cannot answer.
    #[staticmethod]
    fn unknown() -> Self {
        Self {
            inner: CoreMemoryInfo::unknown(),
        }
    }

    /// The **host system memory** (physical RAM) snapshot — total and currently available —
    /// from the fastest platform route (Windows `GlobalMemoryStatusEx`, Linux `/proc/meminfo`),
    /// else [`unknown`](MemoryInfo::unknown). This is the CPU device's memory.
    #[staticmethod]
    fn system() -> Self {
        Self {
            inner: CoreMemoryInfo::system(),
        }
    }

    /// The total capacity in bytes.
    fn total(&self) -> u64 {
        self.inner.total()
    }

    /// The currently available (free) bytes.
    fn available(&self) -> u64 {
        self.inner.available()
    }

    /// The bytes in use — `total - available`.
    fn used(&self) -> u64 {
        self.inner.used()
    }

    /// The fraction of capacity in use, `0.0..=1.0` (`0.0` when the total is unknown/zero).
    fn usage_ratio(&self) -> f64 {
        self.inner.usage_ratio()
    }

    /// Whether this is the **unknown** snapshot (the platform could not report capacity).
    fn is_unknown(&self) -> bool {
        self.inner.is_unknown()
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }

    fn __hash__(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.inner.hash(&mut hasher);
        hasher.finish()
    }

    /// Pickles through the `(total, available)` constructor pair — the exact value.
    fn __reduce__(&self, py: Python<'_>) -> PyResult<(Py<PyAny>, (u64, u64))> {
        let ctor = py.get_type_bound::<MemoryInfo>().into_any().unbind();
        Ok((ctor, (self.inner.total(), self.inner.available())))
    }

    fn __repr__(&self) -> String {
        format!(
            "MemoryInfo(total={}, available={})",
            self.inner.total(),
            self.inner.available()
        )
    }
}
