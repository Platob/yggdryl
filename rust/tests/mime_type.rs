//! One test file per file under `rust/src/mime_type/`.
//!
//! The media-type registry a caller reads is pinned through `yggdryl::` like
//! any other surface; what is here is the line classifier underneath it,
//! which a caller reaches only by the answer it gives, so its file is
//! declared behind the `internals` feature.

#[cfg(feature = "internals")]
#[path = "mime_type/line.rs"]
mod line;
#[path = "mime_type/registry.rs"]
mod registry;
