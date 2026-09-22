//! One test file per file under `rust/src/s3/` that pins something of its
//! own, mirrored file for file: [`client`], [`sigv4`] and [`xml`] here, the
//! three files each dialect owns under [`aws`] and [`azure`]. Beside them are
//! the suites that drive the backend whole rather than one of its files -
//! [`accounting`], [`dialects`], [`encryption`], [`properties`], [`protocol`]
//! and [`roles`].
//!
//! The whole backend is behind the `s3` feature, so every module here
//! carries that cfg; the ones that pin something a caller cannot reach - the
//! signing, the two XML vocabularies, the credential chain's own readings, the
//! addressing - carry the `internals` cfg beside it and reach the crate through
//! `yggdryl::internals`. Everything else reaches it through `yggdryl::` like
//! any other caller.
//!
//! [`server`] is not a suite either: it is the in-process store that speaks
//! all three dialects, declared here once so every suite over a socket shares
//! one fixture, and [`mod_`] is the handle-building that goes with it.

#[cfg(feature = "s3")]
#[path = "support/server.rs"]
mod server;

#[cfg(feature = "s3")]
#[path = "s3/aws/mod_.rs"]
mod aws;
#[cfg(feature = "s3")]
#[path = "s3/azure/mod_.rs"]
mod azure;
#[cfg(all(feature = "s3", feature = "internals"))]
#[path = "s3/client.rs"]
mod client;
#[cfg(feature = "s3")]
#[path = "s3/encryption.rs"]
mod encryption;
#[cfg(all(feature = "s3", feature = "internals"))]
#[path = "s3/file.rs"]
mod file;
#[cfg(feature = "s3")]
#[path = "s3/folder.rs"]
mod folder;
#[cfg(feature = "s3")]
#[path = "s3/mod_.rs"]
mod mod_;
#[cfg(feature = "s3")]
#[path = "s3/options.rs"]
mod options;
#[cfg(feature = "s3")]
#[path = "s3/path.rs"]
mod path;
#[cfg(all(feature = "s3", feature = "internals"))]
#[path = "s3/properties.rs"]
mod properties;
#[cfg(feature = "s3")]
#[path = "s3/provider.rs"]
mod provider;
#[cfg(all(feature = "s3", feature = "internals"))]
#[path = "s3/sigv4.rs"]
mod sigv4;
#[cfg(all(feature = "s3", feature = "internals"))]
#[path = "s3/xml.rs"]
mod xml;
