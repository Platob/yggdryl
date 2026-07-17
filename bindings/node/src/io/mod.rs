//! The `io` layer of the Node binding — mirrors `yggdryl_core::io`'s folder tree: one file
//! per core module (`memory`, `uri`), each exporting its own napi namespace.

pub mod memory;
pub mod uri;
