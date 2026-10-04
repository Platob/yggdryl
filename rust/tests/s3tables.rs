//! `rust/src/s3tables/`, file for file: [`client`] the one door every
//! request leaves through, [`bucket`], [`namespace`] and [`table`] the verbs
//! of each level, [`listing`] the lazy pages, [`mod_`] what every verb costs
//! and a table's whole life, and [`catalog`] the table bucket as a warehouse
//! catalog. Every module carries the `s3tables` cfg; what only
//! `yggdryl::internals` reaches is pinned in `client`'s `internal` module.
//!
//! [`fake`] is the in-process control plane every suite runs on, [`server`]
//! the `s3` suites' fake object store [`catalog`] keeps its warehouses in,
//! and [`live`] the ignored run against the real service for an operator
//! who names a signed-in profile.

#[cfg(feature = "s3tables")]
#[path = "support/s3tables.rs"]
mod fake;
#[cfg(feature = "s3tables")]
#[path = "support/server.rs"]
mod server;

#[cfg(feature = "s3tables")]
#[path = "s3tables/bucket.rs"]
mod bucket;
#[cfg(feature = "s3tables")]
#[path = "s3tables/catalog.rs"]
mod catalog;
#[cfg(feature = "s3tables")]
#[path = "s3tables/client.rs"]
mod client;
#[cfg(feature = "s3tables")]
#[path = "s3tables/listing.rs"]
mod listing;
#[cfg(feature = "s3tables")]
#[path = "s3tables/live.rs"]
mod live;
#[cfg(feature = "s3tables")]
#[path = "s3tables/mod_.rs"]
mod mod_;
#[cfg(feature = "s3tables")]
#[path = "s3tables/namespace.rs"]
mod namespace;
#[cfg(feature = "s3tables")]
#[path = "s3tables/table.rs"]
mod table;
