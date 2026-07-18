//! The `io` layer of the Node binding — mirrors `yggdryl_core::io`'s folder tree: one file
//! per core module. The root value types (`kind`, `mode`, `meminfo`) share the `io` napi
//! namespace; `gpu` (the device-memory layer), `local` (the lazy `LocalIO` access point and the
//! raw memory-mapped `Mmap`, which moved here from `memory` with the core), `memory`, and `uri`
//! export their own.

pub mod gpu;
pub mod kind;
pub mod local;
pub mod meminfo;
pub mod memory;
pub mod mode;
