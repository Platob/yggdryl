//! One test file per file under `rust/src/local/`, mirrored file for file.
//!
//! The three local roles - a path, a container and a leaf - are `yggdryl::local`
//! API and are pinned through it, one file per role. Beside them sits the
//! module's own well-known roots: the home resolution takes the two
//! environment values as arguments, which no caller does, so that part
//! carries the `internals` cfg and reaches the crate through
//! `yggdryl::internals`.

#[path = "local/file.rs"]
mod file;
#[path = "local/folder.rs"]
mod folder;
#[path = "local/mod_.rs"]
mod mod_;
#[path = "local/path.rs"]
mod path;
