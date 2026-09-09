//! Amazon S3, and every store that speaks its REST API.
//!
//! The dialect owns what is AWS's own: Signature Version 4 over the shared
//! [`sigv4`](super::sigv4) implementation, the credential chain the AWS tools
//! walk, the STS exchange that trades keys for a role's, the shared
//! configuration files, and the S3 XML vocabulary. Everything above it - the
//! transport, the retry, the three handle roles - is written once in the
//! backend that holds this module.

pub(crate) mod credentials;
pub(crate) mod profile;
pub(crate) mod sts;
pub(crate) mod xml;

pub use credentials::Credentials;
pub use sts::AssumedRole;
