//! The `yggdryl.io` [`IOMode`] enum — how a source may be accessed.
//!
//! Mirrors [`yggdryl_core::io::IOMode`]: an int enum with wire-stable values (`Read = 1`,
//! `Write = 2`, `ReadWrite = 3`, `Append = 4`, `Overwrite = 5`), the capability predicates
//! (`is_readable` / `is_writable`), and one generic, type-inferring [`parse`](IOMode::parse)
//! that dispatches a `str` name to the core `parse_str` and an `int` value to the core
//! `from_u8`. A failed parse raises a guided `ValueError` carrying the core error text
//! unchanged.

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

/// The access mode of an I/O source — read / write / read-write / append / overwrite, with
/// the same wire-stable numeric values as the core (`Read = 1`, … `Overwrite = 5`), so
/// `IOMode.Read == 1` and `int(IOMode.Read) == 1`. Hashable and frozen like an int enum.
#[pyclass(module = "yggdryl.io", eq, eq_int, hash, frozen)]
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum IOMode {
    /// Read-only access — `"read"` / `"r"`. Value `1`.
    Read = 1,
    /// Write-only access — `"write"` / `"w"`. Value `2`.
    Write = 2,
    /// Read and write access — `"read_write"` / `"rw"` / `"+"`. Value `3`.
    ReadWrite = 3,
    /// Write-only, every write lands at the end — `"append"` / `"a"`. Value `4`.
    Append = 4,
    /// Write that truncates existing content first — `"overwrite"` / `"o"` / `"truncate"`.
    /// Value `5`.
    Overwrite = 5,
}

impl From<IOMode> for io::IOMode {
    fn from(mode: IOMode) -> Self {
        match mode {
            IOMode::Read => io::IOMode::Read,
            IOMode::Write => io::IOMode::Write,
            IOMode::ReadWrite => io::IOMode::ReadWrite,
            IOMode::Append => io::IOMode::Append,
            IOMode::Overwrite => io::IOMode::Overwrite,
        }
    }
}

impl From<io::IOMode> for IOMode {
    fn from(mode: io::IOMode) -> Self {
        match mode {
            io::IOMode::Read => IOMode::Read,
            io::IOMode::Write => IOMode::Write,
            io::IOMode::ReadWrite => IOMode::ReadWrite,
            io::IOMode::Append => IOMode::Append,
            io::IOMode::Overwrite => IOMode::Overwrite,
        }
    }
}

#[pymethods]
impl IOMode {
    /// The generic, type-inferring parse: a `str` name (`"read"`, `"rw"`, …, ASCII
    /// case-insensitive) dispatches to the core `parse_str`; an `int` value (`1..=5`) to the
    /// core `from_u8`. Anything else raises a guided `ValueError`.
    #[staticmethod]
    fn parse(value: &Bound<'_, PyAny>) -> PyResult<IOMode> {
        if let Ok(name) = value.extract::<String>() {
            io::IOMode::parse_str(&name)
                .map(IOMode::from)
                .map_err(ioerr)
        } else if let Ok(number) = value.extract::<u8>() {
            io::IOMode::from_u8(number).map(IOMode::from).map_err(ioerr)
        } else if let Ok(number) = value.extract::<i128>() {
            // An int outside u8 range still gets the exact core `from_u8` error text.
            Err(ioerr(IoError::UnknownName {
                kind: "IOMode",
                input: number.to_string(),
                expected: "1 (read), 2 (write), 3 (read_write), 4 (append), 5 (overwrite)",
            }))
        } else {
            Err(PyValueError::new_err(format!(
                "unknown IOMode {}: expected a str name (read/r, write/w, read_write/rw/+, \
                 append/a, overwrite/o/truncate) or an int value 1..=5",
                value.repr()?
            )))
        }
    }

    /// The canonical snake_case name (`"read_write"`) — the exact inverse of
    /// [`parse`](IOMode::parse).
    fn name(&self) -> &'static str {
        io::IOMode::from(*self).name()
    }

    /// The stable numeric value (`Read = 1`, … `Overwrite = 5`).
    fn to_u8(&self) -> u8 {
        io::IOMode::from(*self).to_u8()
    }

    /// Whether this mode allows reading (`Read` / `ReadWrite`).
    fn is_readable(&self) -> bool {
        io::IOMode::from(*self).is_readable()
    }

    /// Whether this mode allows writing (everything except `Read`).
    fn is_writable(&self) -> bool {
        io::IOMode::from(*self).is_writable()
    }

    /// The canonical name (so `str(mode)` reads like the core `Display`).
    fn __str__(&self) -> &'static str {
        io::IOMode::from(*self).name()
    }
}
