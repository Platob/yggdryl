//! One test file per file under `rust/src/object/aws/`.
//!
//! Amazon's dialect owns the credential chain, the shared configuration files
//! and the S3 XML vocabulary; none of the three is reachable from outside the
//! crate, so each is pinned through `yggdryl::internals`.

#[path = "aws/credentials.rs"]
mod credentials;
#[path = "aws/profile.rs"]
mod profile;
#[path = "aws/xml.rs"]
mod xml;
