//! Objects on Amazon S3, Google Cloud Storage, and Azure Blob Storage as
//! [`IOBase`](crate::IOBase) handles.
//!
//! The same three roles every storage backend supplies, spoken to each store
//! over its own REST API directly - no SDK, no async runtime, no object-store
//! abstraction in between:
//!
//! - [`Path`] is the generic location, which resolves to whichever of the two
//!   it turns out to name.
//! - [`Folder`] is the container: a key prefix, or a whole bucket or container.
//! - [`File`] is the leaf: one object, read by range and written whole.
//!
//! Which store answers is the location's scheme. `s3`, `s3a`, and `s3n` name
//! Amazon S3; `gs` and `gcs` name Google Cloud Storage; `az`, `abfs`, `abfss`,
//! `wasb`, and `wasbs` name Azure Blob Storage. The extra spellings differ only
//! in the connector that once read them, so a location written by another tool
//! selects this backend and addresses the same object. A handle reports the
//! spelling it was handed, in its location and in its refusals.
//!
//! Everything else follows from [`IOBase`](crate::IOBase) rather than being
//! written again here - globs, Hive partitions, page caching through
//! [`Buffered`](crate::holder::buffered::Buffered), content codings, IPC,
//! Parquet, Avro, Iceberg tables. A record reader over an object is the same
//! reader that runs over a mapped file, whichever store holds it.
//!
//! # One backend, three dialects
//!
//! What the three stores share is nearly all of it: the connection pool, the
//! retry budget, the streaming reader that resumes a cut transfer, the staging
//! model behind a write, the listing pipeline, and the three handle roles. What
//! they do not share is named by [`Provider`] and lives in that store's own
//! module - how a request is authorized, what its path is, how a large value is
//! uploaded, and how a listing, an object, and a refusal are spelled.
//!
//! Configuration follows the same split. [`ObjectOptions`] holds what all three
//! have - an endpoint, a region, credentials, timeouts, retries, part sizes,
//! encryption - and [`AwsOptions`], [`GoogleOptions`], and [`AzureOptions`]
//! hold what one has. A knob therefore has exactly one owner, and nothing
//! pretends the three stores are one store.
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
//! | a whole write | one write, or a chunked upload when large |
//! | listing a level, or a whole subtree | one listing per page |
//! | emptying or removing a prefix | one listing and one bulk delete per batch |
//!
//! [`Folder::stats`], [`File::stats`], and [`Path::stats`] report what
//! actually went out, so "this costs one request" is a thing a caller can
//! check rather than take on trust.
//!
//! # Reaching a store
//!
//! ```no_run
//! use yggdryl::IOBase;
//! use yggdryl::holder::object;
//!
//! # fn main() -> yggdryl::Result<()> {
//! // Credentials, region, and endpoint resolve the way each store's own tools
//! // resolve them; nothing is read until the first request.
//! let mut part = object::file("s3://trades/lake/year=2026/part.parquet")?;
//!
//! // A footer read transfers the footer, not the file.
//! let footer = part.read_range_bytes(part.size().saturating_sub(8), 8)?;
//! assert_eq!(footer.len(), 8);
//!
//! // The same call, against the other two stores.
//! let google = object::file("gs://trades/lake/year=2026/part.parquet")?;
//! let azure = object::file("abfss://lake@trades.dfs.core.windows.net/part.parquet")?;
//! assert_eq!(google.key(), "lake/year=2026/part.parquet");
//! assert_eq!(azure.key(), "part.parquet");
//! # Ok(())
//! # }
//! ```
//!
//! A store that is not the published one - MinIO, Ceph, an S3-compatible
//! gateway, `fake-gcs-server`, Azurite - is named by its endpoint, either in the
//! location itself or through [`ObjectOptions`]:
//!
//! ```
//! use yggdryl::holder::object::{Credentials, ObjectOptions};
//!
//! let options = ObjectOptions::default()
//!     .with_endpoint("http://localhost:9000")
//!     .with_credentials(Credentials::new("minioadmin", "minioadmin"))
//!     .with_region("us-east-1");
//! assert_eq!(options.endpoint(), Some("http://localhost:9000"));
//! ```

use std::sync::Arc;

use crate::holder::Holder;
use crate::{Error, Result, Url};

mod answer;
pub mod aws;
pub mod azure;
mod client;
mod encryption;
mod file;
mod folder;
pub mod google;
mod options;
mod path;
mod properties;
mod provider;
mod request;
mod sigv4;
mod xml;

pub use aws::{AssumedRole, AwsOptions, Checksum, Credentials};
pub use azure::{AzureOptions, BlobType};
pub use client::StatsSnapshot;
pub use encryption::{CustomerKey, Encryption, KmsKey};
pub use file::File;
pub use folder::Folder;
pub use google::GoogleOptions;
pub use options::ObjectOptions;
pub use path::Path;
pub use provider::Provider;

use client::Client;

/// Hold the resource `url` names, resolving its role only when asked.
///
/// Construction performs no request. A caller who already knows the role
/// reaches for [`file()`] or [`folder()`] instead.
///
/// # Errors
///
/// Returns a refusal when `url` is not an object-store location naming a
/// container.
pub fn located(url: &str) -> Result<Holder> {
    located_with(url, ObjectOptions::default())
}

/// Hold the resource `url` names, configured by `options`.
///
/// # Errors
///
/// Returns a refusal when `url` is not an object-store location naming a
/// container.
pub fn located_with(url: &str, options: ObjectOptions) -> Result<Holder> {
    let url = parse(url)?;
    let client = Arc::new(Client::new(&url, options)?);
    Path::new(client, url).map(Holder::ObjectPath)
}

/// Hold the object `url` names, whether or not it exists yet.
///
/// # Errors
///
/// Returns a refusal when `url` is not an object-store location naming a
/// container.
pub fn file(url: &str) -> Result<File> {
    file_with(url, ObjectOptions::default())
}

/// Hold the object `url` names, configured by `options`.
///
/// # Errors
///
/// Returns a refusal when `url` is not an object-store location naming a
/// container.
pub fn file_with(url: &str, options: ObjectOptions) -> Result<File> {
    let url = parse(url)?;
    let client = Arc::new(Client::new(&url, options)?);
    File::new(client, url)
}

/// Hold the prefix or container `url` names, whether or not anything is under
/// it.
///
/// # Errors
///
/// Returns a refusal when `url` is not an object-store location naming a
/// container.
pub fn folder(url: &str) -> Result<Folder> {
    folder_with(url, ObjectOptions::default())
}

/// Hold the prefix or bucket `url` names, configured by `options`.
///
/// # Errors
///
/// Returns a refusal when `url` is not an object-store location naming a
/// container.
pub fn folder_with(url: &str, options: ObjectOptions) -> Result<Folder> {
    let url = parse(url)?;
    let client = Arc::new(Client::new(&url, options)?);
    Folder::new(client, url)
}

/// Hold the object `key` names in `container` on `provider`, whether or not it
/// exists yet.
///
/// This is the raw-name entry point, and it is where encoding belongs: a key
/// is arbitrary UTF-8, so `a b/c.txt` and `100%/done.txt` are ordinary names
/// here while a URL cannot spell either without escaping them. The store is an
/// argument because a raw name, unlike a location, does not say which store
/// holds it. A caller holding a location reaches for [`file()`].
///
/// ```
/// use yggdryl::holder::object::{self, Provider};
///
/// # fn main() -> yggdryl::Result<()> {
/// let handle = object::file_at(Provider::Aws, "trades", "lake/a b/part.parquet")?;
/// assert_eq!(handle.key(), "lake/a b/part.parquet");
/// // The location it reports escapes what a URL cannot carry.
/// assert_eq!(
///     handle.url().to_string(),
///     "s3://trades/lake/a%20b/part.parquet"
/// );
///
/// // The same name on another store is the same key under another scheme.
/// let google = object::file_at(Provider::Google, "trades", "lake/part.parquet")?;
/// assert_eq!(google.url().to_string(), "gs://trades/lake/part.parquet");
/// # Ok(())
/// # }
/// ```
///
/// # Errors
///
/// Returns a refusal when `container` is empty or cannot form a location.
pub fn file_at(provider: Provider, container: &str, key: &str) -> Result<File> {
    file_at_with(provider, container, key, ObjectOptions::default())
}

/// Hold the object `key` names in `container`, configured by `options`.
///
/// # Errors
///
/// Returns a refusal when `container` is empty or cannot form a location.
pub fn file_at_with(
    provider: Provider,
    container: &str,
    key: &str,
    options: ObjectOptions,
) -> Result<File> {
    let url = key_url(provider, container, key)?;
    let client = Arc::new(Client::new(&url, options)?);
    File::new(client, url)
}

/// Hold the prefix `key` names in `container` on `provider`, entries or not.
///
/// The raw-name counterpart of [`folder()`], per [`file_at`].
///
/// # Errors
///
/// Returns a refusal when `container` is empty or cannot form a location.
pub fn folder_at(provider: Provider, container: &str, key: &str) -> Result<Folder> {
    folder_at_with(provider, container, key, ObjectOptions::default())
}

/// Hold the prefix `key` names in `container`, configured by `options`.
///
/// # Errors
///
/// Returns a refusal when `container` is empty or cannot form a location.
pub fn folder_at_with(
    provider: Provider,
    container: &str,
    key: &str,
    options: ObjectOptions,
) -> Result<Folder> {
    let url = key_url(provider, container, key)?;
    let client = Arc::new(Client::new(&url, options)?);
    Folder::new(client, url)
}

/// Hold the resource `key` names in `container`, resolving its role when asked.
///
/// The raw-name counterpart of [`located`], per [`file_at`].
///
/// # Errors
///
/// Returns a refusal when `container` is empty or cannot form a location.
pub fn path_at(provider: Provider, container: &str, key: &str) -> Result<Path> {
    path_at_with(provider, container, key, ObjectOptions::default())
}

/// Hold the resource `key` names in `container`, configured by `options`.
///
/// # Errors
///
/// Returns a refusal when `container` is empty or cannot form a location.
pub fn path_at_with(
    provider: Provider,
    container: &str,
    key: &str,
    options: ObjectOptions,
) -> Result<Path> {
    let url = key_url(provider, container, key)?;
    let client = Arc::new(Client::new(&url, options)?);
    Path::new(client, url)
}

/// The canonical location of a raw `key` in `container` on `provider`.
fn key_url(provider: Provider, container: &str, key: &str) -> Result<Url> {
    if container.is_empty() {
        return Err(Error::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!(
                "expected {} {} name, got \"\"",
                provider.described(),
                provider.container_word()
            ),
        )));
    }
    Url::from_str(&format!(
        "{}://{}/{}",
        provider.scheme().as_str(),
        crate::uri::percent_encode_segment(container),
        encode_key_path(key)
    ))
}

/// Parse an object-store location, refusing anything else.
fn parse(url: &str) -> Result<Url> {
    let url = Url::from_str(url)?;
    if Provider::from_scheme(url.scheme()).is_none() || url.bucket().is_none() {
        return Err(no_container(&url));
    }
    Ok(url)
}

/// Refuse a location that names no container, without rendering its
/// credentials.
///
/// A caller may have written keys into the location, and a refusal is exactly
/// the text that reaches a log or a traceback, so it names the location the
/// handle would have reported rather than the one it was handed.
fn no_container(url: &Url) -> Error {
    let word = Provider::from_scheme(url.scheme())
        .map_or("container", |provider| provider.container_word());
    Error::Io(std::io::Error::new(
        std::io::ErrorKind::InvalidInput,
        format!(
            "expected an object-store location naming a {word}, got {}",
            without_credentials(url.clone())
        ),
    ))
}

/// The container and the decoded object key `url` addresses.
///
/// The key is decoded because that is what the store names the object by; the
/// URL keeps the escaped spelling, because that is what a URL path admits.
fn split_location(url: &Url) -> Result<(String, String)> {
    let container = url.bucket().ok_or_else(|| no_container(url))?;
    let key = url.key().unwrap_or_default();
    Ok((decode_key(container), decode_key(key)))
}

/// Decode one key's percent escapes, keeping its separators.
///
/// A key a store gave back round-trips exactly; text that will not decode is
/// kept as it stands rather than silently mangled.
fn decode_key(key: &str) -> String {
    crate::uri::percent_decode(key, "object key")
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
/// again. A password is what makes user information a credential - it is the
/// half a client signs with, and the half worth hiding - so user information
/// without one is a name and stays: `abfss://trades@lake.dfs.core.windows.net`
/// writes Azure's container there, and a handle that dropped it would report a
/// location naming a different container than the one it was handed.
fn without_credentials(url: Url) -> Url {
    if url.password().is_none() {
        return url;
    }
    let authority = url.authority().as_str();
    let Some((user, host)) = authority.rsplit_once('@') else {
        return url;
    };
    // Only Azure reads the user position as a name rather than as a key, so it
    // is the only place anything there is worth keeping.
    let kept = match url.scheme().is_az() {
        true => match user.split_once(':') {
            Some((container, _)) if !container.is_empty() => format!("{container}@"),
            _ => String::new(),
        },
        false => String::new(),
    };
    let rebuilt = format!(
        "{}://{kept}{host}{}",
        url.scheme().as_str(),
        url.path().as_str()
    );
    Url::from_str(&rebuilt).unwrap_or(url)
}

#[cfg(test)]
mod tests;
