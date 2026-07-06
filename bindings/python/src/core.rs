//! The `yggdryl.core` submodule — thin wrappers over the `yggdryl-core` crate.
//!
//! It currently exposes the hello-world entry points; more surface is added here as
//! the core crate grows.

use pyo3::prelude::*;
use pyo3::wrap_pyfunction;

/// The `yggdryl-core` version string.
#[pyfunction]
fn version() -> &'static str {
    yggdryl_core::version()
}

/// Prints `Hello, world!` to standard output — the minimal cross-language example.
#[pyfunction]
fn hello() {
    yggdryl_core::hello()
}

/// Populates the `core` submodule.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(version, module)?)?;
    module.add_function(wrap_pyfunction!(hello, module)?)?;
    Ok(())
}
