//! One test file per file under `rust/src/object/` that pins something of its
//! own, mirrored file for file: [`client`], [`sigv4`] and [`xml`] here, the
//! three files each dialect owns under [`aws`] and [`azure`]. Beside them are
//! the suites that drive the backend whole rather than one of its files -
//! [`accounting`], [`dialects`], [`encryption`], [`properties`], [`protocol`]
//! and [`roles`].
//!
//! The whole backend is behind the `object` feature, so every module here
//! carries that cfg; the ones that pin something a caller cannot reach - the
//! signing, the two XML vocabularies, the credential chain's own readings, the
//! addressing - carry the `internals` cfg beside it and reach the crate through
//! `yggdryl::internals`. Everything else reaches it through `yggdryl::` like
//! any other caller.
//!
//! [`server`] is not a suite either: it is the in-process store that speaks
//! all three dialects, declared here once so every suite over a socket shares
//! one fixture, and [`mod_`] is the handle-building that goes with it.

#[cfg(feature = "object")]
#[path = "object/mod_.rs"]
mod mod_;
#[cfg(feature = "object")]
#[path = "object/server.rs"]
mod server;

#[cfg(all(feature = "object", feature = "internals"))]
#[path = "object/accounting.rs"]
mod accounting;
#[cfg(all(feature = "object", feature = "internals"))]
#[path = "object/aws.rs"]
mod aws;
#[cfg(all(feature = "object", feature = "internals"))]
#[path = "object/azure.rs"]
mod azure;
#[cfg(all(feature = "object", feature = "internals"))]
#[path = "object/client.rs"]
mod client;
#[cfg(feature = "object")]
#[path = "object/dialects.rs"]
mod dialects;
#[cfg(feature = "object")]
#[path = "object/encryption.rs"]
mod encryption;
#[cfg(all(feature = "object", feature = "internals"))]
#[path = "object/properties.rs"]
mod properties;
#[cfg(all(feature = "object", feature = "internals"))]
#[path = "object/protocol.rs"]
mod protocol;
#[cfg(feature = "object")]
#[path = "object/roles.rs"]
mod roles;
#[cfg(all(feature = "object", feature = "internals"))]
#[path = "object/sigv4.rs"]
mod sigv4;
#[cfg(all(feature = "object", feature = "internals"))]
#[path = "object/xml.rs"]
mod xml;
