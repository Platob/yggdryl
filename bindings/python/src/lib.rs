//! Python extension for yggdryl — a thin PyO3 wrapper that delegates to `yggdryl-core`.
//!
//! The core is the source of truth; each item here is one or two lines over `yggdryl_core`.
//! The top-level `yggdryl.version()` is the minimal example; richer surfaces live in
//! submodules that mirror the core's modules — `yggdryl.uri` (RFC 3986 URIs, absolute URLs,
//! and authorities), `yggdryl.io` (the byte-I/O `Bytes` buffer + `Whence`, and the `Headers`
//! metadata/header map), `yggdryl.types` (the typed-data schema layer: `DataType` / `Field`, plus
//! the fixed-width value/column types `U8Scalar`/`U8Serie` … `F64Scalar`/`F64Serie`), and
//! `yggdryl.decimal` (the fixed-width scaled decimals `D32`/`D64`/`D128`/`D256`), all mirroring
//! `yggdryl_core::io`.

use pyo3::prelude::*;

mod buffers;
mod bytes;
mod deccolumn;
mod decimal;
mod headers;
mod nested;
mod nullvalues;
mod temporal;
mod types;
mod uri;
mod values;
mod varvalues;

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
    add_submodule(py, module, "io", |io| {
        bytes::register(io)?;
        headers::register(io)
    })?;
    add_submodule(py, module, "types", |types| {
        types::register(types)?;
        values::register(types)?;
        varvalues::register(types)?;
        nullvalues::register(types)?;
        nested::register(types)?;
        buffers::register(types)
    })?;
    add_submodule(py, module, "decimal", |decimal| {
        decimal::register(decimal)?;
        deccolumn::register(decimal)
    })?;
    add_submodule(py, module, "temporal", temporal::register)?;
    Ok(())
}
