//! The `yggdryl.core` submodule — thin wrappers over the `yggdryl-core` crate.

use pyo3::prelude::*;
use pyo3::wrap_pyfunction;

/// The `yggdryl-core` version string.
#[pyfunction]
fn version() -> &'static str {
    yggdryl_core::version()
}

/// Populates the `core` submodule.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(version, module)?)?;
    Ok(())
}
