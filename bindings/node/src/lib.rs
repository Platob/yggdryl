//! Node extension for yggdryl — a thin napi wrapper that delegates to `yggdryl-core`.
//!
//! The core is the source of truth; each item here is one or two lines over `yggdryl_core`.
//! The top-level `yggdryl.version()` is the minimal example; richer surfaces live in JS
//! namespaces that mirror the core's modules — currently `yggdryl.uri` (RFC 3986 URIs,
//! absolute URLs, and authorities, mirroring `yggdryl_core::io`).

#[macro_use]
extern crate napi_derive;

pub mod uri;

/// The library version string — delegates to [`yggdryl_core::version`].
#[napi]
pub fn version() -> String {
    yggdryl_core::version().to_string()
}
