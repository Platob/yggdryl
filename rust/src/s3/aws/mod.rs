//! Amazon S3, and every store that speaks its REST API.
//!
//! The dialect owns what is S3's own: the knobs an S3 request carries - the
//! payload-signing policy, a storage class, requester-pays, a checksum - and
//! the S3 XML vocabulary. Who signs, and with what, is not S3's but AWS's,
//! and lives in [`crate::aws`]: the credential chain, the shared files, the
//! STS exchange, Signature Version 4. Everything above this module - the
//! transport, the retry, the three handle roles - is written once in the
//! backend that holds it.

pub(crate) mod options;
pub(crate) mod xml;

pub use options::{AwsOptions, Checksum};
