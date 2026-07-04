//! Python extension for **yggdryl**.
//!
//! Each Rust crate is exposed as a submodule of the top-level `yggdryl` package —
//! `yggdryl.core` (the foundations), `yggdryl.dtype` (the data types),
//! `yggdryl.field` (the fields) and `yggdryl.scalar` (the scalars) — mirroring the
//! crate tree. A convenience `yggdryl.factory` submodule adds the type-inference
//! factory (`scalar` / `dtype` / `field`), building the matching object from a
//! native value without naming its type. The wrappers are thin: all logic lives in
//! the Rust crates, so the Python and Node bindings behave identically.

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

mod core;
mod dtype;
mod factory;
mod field;
mod scalar;

/// Wraps a data-layer error (or a binding-level message, such as an unknown
/// charset name) so pyo3 raises it as a Python `ValueError` — the shared error
/// conversion for the `dtype`, `field` and `scalar` submodules.
pub(crate) enum DataErr {
    Data(yggdryl_dtype::DataError),
    Message(String),
}

impl From<yggdryl_dtype::DataError> for DataErr {
    fn from(error: yggdryl_dtype::DataError) -> Self {
        DataErr::Data(error)
    }
}

impl From<DataErr> for PyErr {
    fn from(error: DataErr) -> Self {
        PyValueError::new_err(match error {
            DataErr::Data(error) => error.to_string(),
            DataErr::Message(message) => message,
        })
    }
}

/// Builds a child module, runs `populate`, attaches it to `parent`, and registers
/// it in `sys.modules` so `import yggdryl.<name>` works as well as attribute access.
fn add_submodule(
    py: Python<'_>,
    parent: &Bound<'_, PyModule>,
    name: &str,
    populate: impl FnOnce(&Bound<'_, PyModule>) -> PyResult<()>,
) -> PyResult<()> {
    let child = PyModule::new_bound(py, name)?;
    populate(&child)?;
    parent.add_submodule(&child)?;
    py.import_bound("sys")?
        .getattr("modules")?
        .set_item(format!("yggdryl.{name}"), &child)?;
    Ok(())
}

/// The compiled `yggdryl` extension module.
#[pymodule]
fn yggdryl(py: Python<'_>, module: &Bound<'_, PyModule>) -> PyResult<()> {
    add_submodule(py, module, "core", core::register)?;
    add_submodule(py, module, "dtype", dtype::register)?;
    add_submodule(py, module, "field", field::register)?;
    add_submodule(py, module, "scalar", scalar::register)?;
    add_submodule(py, module, "factory", factory::register)?;
    Ok(())
}
