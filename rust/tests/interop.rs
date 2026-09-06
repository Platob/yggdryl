//! External-format interoperability tests.

#[path = "interop/avro.rs"]
mod avro;
#[cfg(feature = "iceberg")]
#[path = "interop/iceberg.rs"]
mod iceberg;
#[cfg(feature = "s3")]
#[path = "interop/s3.rs"]
mod s3;
