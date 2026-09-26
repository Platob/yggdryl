//! What every identity provider shares.
//!
//! Amazon, Google and Azure each answer "who is asking" their own way - a
//! signature, a bearer token, a shared key - but the shape around the answer
//! is one shape: a secret that must never render, a value that lapses and is
//! obtained again before it does, an environment the process has or a caller
//! hands over, and a chain of sources walked in order with every failure
//! kept for the refusal at the end. Those four are written here once, and
//! each provider - [`crate::aws`], the Google and Azure dialects of the S3
//! backend - carries only what is its own: where its answer comes from and
//! how it is spelled on the wire.
//!
//! The module is private and rides the `http` feature, whose
//! [`Authorization`](crate::http::Authorization) is the first consumer of
//! [`Secret`] and whose client reads the process environment through
//! [`variable`]; the lease, the report and the environment a caller hands
//! over are the providers' alone and ride `aws`.

pub(crate) mod environment;
#[cfg(feature = "aws")]
pub(crate) mod lease;
#[cfg(feature = "aws")]
pub(crate) mod report;
pub(crate) mod secret;

pub(crate) use environment::variable;
#[cfg(feature = "aws")]
pub(crate) use environment::{Environment, is_true};
#[cfg(feature = "s3")]
pub(crate) use lease::Bearer;
#[cfg(feature = "aws")]
pub(crate) use lease::{Expiring, Lease, instant, instant_from_millis, iso8601};
#[cfg(feature = "aws")]
pub(crate) use report::Report;
pub(crate) use secret::Secret;
#[cfg(feature = "aws")]
pub(crate) use secret::write_private;
