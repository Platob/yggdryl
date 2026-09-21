//! Record-format integration tests, one module per media family.

#[path = "media/avro.rs"]
mod avro;
#[path = "media/avro_variant.rs"]
mod avro_variant;
#[cfg(feature = "iceberg")]
#[path = "media/iceberg.rs"]
mod iceberg;
#[path = "media/inference.rs"]
mod inference;
#[path = "media/ipc.rs"]
mod ipc;
#[path = "media/magic.rs"]
mod magic;
#[path = "media/merge.rs"]
mod merge;
#[path = "media/options.rs"]
mod options;
#[cfg(feature = "parquet")]
#[path = "media/parquet.rs"]
mod parquet;
#[path = "media/partition/mod.rs"]
mod partition;
#[path = "media/routing.rs"]
mod routing;
#[path = "media/structured.rs"]
mod structured;
#[path = "media/text.rs"]
mod text;
