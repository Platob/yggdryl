//! One test file per file under `rust/src/local/`, mirrored file for file.
//!
//! The three local roles - a path, a container and a leaf - are `yggdryl::local`
//! API and are pinned through it in `rust/tests/holder/local.rs`. What is left
//! here is the module's own well-known roots: the home resolution takes the two
//! environment values as arguments, which no caller does, so it carries the
//! `internals` cfg and reaches the crate through `yggdryl::internals`.

#[cfg(feature = "internals")]
#[path = "local/mod_.rs"]
mod mod_;
