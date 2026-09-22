//! Exchange objects with a real implementation of each store, both directions.
//!
//! Self-consistency proves nothing about a wire protocol: the fake store the
//! unit tests run against answers all three dialects, but it was written from
//! the same reading of each API as the client, so the two can agree and both be
//! wrong. These suites run against a real server per store - MinIO, Azurite,
//! `fake-gcs-server` - and cross-check with that store's own reference client.
//!
//! Nothing runs without an endpoint. One environment variable per store is what
//! turns each suite on, and the reading half prints `SKIPPED` when what the
//! reference client was meant to write is not there, so a skipped half can
//! never read as a pass.

#[path = "aws.rs"]
mod aws;
#[path = "azure.rs"]
mod azure;
#[path = "gcs.rs"]
mod gcs;
