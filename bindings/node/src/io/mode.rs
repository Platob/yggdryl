//! The `yggdryl.io` namespace's [`IOMode`] — how a source may be accessed.
//!
//! Mirrors `yggdryl_core::io::IOMode`, an int enum with wire-stable numeric values
//! (`Read = 1` … `Overwrite = 5`). A napi enum cannot carry methods, so the core's surface is
//! exposed as namespace-level functions: `parseIoMode` (the generic, type-inferring entry — a
//! name dispatches to the core's `parse_str`, a number to `from_u8`), `ioModeName`,
//! `ioModeIsReadable`, and `ioModeIsWritable`. Every parse failure surfaces as a thrown
//! `Error` carrying the core's guided text unchanged.

use napi::bindgen_prelude::Either;
use napi_derive::napi;

use yggdryl_core::io as core;

/// Maps any core error to a thrown JS `Error` (its guided text).
fn to_error(error: impl std::fmt::Display) -> napi::Error {
    napi::Error::from_reason(error.to_string())
}

/// The access mode of an I/O source — how the source may be used. The numeric values are
/// wire-stable: `Read = 1`, `Write = 2`, `ReadWrite = 3` (= `Read | Write`), `Append = 4`,
/// `Overwrite = 5`. Every `memory` source reports one (`Heap.mode`).
#[napi(namespace = "io")]
pub enum IOMode {
    /// Read-only access — `"read"` / `"r"`. Value `1`.
    Read = 1,
    /// Write-only access — `"write"` / `"w"`. Value `2`.
    Write = 2,
    /// Read and write access — `"read_write"` / `"rw"` / `"+"`. Value `3` (`Read | Write`).
    ReadWrite = 3,
    /// Write-only, every write lands at the end — `"append"` / `"a"`. Value `4`.
    Append = 4,
    /// Write that truncates existing content first — `"overwrite"` / `"o"` / `"truncate"`.
    /// Value `5`.
    Overwrite = 5,
}

impl From<IOMode> for core::IOMode {
    fn from(value: IOMode) -> Self {
        match value {
            IOMode::Read => core::IOMode::Read,
            IOMode::Write => core::IOMode::Write,
            IOMode::ReadWrite => core::IOMode::ReadWrite,
            IOMode::Append => core::IOMode::Append,
            IOMode::Overwrite => core::IOMode::Overwrite,
        }
    }
}

impl From<core::IOMode> for IOMode {
    fn from(value: core::IOMode) -> Self {
        match value {
            core::IOMode::Read => IOMode::Read,
            core::IOMode::Write => IOMode::Write,
            core::IOMode::ReadWrite => IOMode::ReadWrite,
            core::IOMode::Append => IOMode::Append,
            core::IOMode::Overwrite => IOMode::Overwrite,
        }
    }
}

/// Parses an [`IOMode`] — the generic, type-inferring entry. A **string** dispatches to the
/// core name parser (ASCII case-insensitive: the canonical snake_case name or its short
/// POSIX-style alias — `io.parseIoMode('rw')`), a **number** to the stable numeric values
/// (`io.parseIoMode(4)`). The number crosses as an `i64` (never ECMAScript `ToUint32`), so a
/// negative or huge input is rejected as itself rather than silently wrapped modulo 2^32.
/// Throws a guided `Error` naming the offending input and every accepted token.
#[napi(namespace = "io")]
pub fn parse_io_mode(value: Either<String, i64>) -> napi::Result<IOMode> {
    match value {
        Either::A(name) => core::IOMode::parse_str(&name),
        Either::B(number) => match u8::try_from(number) {
            Ok(byte) => core::IOMode::from_u8(byte),
            // A value outside u8 can never be a mode; raise the core's own guided error so
            // the offending number is named exactly (text identical to Rust/Python).
            Err(_) => Err(core::IoError::UnknownName {
                kind: "IOMode",
                input: number.to_string(),
                expected: "1 (read), 2 (write), 3 (read_write), 4 (append), 5 (overwrite)",
            }),
        },
    }
    .map(IOMode::from)
    .map_err(to_error)
}

/// The canonical snake_case name of `mode` (`"read_write"`) — the exact inverse of
/// `parseIoMode`.
#[napi(namespace = "io")]
pub fn io_mode_name(mode: IOMode) -> String {
    core::IOMode::from(mode).name().to_string()
}

/// Whether `mode` allows reading (`Read` / `ReadWrite`).
#[napi(namespace = "io")]
pub fn io_mode_is_readable(mode: IOMode) -> bool {
    core::IOMode::from(mode).is_readable()
}

/// Whether `mode` allows writing (everything except `Read`).
#[napi(namespace = "io")]
pub fn io_mode_is_writable(mode: IOMode) -> bool {
    core::IOMode::from(mode).is_writable()
}
