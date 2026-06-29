//! Python extension for **yggdryl**.
//!
//! Thin PyO3 wrappers over the `yggdryl_core` types; each type gets its own module
//! mirroring the Rust crate, with all logic living in the shared core so the
//! Python and Node bindings behave identically.
//!
//! The implementation was removed in a project reset; this scaffold surfaces only
//! `version()` so the extension builds and the cross-language pattern is in place.

use pyo3::prelude::*;
use pyo3::wrap_pyfunction;

/// The `yggdryl-core` version string.
#[pyfunction]
fn version() -> &'static str {
    yggdryl_core::version()
}

/// The compiled `yggdryl` extension module.
#[pymodule]
fn yggdryl(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(version, module)?)?;
    Ok(())
}
