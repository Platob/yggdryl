//! One test file per file under `rust/src/warehouse/`, under `tests/warehouse/`.
//!
//! `rust/tests/` mirrors `rust/src/`: a source file has exactly one test file
//! at the matching path, and this target is the harness for the warehouse -
//! the catalog, namespace and table abstraction, its generic implementations
//! and the process's registry. A test reaches the crate through `yggdryl::`
//! and nothing else.

#[path = "support/counting_filesystem.rs"]
mod counting_filesystem;
#[cfg(feature = "s3")]
#[path = "support/server.rs"]
mod server;

#[path = "warehouse/catalog.rs"]
mod catalog;
#[path = "warehouse/folder.rs"]
mod folder;
#[path = "warehouse/handle.rs"]
mod handle;
#[path = "warehouse/media.rs"]
mod media;
#[path = "warehouse/memory.rs"]
mod memory;
#[path = "warehouse/mod_.rs"]
mod mod_;
#[path = "warehouse/namespace.rs"]
mod namespace;
#[path = "warehouse/object.rs"]
mod object;
#[path = "warehouse/properties.rs"]
mod properties;
#[path = "warehouse/system.rs"]
mod system;
#[path = "warehouse/table.rs"]
mod table;
