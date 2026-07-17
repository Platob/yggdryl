//! The `io` layer of the Node binding — mirrors `yggdryl_core::io`'s folder tree: one file
//! per core module. The root value types (`headers`, `kind`, `mode`) share the `io` napi
//! namespace; `memory` and `uri` export their own.

pub mod headers;
pub mod kind;
pub mod memory;
pub mod mode;
pub mod uri;
