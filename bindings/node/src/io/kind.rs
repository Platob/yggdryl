//! The `yggdryl.io` namespace's [`IOKind`] — what kind of thing an I/O source is.
//!
//! Mirrors `yggdryl_core::io::IOKind`, an int enum with wire-stable numeric values
//! (`Unknown = 0` … `Heap = 4`). A napi enum cannot carry methods, so the core's surface is
//! exposed as namespace-level functions: `parseIoKind` (the generic, type-inferring entry — a
//! name dispatches to the core's `parse_str`, a number to `from_u8`), `ioKindName`, and
//! `ioKindExists`. Every parse failure surfaces as a thrown `Error` carrying the core's
//! guided text unchanged.

use napi::bindgen_prelude::Either;
use napi_derive::napi;

use yggdryl_core::io as core;

/// Maps any core error to a thrown JS `Error` (its guided text).
fn to_error(error: impl std::fmt::Display) -> napi::Error {
    napi::Error::from_reason(error.to_string())
}

/// The kind of an I/O source — what the source physically is. The numeric values are
/// wire-stable: `Unknown = 0` (the **default**), `Missing = 1`, `File = 2`, `Directory = 3`,
/// `Heap = 4`. `Unknown` means something exists at the address but its type is not one of the
/// others (a special file, an object-store entry of an unrecognized type). Every `memory`
/// source reports one (`Heap.kind`).
#[napi(namespace = "io")]
pub enum IOKind {
    /// Something exists at the address but its type is not `File` / `Directory` / `Heap` — a
    /// special file, a symlink left unclassified, or an object-store entry of an unrecognized
    /// type. The **default** (zero) value. Value `0`.
    Unknown = 0,
    /// Nothing exists at the source's address. Value `1`.
    Missing = 1,
    /// A regular file. Value `2`.
    File = 2,
    /// A directory. Value `3`.
    Directory = 3,
    /// An in-memory heap buffer. Value `4`.
    Heap = 4,
}

impl From<IOKind> for core::IOKind {
    fn from(value: IOKind) -> Self {
        match value {
            IOKind::Unknown => core::IOKind::Unknown,
            IOKind::Missing => core::IOKind::Missing,
            IOKind::File => core::IOKind::File,
            IOKind::Directory => core::IOKind::Directory,
            IOKind::Heap => core::IOKind::Heap,
        }
    }
}

impl From<core::IOKind> for IOKind {
    fn from(value: core::IOKind) -> Self {
        match value {
            core::IOKind::Unknown => IOKind::Unknown,
            core::IOKind::Missing => IOKind::Missing,
            core::IOKind::File => IOKind::File,
            core::IOKind::Directory => IOKind::Directory,
            core::IOKind::Heap => IOKind::Heap,
        }
    }
}

/// Parses an [`IOKind`] — the generic, type-inferring entry. A **string** dispatches to the
/// core name parser (ASCII case-insensitive: `"missing"`, `"file"`, `"directory"` / `"dir"`,
/// `"heap"`, `"unknown"` — `io.parseIoKind('dir')`), a **number** to the stable numeric values
/// (`io.parseIoKind(4)`). The number crosses as an `i64` (never ECMAScript `ToUint32`), so a
/// negative or huge input is rejected as itself rather than silently wrapped modulo 2^32.
/// Throws a guided `Error` naming the offending input and every accepted token.
#[napi(namespace = "io")]
pub fn parse_io_kind(value: Either<String, i64>) -> napi::Result<IOKind> {
    match value {
        Either::A(name) => core::IOKind::parse_str(&name),
        Either::B(number) => match u8::try_from(number) {
            Ok(byte) => core::IOKind::from_u8(byte),
            // A value outside u8 can never be a kind; raise the core's own guided error so
            // the offending number is named exactly (text identical to Rust/Python).
            Err(_) => Err(core::IoError::UnknownName {
                kind: "IOKind",
                input: number.to_string(),
                expected: "0 (unknown), 1 (missing), 2 (file), 3 (directory), 4 (heap)",
            }),
        },
    }
    .map(IOKind::from)
    .map_err(to_error)
}

/// The canonical lowercase name of `kind` (`"directory"`) — the exact inverse of
/// `parseIoKind`.
#[napi(namespace = "io")]
pub fn io_kind_name(kind: IOKind) -> String {
    core::IOKind::from(kind).name().to_string()
}

/// Whether the source exists at all (everything except `Missing`).
#[napi(namespace = "io")]
pub fn io_kind_exists(kind: IOKind) -> bool {
    core::IOKind::from(kind).exists()
}
