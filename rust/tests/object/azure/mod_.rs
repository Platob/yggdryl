//! `rust/src/object/azure/mod.rs`: the files the Azure dialect owns,
//! declared together.

#[cfg(all(feature = "object", feature = "internals"))]
#[path = "dialect.rs"]
mod dialect;
#[cfg(all(feature = "object", feature = "internals"))]
#[path = "sign.rs"]
mod sign;
#[cfg(all(feature = "object", feature = "internals"))]
#[path = "xml.rs"]
mod xml;
