//! Python extension for **yggdryl**.
//!
//! Each Rust crate is exposed as a submodule of the top-level `yggdryl` package —
//! `yggdryl.core` (the foundations) and `yggdryl.data` (the Arrow data-model layer)
//! — mirroring the crate tree. The wrappers are thin: all logic lives in the Rust
//! crates, so the Python and Node bindings behave identically.

use pyo3::prelude::*;

mod core;
mod data;

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
    add_submodule(py, module, "data", data::register)?;
    Ok(())
}
