//! `rust/src/s3/azure/mod.rs`: the files the Azure dialect owns,
//! declared together.

#[cfg(all(feature = "s3", feature = "internals"))]
#[path = "dialect.rs"]
mod dialect;
#[cfg(all(feature = "s3", feature = "internals"))]
#[path = "sign.rs"]
mod sign;
#[cfg(all(feature = "s3", feature = "internals"))]
#[path = "xml.rs"]
mod xml;
