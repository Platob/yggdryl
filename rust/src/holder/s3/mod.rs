//! Amazon S3 objects as [`IOBase`](crate::IOBase) handles.
//!
//! The same three roles every storage backend supplies, spoken to S3 over its
//! REST API directly - no SDK, no async runtime, no object-store abstraction
//! in between:
//!
//! - [`Path`] is the generic location, which resolves to whichever of the two
//!   it turns out to name.
//! - [`Folder`] is the container: a key prefix, or a whole bucket.
//! - [`File`] is the leaf: one object, read by range and written whole.
//!
//! Everything else follows from [`IOBase`](crate::IOBase) rather than being
//! written again here - globs, Hive partitions, page caching through
//! [`Buffered`](crate::holder::buffered::Buffered), content codings, IPC,
//! Parquet, Avro, Iceberg tables. A record reader over an object is the same
//! reader that runs over a mapped file.
//!
//! # The design goal is the request count
//!
//! Every operation states how many round trips it costs, on the method that
//! performs it, and the counts are asserted by tests rather than intended:
//!
//! | operation | requests |
//! | --- | --- |
//! | building any handle | none |
//! | resolving a `lake/` location | none |
//! | resolving any other location | one listing of one key |
//! | a ranged read | one `GET`, transferring the range |
//! | a whole read, or a full stream drain | one `GET` |
//! | a whole write | one `PUT`, or a multipart upload when large |
//! | listing a level, or a whole subtree | one listing per 1000 entries |
//! | emptying or removing a prefix | one listing and one bulk delete per 1000 keys |
//!
//! [`Folder::stats`], [`File::stats`], and [`Path::stats`] report what
//! actually went out, so "this costs one request" is a thing a caller can
//! check rather than take on trust.
//!
//! # Reaching a store
//!
//! ```no_run
//! use yggdryl::IOBase;
//! use yggdryl::holder::s3;
//!
//! # fn main() -> yggdryl::Result<()> {
//! // Credentials, region, and endpoint resolve the way the AWS tools resolve
//! // them; nothing is read until the first request.
//! let mut part = s3::file("s3://trades/lake/year=2026/part.parquet")?;
//!
//! // A footer read transfers the footer, not the file.
//! let footer = part.read_range_bytes(part.size().saturating_sub(8), 8)?;
//! assert_eq!(footer.len(), 8);
//! # Ok(())
//! # }
//! ```
//!
//! A store that is not AWS - MinIO, Ceph, an S3-compatible gateway - is named
//! by its endpoint, either in the location itself or through [`S3Options`]:
//!
//! ```
//! use yggdryl::holder::s3::{Credentials, S3Options};
//!
//! let options = S3Options::default()
//!     .with_endpoint("http://localhost:9000")
//!     .with_credentials(Credentials::new("minioadmin", "minioadmin"))
//!     .with_region("us-east-1");
//! assert_eq!(options.endpoint(), Some("http://localhost:9000"));
//! ```

use std::sync::Arc;

use crate::holder::Holder;
use crate::{Error, Result, Url};

mod client;
mod credentials;
mod file;
mod folder;
mod options;
mod path;
mod profile;
mod sign;
mod xml;

pub use client::StatsSnapshot;
pub use credentials::Credentials;
pub use file::File;
pub use folder::Folder;
pub use options::S3Options;
pub use path::Path;

use client::Client;

/// Hold the S3 resource `url` names, resolving its role only when asked.
///
/// Construction performs no request. A caller who already knows the role
/// reaches for [`file`] or [`folder`] instead.
///
/// # Errors
///
/// Returns a refusal when `url` is not an `s3` location naming a bucket.
pub fn located(url: &str) -> Result<Holder> {
    located_with(url, S3Options::default())
}

/// Hold the S3 resource `url` names, configured by `options`.
///
/// # Errors
///
/// Returns a refusal when `url` is not an `s3` location naming a bucket.
pub fn located_with(url: &str, options: S3Options) -> Result<Holder> {
    let url = parse(url)?;
    let client = Arc::new(Client::new(&url, options)?);
    Path::new(client, url).map(Holder::S3Path)
}

/// Hold the object `url` names, whether or not it exists yet.
///
/// # Errors
///
/// Returns a refusal when `url` is not an `s3` location naming a bucket.
pub fn file(url: &str) -> Result<File> {
    file_with(url, S3Options::default())
}

/// Hold the object `url` names, configured by `options`.
///
/// # Errors
///
/// Returns a refusal when `url` is not an `s3` location naming a bucket.
pub fn file_with(url: &str, options: S3Options) -> Result<File> {
    let url = parse(url)?;
    let client = Arc::new(Client::new(&url, options)?);
    File::new(client, url)
}

/// Hold the prefix or bucket `url` names, whether or not anything is under it.
///
/// # Errors
///
/// Returns a refusal when `url` is not an `s3` location naming a bucket.
pub fn folder(url: &str) -> Result<Folder> {
    folder_with(url, S3Options::default())
}

/// Hold the prefix or bucket `url` names, configured by `options`.
///
/// # Errors
///
/// Returns a refusal when `url` is not an `s3` location naming a bucket.
pub fn folder_with(url: &str, options: S3Options) -> Result<Folder> {
    let url = parse(url)?;
    let client = Arc::new(Client::new(&url, options)?);
    Folder::new(client, url)
}

/// Parse an `s3` location, refusing anything else.
fn parse(url: &str) -> Result<Url> {
    let url = Url::from_str(url)?;
    if url.bucket().is_none() {
        return Err(Error::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("expected an s3 location naming a bucket, got {url}"),
        )));
    }
    Ok(url)
}

/// The bucket and the decoded object key `url` addresses.
///
/// The key is decoded because that is what the store names the object by; the
/// URL keeps the escaped spelling, because that is what a URL path admits.
fn split_location(url: &Url) -> Result<(String, String)> {
    let bucket = url.bucket().ok_or_else(|| {
        Error::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("expected an s3 location naming a bucket, got {url}"),
        ))
    })?;
    let key = url.key().unwrap_or_default();
    Ok((decode_key(bucket), decode_key(key)))
}

/// Decode one key's percent escapes, keeping its separators.
///
/// A key a store gave back round-trips exactly; text that will not decode is
/// kept as it stands rather than silently mangled.
fn decode_key(key: &str) -> String {
    crate::uri::percent_decode(key, "s3 key")
        .map_or_else(|_| key.to_owned(), std::borrow::Cow::into_owned)
}

/// Spell a raw key as URI path text, so a URL can carry it.
///
/// The separators stay separators - only what sits between them is escaped -
/// so a listing's key becomes a location whose segments still line up with the
/// prefixes above it.
fn encode_key_path(key: &str) -> String {
    key.split('/')
        .map(|segment| crate::uri::percent_encode_segment(segment).into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

/// The same location with any credentials the caller wrote into it removed.
///
/// A URL is what a handle reports, logs, and puts in an error, so a secret a
/// caller spelled into one is used to build the client and then never rendered
/// again.
fn without_credentials(url: Url) -> Url {
    if url.user().is_none() {
        return url;
    }
    let authority = url.authority().as_str();
    let Some((_, host)) = authority.rsplit_once('@') else {
        return url;
    };
    let rebuilt = format!("{}://{host}{}", url.scheme().as_str(), url.path().as_str());
    Url::from_str(&rebuilt).unwrap_or(url)
}

#[cfg(test)]
mod tests;
