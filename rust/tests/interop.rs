//! External-format interoperability tests.

#[path = "interop/avro.rs"]
mod avro;
#[cfg(feature = "iceberg")]
#[path = "interop/iceberg.rs"]
mod iceberg;
#[cfg(feature = "object")]
#[path = "interop/object.rs"]
mod object;
#[path = "interop/zip.rs"]
mod zip;
