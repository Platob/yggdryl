//! The `io` layer of the Python binding — mirrors `yggdryl_core::io`'s folder tree: one
//! file per core module (`headers`, `kind`, `mode`, `memory`, `uri`). The io-root value types
//! (`Headers`, `IOMode`, `IOKind`) register on the `yggdryl.io` submodule; `memory` and `uri`
//! register their own Python submodules.

use pyo3::prelude::*;

pub mod headers;
pub mod kind;
pub mod memory;
pub mod mode;
pub mod uri;

/// Populates the `io` submodule with the root value types shared by every source.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<headers::Headers>()?;
    module.add_class::<mode::IOMode>()?;
    module.add_class::<kind::IOKind>()?;
    Ok(())
}
