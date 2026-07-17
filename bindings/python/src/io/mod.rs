//! The `io` layer of the Python binding — mirrors `yggdryl_core::io`'s folder tree: one
//! file per core module (`kind`, `mode`, `memory`, `local`). The io-root value types
//! (`Headers`, `IOMode`, `IOKind`) register on the `yggdryl.io` submodule; `memory` (the
//! in-heap sources), `local` (the local-filesystem `LocalIO` access point and the raw
//! `Mmap`, moved here from `memory` with the core's `io::local` family), and `uri` register
//! their own Python submodules.

use pyo3::prelude::*;

pub mod kind;
pub mod local;
pub mod memory;
pub mod mode;

/// Populates the `io` submodule with the root value types shared by every source.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<mode::IOMode>()?;
    module.add_class::<kind::IOKind>()?;
    Ok(())
}
