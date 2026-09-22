//! External-format interoperability tests.

#[path = "interop/avro.rs"]
mod avro;
#[path = "interop/charset.rs"]
mod charset;
#[cfg(feature = "iceberg")]
#[path = "interop/iceberg.rs"]
mod iceberg;
#[cfg(feature = "s3")]
#[path = "interop/s3/mod.rs"]
mod s3;
#[path = "interop/variant.rs"]
mod variant;
#[path = "interop/zip.rs"]
mod zip;
