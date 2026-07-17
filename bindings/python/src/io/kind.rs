//! The `yggdryl.io` [`IOKind`] enum — what kind of thing an I/O source is.
//!
//! Mirrors [`yggdryl_core::io::IOKind`]: an int enum with wire-stable values (`Unknown = 0`,
//! `Missing = 1`, `File = 2`, `Directory = 3`, `Heap = 4`), the [`exists`](IOKind::exists)
//! predicate, and one generic, type-inferring [`parse`](IOKind::parse) that dispatches a `str`
//! name to the core `parse_str` and an `int` value to the core `from_u8`. A failed parse
//! raises a guided `ValueError` carrying the core error text unchanged.

// `useless_conversion`: pyo3's `#[pymethods]` expansion wraps fallible returns in a same-type
// `From`. `wrong_self_convention`: `to_u8` keeps the core method name, but a `#[pymethods]`
// receiver cannot take `self` by value, so it borrows.
#![allow(clippy::useless_conversion, clippy::wrong_self_convention)]

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

use yggdryl_core::io::{self, IoError};

/// Maps an [`IoError`] to a Python `ValueError` carrying its guided text.
fn ioerr(error: IoError) -> PyErr {
    PyValueError::new_err(error.to_string())
}

/// The kind of an I/O source — unknown / missing / file / directory / heap, with the same
/// wire-stable numeric values as the core (`Unknown = 0`, … `Heap = 4`), so `IOKind.Heap == 4`
/// and `int(IOKind.Heap) == 4`. Hashable and frozen like an int enum.
#[pyclass(module = "yggdryl.io", eq, eq_int, hash, frozen)]
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum IOKind {
    /// Something exists at the address but its type is not `File` / `Directory` / `Heap` — a
    /// special file, an unclassified symlink, or an object-store entry of an unrecognized
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

impl From<IOKind> for io::IOKind {
    fn from(kind: IOKind) -> Self {
        match kind {
            IOKind::Unknown => io::IOKind::Unknown,
            IOKind::Missing => io::IOKind::Missing,
            IOKind::File => io::IOKind::File,
            IOKind::Directory => io::IOKind::Directory,
            IOKind::Heap => io::IOKind::Heap,
        }
    }
}

impl From<io::IOKind> for IOKind {
    fn from(kind: io::IOKind) -> Self {
        match kind {
            io::IOKind::Unknown => IOKind::Unknown,
            io::IOKind::Missing => IOKind::Missing,
            io::IOKind::File => IOKind::File,
            io::IOKind::Directory => IOKind::Directory,
            io::IOKind::Heap => IOKind::Heap,
        }
    }
}

#[pymethods]
impl IOKind {
    /// The generic, type-inferring parse: a `str` name (`"unknown"`, `"missing"`, `"file"`,
    /// `"directory"` / `"dir"`, `"heap"`, ASCII case-insensitive) dispatches to the core
    /// `parse_str`; an `int` value (`0..=4`) to the core `from_u8`. Anything else raises a
    /// guided `ValueError`.
    #[staticmethod]
    fn parse(value: &Bound<'_, PyAny>) -> PyResult<IOKind> {
        if let Ok(name) = value.extract::<String>() {
            io::IOKind::parse_str(&name)
                .map(IOKind::from)
                .map_err(ioerr)
        } else if let Ok(number) = value.extract::<u8>() {
            io::IOKind::from_u8(number).map(IOKind::from).map_err(ioerr)
        } else if let Ok(number) = value.extract::<i128>() {
            // An int outside u8 range still gets the exact core `from_u8` error text.
            Err(ioerr(IoError::UnknownName {
                kind: "IOKind",
                input: number.to_string(),
                expected: "0 (unknown), 1 (missing), 2 (file), 3 (directory), 4 (heap)",
            }))
        } else {
            Err(PyValueError::new_err(format!(
                "unknown IOKind {}: expected a str name (unknown, missing, file, directory/dir, \
                 heap) or an int value 0..=4",
                value.repr()?
            )))
        }
    }

    /// The canonical lowercase name (`"directory"`) — the exact inverse of
    /// [`parse`](IOKind::parse).
    fn name(&self) -> &'static str {
        io::IOKind::from(*self).name()
    }

    /// The stable numeric value (`Unknown = 0`, … `Heap = 4`).
    fn to_u8(&self) -> u8 {
        io::IOKind::from(*self).to_u8()
    }

    /// Whether the source exists at all (everything except `Missing`).
    fn exists(&self) -> bool {
        io::IOKind::from(*self).exists()
    }

    /// The canonical name (so `str(kind)` reads like the core `Display`).
    fn __str__(&self) -> &'static str {
        io::IOKind::from(*self).name()
    }
}
