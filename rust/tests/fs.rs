//! One test file per file under `rust/src/fs/`, mirrored file for file.
//!
//! The seam itself - the filesystem trait, the binding a location is, and the
//! URI a backend resolves from - is `yggdryl::fs` API, so those suites reach
//! the crate the way a caller does. The local backend's hardening against a
//! volume root is two private helpers every destructive operation goes
//! through, so that one carries the `internals` cfg and reaches the crate
//! through `yggdryl::internals`. What the backends do over a whole tree is in
//! `rust/tests/holder/fs.rs`.

#[cfg(feature = "internals")]
#[path = "fs/local.rs"]
mod local;
#[path = "fs/location.rs"]
mod location;
#[path = "fs/uri.rs"]
mod uri;
