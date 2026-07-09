//! Python extension for **yggdryl**.
//!
//! Each Rust crate — and, where a crate spans several concerns, each concern — is
//! exposed as a submodule of the top-level `yggdryl` package: `yggdryl.core` (the
//! foundations), `yggdryl.compression` (the compression codecs), `yggdryl.io`
//! (the positioned byte-IO resources), and `yggdryl.buffer` (the typed native-type
//! buffers), all mirroring `yggdryl-core`, plus `yggdryl.infer` (a binding-only
//! convenience that reads a value's runtime type and builds the matching buffer —
//! `CLAUDE.md` rule 13, so it has no core counterpart) and `yggdryl.converter` (a
//! dtype-keyed facade over the core representation converters). More
//! submodules are added here as the crate tree grows. The wrappers are thin: all
//! logic lives in the Rust crates, so the Python and Node bindings behave
//! identically.

use pyo3::prelude::*;

mod buffer;
mod compression;
mod converter;
mod core;
mod infer;
mod io;

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
    add_submodule(py, module, "compression", compression::register)?;
    add_submodule(py, module, "io", io::register)?;
    add_submodule(py, module, "buffer", buffer::register)?;
    add_submodule(py, module, "infer", infer::register)?;
    add_submodule(py, module, "converter", converter::register)?;
    Ok(())
}
