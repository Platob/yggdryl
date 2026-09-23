//! One test file per file under `rust/src/iceberg/`, mirrored file for file.
//!
//! The whole table implementation is behind the `iceberg` feature, so every
//! module here carries that cfg. The ones that pin something a caller cannot
//! reach - the portable bound encoding, the writer's bound folding, the
//! metadata counters, the manifest readers, the planning internals, the commit
//! retry and staging - carry the `internals` cfg beside it and reach the crate
//! through `yggdryl::internals`. Everything else reaches it through
//! `yggdryl::` like any other caller.

#[cfg(feature = "iceberg")]
#[path = "iceberg/catalog/mod_.rs"]
mod catalog;
#[cfg(all(feature = "iceberg", feature = "internals"))]
#[path = "iceberg/evolve.rs"]
mod evolve;
#[cfg(all(feature = "iceberg", feature = "internals"))]
#[path = "iceberg/manifest.rs"]
mod manifest;
#[cfg(feature = "iceberg")]
#[path = "iceberg/metadata.rs"]
mod metadata;
#[cfg(all(feature = "iceberg", feature = "internals"))]
#[path = "iceberg/mod_.rs"]
mod mod_;
#[cfg(feature = "iceberg")]
#[path = "iceberg/official.rs"]
mod official;
#[cfg(feature = "iceberg")]
#[path = "iceberg/partition.rs"]
mod partition;
#[cfg(feature = "iceberg")]
#[path = "iceberg/scan.rs"]
mod scan;
#[cfg(feature = "iceberg")]
#[path = "iceberg/schema.rs"]
mod schema;
#[cfg(all(feature = "iceberg", feature = "internals"))]
#[path = "iceberg/snapshot.rs"]
mod snapshot;
#[cfg(feature = "iceberg")]
#[path = "iceberg/staging.rs"]
mod staging;
#[cfg(all(feature = "iceberg", feature = "internals"))]
#[path = "iceberg/statistics.rs"]
mod statistics;
#[cfg(feature = "iceberg")]
#[path = "iceberg/table.rs"]
mod table;
#[cfg(feature = "iceberg")]
#[path = "iceberg/types.rs"]
mod types;
#[cfg(all(feature = "iceberg", feature = "internals"))]
#[path = "iceberg/value.rs"]
mod value;
