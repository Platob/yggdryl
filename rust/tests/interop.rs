//! External-format interoperability tests.

#[path = "interop/avro.rs"]
mod avro;
#[cfg(feature = "iceberg")]
#[path = "interop/iceberg.rs"]
mod iceberg;
#[cfg(feature = "object")]
#[path = "interop/object/mod.rs"]
mod object;
#[path = "interop/xml.rs"]
mod xml;
#[path = "interop/zip.rs"]
mod zip;
