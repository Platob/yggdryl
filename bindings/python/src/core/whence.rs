//! `Whence` — the positional-IO seek origin exposed to Python.

use pyo3::prelude::*;

/// The origin a positional offset is measured from — the start, the current cursor,
/// or the end of an IO source (POSIX `SEEK_SET` / `SEEK_CUR` / `SEEK_END`).
///
/// Mirrors `yggdryl_core::Whence`; keep the variants and their values in sync with
/// the core enum.
#[pyclass(name = "Whence", eq, eq_int, frozen, hash)]
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum Whence {
    Start = 0,
    Current = 1,
    End = 2,
}
