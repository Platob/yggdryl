//! One test file per file under `rust/src/auth/`, mirrored file for file:
//! [`secret`], [`environment`], [`lease`] and [`report`], with [`mod_`] for
//! the module's own door.
//!
//! The module is private and rides the `aws` feature, so every file here
//! carries that cfg and the `internals` one beside it, and reaches the crate
//! through `yggdryl::internals::auth_<file>`.

#[cfg(all(feature = "aws", feature = "internals"))]
#[path = "auth/environment.rs"]
mod environment;
#[cfg(all(feature = "aws", feature = "internals"))]
#[path = "auth/lease.rs"]
mod lease;
#[cfg(all(feature = "aws", feature = "internals"))]
#[path = "auth/mod_.rs"]
mod mod_;
#[cfg(all(feature = "aws", feature = "internals"))]
#[path = "auth/report.rs"]
mod report;
#[cfg(all(feature = "aws", feature = "internals"))]
#[path = "auth/secret.rs"]
mod secret;
