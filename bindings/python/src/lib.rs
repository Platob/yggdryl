//! Python extension for yggdryl — a thin PyO3 wrapper that delegates to `yggdryl-core`.
//!
//! The core is the source of truth; each item here is one or two lines over `yggdryl_core`.
//! The top-level `yggdryl.version()` is the minimal example; richer surfaces live in
//! submodules that mirror the core's modules — currently `yggdryl.uri` (RFC 3986 URIs,
//! absolute URLs, and authorities, mirroring `yggdryl_core::io`).

use pyo3::prelude::*;

mod uri;

/// The library version string — delegates to [`yggdryl_core::version`].
#[pyfunction]
fn version() -> &'static str {
    yggdryl_core::version()
}

/// Builds a child module, runs `populate`, attaches it to `parent`, and registers it in
/// `sys.modules` so `import yggdryl.<name>` works as well as attribute access.
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

/// The `yggdryl` Python module.
#[pymodule]
fn yggdryl(py: Python<'_>, module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(version, module)?)?;
    add_submodule(py, module, "uri", uri::register)?;
    Ok(())
}
