//! Node extension for yggdryl — a thin napi wrapper that delegates to `yggdryl-core`.
//!
//! The core is the source of truth; each item here is one or two lines over `yggdryl_core`.
//! The top-level `yggdryl.version()` is the minimal example, plus the `yggdryl.io` namespace
//! (the root value types: the `Headers` metadata map and the `IOMode` / `IOKind` int enums),
//! the `yggdryl.uri` namespace (RFC 3986 URIs, absolute URLs, and authorities), mirroring
//! `yggdryl_core::uri`, the `yggdryl.memory` namespace (the in-heap `Heap` byte source
//! and the `Whence` seek anchor), mirroring `yggdryl_core::io::memory`, and the
//! `yggdryl.local` namespace (the local-filesystem family: the lazy `LocalIO` single access
//! point and the raw memory-mapped `Mmap` it builds on), mirroring `yggdryl_core::io::local`.

#[macro_use]
extern crate napi_derive;

pub mod headers;
pub mod io;
pub mod uri;

/// The library version string — delegates to [`yggdryl_core::version`].
#[napi]
pub fn version() -> String {
    yggdryl_core::version().to_string()
}
