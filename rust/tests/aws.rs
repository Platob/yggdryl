//! One test file per file under `rust/src/aws/`, mirrored file for file:
//! [`session`] drives the chain whole over the identity fake, and the rest
//! pin what each source reads and writes - [`credentials`], [`profile`],
//! [`sts`], [`sso`], [`process`], [`container`], [`metadata`], [`sigv4`],
//! [`environment`] - with [`mod_`] for the module's own door.
//!
//! The whole module is behind the `aws` feature, so every module here carries
//! that cfg; the ones that pin something a caller cannot reach - the signing,
//! the STS documents, the caches the CLI shares, the file reading - carry the
//! `internals` cfg beside it and reach the crate through `yggdryl::internals`.
//!
//! [`identity`] is not a suite: it is the in-process fake of every identity
//! endpoint, declared here once so every suite over a socket shares it.

#[cfg(feature = "aws")]
#[path = "support/identity.rs"]
mod identity;

#[cfg(all(feature = "aws", feature = "internals"))]
#[path = "aws/container.rs"]
mod container;
#[cfg(all(feature = "aws", feature = "internals"))]
#[path = "aws/credentials.rs"]
mod credentials;
#[cfg(feature = "aws")]
#[path = "aws/environment.rs"]
mod environment;
#[cfg(feature = "aws")]
#[path = "aws/metadata.rs"]
mod metadata;
#[cfg(feature = "aws")]
#[path = "aws/mod_.rs"]
mod mod_;
#[cfg(feature = "aws")]
#[path = "aws/process.rs"]
mod process;
#[cfg(all(feature = "aws", feature = "internals"))]
#[path = "aws/profile.rs"]
mod profile;
#[cfg(feature = "aws")]
#[path = "aws/session.rs"]
mod session;
#[cfg(all(feature = "aws", feature = "internals"))]
#[path = "aws/sigv4.rs"]
mod sigv4;
#[cfg(all(feature = "aws", feature = "internals"))]
#[path = "aws/sso.rs"]
mod sso;
#[cfg(all(feature = "aws", feature = "internals"))]
#[path = "aws/sts.rs"]
mod sts;
