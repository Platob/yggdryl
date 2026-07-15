//! Node extension for yggdryl — a thin napi wrapper that delegates to `yggdryl-core`.
//!
//! The core is the source of truth; each item here is one or two lines over `yggdryl_core`.
//! The top-level `yggdryl.version()` is the minimal example; richer surfaces live in JS
//! namespaces that mirror the core's modules — `yggdryl.uri` (RFC 3986 URIs, absolute URLs,
//! and authorities), `yggdryl.io` (the byte-I/O `Bytes` buffer + `Whence`, and the `Headers`
//! metadata/header map), `yggdryl.types` (the typed-data schema layer: `DataType` / `Field`, plus
//! the fixed-width value/column types `U8Scalar`/`U8Serie` … `F64Scalar`/`F64Serie`), and
//! `yggdryl.decimal` (the fixed-width scaled decimals `D32`/`D64`/`D128`/`D256`), all mirroring
//! `yggdryl_core::io`.

#[macro_use]
extern crate napi_derive;

pub mod bytes;
pub mod deccolumn;
pub mod decimal;
pub mod headers;
pub mod nested;
pub mod nullvalues;
pub mod temporal;
pub mod temporal_column;
pub mod types;
pub mod uri;
pub mod values;
pub mod varvalues;

/// The library version string — delegates to [`yggdryl_core::version`].
#[napi]
pub fn version() -> String {
    yggdryl_core::version().to_string()
}
