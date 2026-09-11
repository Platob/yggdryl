//! External-format interoperability tests.

#[path = "interop/avro.rs"]
mod avro;
#[path = "interop/charset.rs"]
mod charset;
#[cfg(feature = "iceberg")]
#[path = "interop/iceberg.rs"]
mod iceberg;
#[cfg(feature = "object")]
#[path = "interop/object/mod.rs"]
mod object;
#[path = "interop/zip.rs"]
mod zip;
