//! One test file per file under `rust/src/s3/` that pins something of its
//! own, mirrored file for file: [`client`], [`encryption`], [`file`],
//! [`folder`], [`options`], [`path`], [`properties`], [`provider`] and
//! [`xml`] here, and the files each dialect owns under [`aws`] and
//! [`azure`]. Who a request signs as - the credential chain, the profile, a
//! role, a sign-in - and the Signature Version 4 signing itself are the root
//! `aws` module's, so they are pinned under `rust/tests/aws/` (the signing in
//! `rust/tests/aws/sigv4.rs`); what stays here is how the S3 backend wires a
//! session in, which the suites above drive over a socket.
//!
//! The whole backend is behind the `s3` feature, so every module here
//! carries that cfg; the ones that pin something a caller cannot reach - the
//! two XML vocabularies, the addressing, the payload-signing policy - carry
//! the `internals` cfg beside it, on the module or on a module inside the
//! file, and reach the crate through `yggdryl::internals`. Everything else
//! reaches it through `yggdryl::` like any other caller.
//!
//! [`server`] is not a suite either: it is the in-process store that speaks
//! all three dialects, and answers STS's `AssumeRole` on the same endpoint,
//! declared here once so every suite over a socket shares one fixture, and
//! [`mod_`] is the handle-building that goes with it.

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
#[path = "s3/xml.rs"]
mod xml;
