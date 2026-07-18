//! Python extension for yggdryl — a thin PyO3 wrapper that delegates to `yggdryl-core`.
//!
//! The core is the source of truth; each item here is one or two lines over `yggdryl_core`.
//! The top-level `yggdryl.version()` is the minimal example, plus the `yggdryl.io` submodule
//! (the io-root value types `Headers` / `IOMode` / `IOKind` / `MemoryInfo`, mirroring
//! `yggdryl_core::io`), the `yggdryl.datatype_id` submodule (the `DataTypeId` primitive element
//! data types, mirroring `yggdryl_core::datatype_id`), the `yggdryl.amd` submodule (the AMD Radeon
//! device-memory family — `AmdDevice` / `AmdHeap` + the `detect` probe, mirroring
//! `yggdryl_core::io::amd`),
//! the `yggdryl.memory` submodule (the in-heap `Heap` byte source and the `Whence` seek anchor,
//! mirroring `yggdryl_core::io::memory`), the `yggdryl.local` submodule (the local-filesystem
//! `LocalIO` access point and the raw `Mmap` mapping — moved from `yggdryl.memory` —
//! mirroring `yggdryl_core::io::local`), the `yggdryl.uri` submodule (RFC 3986 URIs, absolute
//! URLs, and authorities, mirroring `yggdryl_core::uri`), the `yggdryl.mimetype` submodule
//! (the `MimeType` media type and the `MimeCatalog` registry, mirroring
//! `yggdryl_core::mimetype`), the `yggdryl.mediatype` submodule (the layered `MediaType`
//! list, mirroring `yggdryl_core::mediatype`), the `yggdryl.typed` submodule (the typed-column
//! surface — a `Serie` and its `Field`, mirroring `yggdryl_core::typed`) and the
//! `yggdryl.compression` submodule (the
//! `Gzip` / `Zlib` / `Zstd` / `Lzma` codecs and the `codec_for` resolver, mirroring
//! `yggdryl_core::compression`).

// `useless_conversion`: pyo3's `#[pyfunction]` expansion wraps a fallible return in a same-type
// `From` (the submodules allow the same at their module level).
#![allow(clippy::useless_conversion)]

use pyo3::prelude::*;

mod builders;
mod compression;
mod datatype_id;
mod headers;
mod io;
mod mediatype;
mod mimetype;
mod typed;
mod uri;

/// The library version string — delegates to [`yggdryl_core::version`].
#[pyfunction]
fn version() -> &'static str {
    yggdryl_core::version()
}

/// The project's `open()` — the binding analogue of Python's builtin `open`. Dispatches on the
/// runtime type of `target` and hands back the **concrete** opened source: a `bytes` /
/// `bytearray` value wraps into a `yggdryl.memory.Heap`; a `yggdryl.uri.Uri` / `Url` or a
/// `str` address routes by scheme (`mem://` → `Heap`, `file://` or a plain path →
/// `yggdryl.local.LocalIO`); an `os.PathLike` (a `pathlib.Path`) opens a `LocalIO`.
#[pyfunction]
fn open(py: Python<'_>, target: &Bound<'_, PyAny>) -> PyResult<PyObject> {
    io::open_target(py, target)
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
    module.add_function(wrap_pyfunction!(open, module)?)?;
    builders::register(module)?;
    add_submodule(py, module, "amd", io::amd::register)?;
    add_submodule(py, module, "compression", compression::register)?;
    add_submodule(py, module, "datatype_id", datatype_id::register)?;
    add_submodule(py, module, "headers", |m| m.add_class::<headers::Headers>())?;
    add_submodule(py, module, "io", io::register)?;
    add_submodule(py, module, "local", io::local::register)?;
    add_submodule(py, module, "mediatype", mediatype::register)?;
    add_submodule(py, module, "memory", io::memory::register)?;
    add_submodule(py, module, "mimetype", mimetype::register)?;
    add_submodule(py, module, "typed", typed::register)?;
    add_submodule(py, module, "uri", uri::register)?;
    Ok(())
}
