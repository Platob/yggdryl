//! One test file per file under `rust/src/object/azure/`.
//!
//! Azure's dialect owns the Shared Key signature, the block naming a staged
//! upload commits with, and its own XML vocabulary; none is reachable from
//! outside the crate, so each is pinned through `yggdryl::internals`.

#[path = "azure/dialect.rs"]
mod dialect;
#[path = "azure/sign.rs"]
mod sign;
#[path = "azure/xml.rs"]
mod xml;
