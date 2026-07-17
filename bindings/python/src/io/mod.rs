//! The `io` layer of the Python binding — mirrors `yggdryl_core::io`'s folder tree: one
//! file per core module (`memory`, `uri`), each registering its own Python submodule.

pub mod memory;
pub mod uri;
