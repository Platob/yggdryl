//! External-format interoperability tests.

#[path = "interop/avro.rs"]
mod avro;
#[cfg(feature = "iceberg")]
#[path = "interop/iceberg.rs"]
mod iceberg;
#[path = "interop/zip.rs"]
mod zip;
