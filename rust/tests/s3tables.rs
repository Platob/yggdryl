//! One test file per file under `rust/src/s3tables/`, mirrored file for
//! file: [`client`] for the one door every request leaves through - the
//! region, the endpoint, the signature, the retries, what a refusal means -
//! [`bucket`], [`namespace`] and [`table`] for the verbs over each level and
//! the values they answer, [`listing`] for the lazy pages, and [`mod_`] for
//! the module as a whole: what every verb costs in requests, and a table's
//! whole life in the catalog. [`catalog`] is the table bucket as a warehouse
//! catalog, its tables committed through the control plane.
//!
//! The whole module is behind the `s3tables` feature, so every module here
//! carries that cfg. What a caller cannot reach - the two spellings of one
//! path, the one sent and the one signed - is pinned in a module inside
//! [`client`] that carries the `internals` cfg and reaches the crate through
//! `yggdryl::internals`; everything else reaches it through `yggdryl::`
//! like any other caller.
//!
//! [`fake`] is not a suite: it is the in-process fake of the service every
//! suite over a socket shares, which checks each request it is sent on its
//! own. [`server`] is not one either: it is the fake object store the `s3`
//! suites run on, which [`catalog`] keeps each table's warehouse in;
//! [`identity`] the fake of the identity services, which [`catalog`] counts
//! the credential walks of a bucket's stores against. [`live`] is not a mirror either: it is the one ignored test that
//! runs the whole of a table's life against the real service, for an
//! operator who names a signed-in profile.

#[cfg(feature = "s3tables")]
#[path = "support/s3tables.rs"]
mod fake;
#[cfg(feature = "s3tables")]
#[path = "support/identity.rs"]
mod identity;
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
