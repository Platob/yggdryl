//! Objects on Amazon S3, Google Cloud Storage, and Azure Blob Storage as
//! [`IOBase`](crate::IOBase) handles.
//!
//! The same three roles every storage backend supplies, spoken to each store
//! over its own REST API directly - no SDK, no async runtime, no object-store
//! abstraction in between:
//!
//! - [`S3Path`] is the generic location, which resolves to whichever of the
//!   two it turns out to name.
//! - [`S3Folder`] is the container: a key prefix, or a whole bucket or
//!   container.
//! - [`S3File`] is the leaf: one object, read by range and written whole.
//!
//! Which store answers is the location's scheme. `s3`, `s3a`, and `s3n` name
//! Amazon S3; `gs` and `gcs` name Google Cloud Storage; `az`, `abfs`, `abfss`,
//! `wasb`, and `wasbs` name Azure Blob Storage. The extra spellings differ only
//! in the connector that once read them, so a location written by another tool
//! selects this backend and addresses the same object. A handle reports the
//! spelling it was handed, in its location and in its refusals.
//!
//! The ten schemes are [`S3_BACKEND`]'s, the
//! [`StorageBackend`] the core claims under
//! them until `yggdryl-s3` does: [`Holder::from_url`] holds a location of any
//! of them as the [`S3Path`] it names, [`Holder::Registered`] as every
//! claimed backend's handle is, and [`Holder::downcast_ref`] answers it as
//! the role it is.
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
//! Configuration follows the same split. [`S3Options`] holds what all three
//! have - an endpoint, a region, credentials, timeouts, retries, part sizes,
//! encryption - and [`AwsOptions`], [`GoogleOptions`], and [`AzureOptions`]
//! hold what one has. A knob therefore has exactly one owner, and nothing
//! pretends the three stores are one store. Who this process is to AWS (the
//! profile, the credential chain, a role, an IAM Identity Center sign-in) is
//! not the store's knob at all but the [`Session`](crate::aws::Session) every
//! AWS request signs with, reached through [`S3Options::with_session`].
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
//! | resolving any other location | one listing of one key, which also states an object's size: no `HEAD` follows |
//! | a ranged read | one `GET`, transferring the range |
//! | a footer-first read ([`IOBase::read_tail_bytes`](crate::IOBase::read_tail_bytes)) | one `GET` with `Range: bytes=-N`, the size learned from its `Content-Range`; on Azure one `HEAD` and one ranged `GET` |
//! | a whole read, or a full stream drain | one `GET` |
//! | a whole write | one write, or a chunked upload when large |
//! | listing a level, or a whole subtree | one listing per page |
//! | emptying or removing a prefix | one listing and one bulk delete per batch |
//!
//! [`S3Folder::stats`], [`S3File::stats`], and [`S3Path::stats`] report what
//! actually went out, so "this costs one request" is a thing a caller can
//! check rather than take on trust.
//!
//! # Reaching a store
//!
//! ```no_run
//! use yggdryl::IOBase;
//! use yggdryl::s3;
//!
//! # fn main() -> yggdryl::Result<()> {
//! // Credentials, region, and endpoint resolve the way each store's own tools
//! // resolve them; nothing is read until the first request.
//! let mut part = s3::file("s3://trades/lake/year=2026/part.parquet")?;
//!
//! // A footer read transfers the footer, not the file.
//! let footer = part.read_range_bytes(part.size().saturating_sub(8), 8)?;
//! assert_eq!(footer.len(), 8);
//!
//! // The same call, against the other two stores.
//! let google = s3::file("gs://trades/lake/year=2026/part.parquet")?;
//! let azure = s3::file("abfss://lake@trades.dfs.core.windows.net/part.parquet")?;
//! assert_eq!(google.key(), "lake/year=2026/part.parquet");
//! assert_eq!(azure.key(), "part.parquet");
//! # Ok(())
//! # }
//! ```
//!
//! A store that is not the published one - MinIO, Ceph, an S3-compatible
//! gateway, `fake-gcs-server`, Azurite - is named by its endpoint, either in the
//! location itself or through [`S3Options`]:
//!
//! ```
//! use yggdryl::s3::{Credentials, S3Options};
//!
//! let options = S3Options::default()
//!     .with_endpoint("http://localhost:9000")
//!     .with_credentials(Credentials::new("minioadmin", "minioadmin"))
//!     .with_region("us-east-1");
//! assert_eq!(options.endpoint(), Some("http://localhost:9000"));
//! ```

use std::sync::Arc;

use crate::holder::{Holder, StorageBackend};
use crate::{Error, Result, Scheme, Url};

pub(crate) mod answer;
pub mod aws;
pub mod azure;
pub(crate) mod client;
mod encryption;
pub(crate) mod file;
mod folder;
pub mod google;
pub(crate) mod options;
mod path;
pub(crate) mod properties;
mod provider;
mod request;
pub(crate) mod xml;

pub use crate::aws::Credentials;
pub use aws::{AwsOptions, Checksum};
pub use azure::{AzureOptions, BlobType};
pub use client::StatsSnapshot;
pub use encryption::{CustomerKey, Encryption, KmsKey};
pub use file::S3File;
pub use folder::S3Folder;
pub use google::GoogleOptions;
pub use options::S3Options;
pub use path::S3Path;
pub use provider::Provider;

use client::Client;

/// The object stores' byte backend: Amazon S3, Google Cloud Storage and
/// Azure Blob Storage behind one [`StorageBackend`], holding a location of
/// any of their ten schemes as the [`S3Path`] it names.
///
/// The core claims it itself, before the register answers anything, until
/// `yggdryl-s3`'s `install()` does. A location's query states the store's
/// properties in the names [`S3Options::with_properties`] reads, and
/// [`Holder::from_url`] refuses a parameter it does not read before the
/// backend is asked. Holding a location sends nothing.
///
/// ```
/// use yggdryl::holder::{Holder, StorageBackend, backend_for};
/// use yggdryl::s3::{S3_BACKEND, S3Path};
/// use yggdryl::{Scheme, Url};
///
/// # fn main() -> yggdryl::Result<()> {
/// let claimed = backend_for(&Scheme::GS).expect("the core claims `gs`");
/// assert_eq!(claimed.name(), "yggdryl-s3");
/// assert!(S3_BACKEND.is_property("region"));
/// assert!(!S3_BACKEND.is_property("versionId"));
///
/// let url = Url::from_str("s3://trades/lake/part.parquet")?;
/// let held = Holder::from_url(&url, [("region", "eu-west-1")])?;
/// let path = held.downcast_ref::<S3Path>().expect("an undecided location");
/// assert_eq!(path.key(), "lake/part.parquet");
/// assert_eq!(path.stats().requests, 0);
/// # Ok(())
/// # }
/// ```
#[derive(Debug)]
pub struct S3Backend;

/// The one [`S3Backend`], claimed under the ten object-store schemes.
pub static S3_BACKEND: S3Backend = S3Backend;

/// The schemes the object stores' locations spell, the keys [`S3_BACKEND`]
/// is claimed under: each [`Provider`]'s own scheme and the spellings older
/// connectors wrote for it.
static SCHEMES: [Scheme; 10] = [
    Scheme::S3,
    Scheme::S3A,
    Scheme::S3N,
    Scheme::GS,
    Scheme::GCS,
    Scheme::AZ,
    Scheme::ABFS,
    Scheme::ABFSS,
    Scheme::WASB,
    Scheme::WASBS,
];

impl StorageBackend for S3Backend {
    fn name(&self) -> &'static str {
        "yggdryl-s3"
    }

    fn schemes(&self) -> &'static [Scheme] {
        &SCHEMES
    }

    /// Whether [`S3Options::with_properties`] reads `name`, in any
    /// vocabulary and spelling it accepts ([`S3Options::is_property`]).
    fn is_property(&self, name: &str) -> bool {
        S3Options::is_property(name)
    }

    /// The [`S3Path`] `url` names, on a client configured by `properties`
    /// ([`located_with`]); the query is already taken off `url` and stated
    /// first among `properties`.
    fn holder(&self, url: &Url, properties: &[(String, String)]) -> Result<Holder> {
        let options =
            S3Options::from_properties(properties.iter().map(|(name, value)| (name, value)))?;
        located_with(&url.to_string(), options)
    }
}

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
    located_with(url, S3Options::default())
}

/// Hold the resource `url` names, configured by `options`.
///
/// # Errors
///
/// Returns a refusal when `url` is not an object-store location naming a
/// container.
pub fn located_with(url: &str, options: S3Options) -> Result<Holder> {
    let url = parse(url)?;
    let client = Arc::new(Client::new(&url, options)?);
    S3Path::new(client, url).map(Holder::from)
}

/// Hold the object `url` names, whether or not it exists yet.
///
/// # Errors
///
/// Returns a refusal when `url` is not an object-store location naming a
/// container.
pub fn file(url: &str) -> Result<S3File> {
    file_with(url, S3Options::default())
}

/// Hold the object `url` names, configured by `options`.
///
/// # Errors
///
/// Returns a refusal when `url` is not an object-store location naming a
/// container.
pub fn file_with(url: &str, options: S3Options) -> Result<S3File> {
    let url = parse(url)?;
    let client = Arc::new(Client::new(&url, options)?);
    S3File::new(client, url)
}

/// Hold the prefix or container `url` names, whether or not anything is under
/// it.
///
/// # Errors
///
/// Returns a refusal when `url` is not an object-store location naming a
/// container.
pub fn folder(url: &str) -> Result<S3Folder> {
    folder_with(url, S3Options::default())
}

/// Hold the prefix or bucket `url` names, configured by `options`.
///
/// # Errors
///
/// Returns a refusal when `url` is not an object-store location naming a
/// container.
pub fn folder_with(url: &str, options: S3Options) -> Result<S3Folder> {
    let url = parse(url)?;
    let client = Arc::new(Client::new(&url, options)?);
    S3Folder::new(client, url)
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
/// use yggdryl::s3::{self, Provider};
///
/// # fn main() -> yggdryl::Result<()> {
/// let handle = s3::file_at(Provider::Aws, "trades", "lake/a b/part.parquet")?;
/// assert_eq!(handle.key(), "lake/a b/part.parquet");
/// // The location it reports escapes what a URL cannot carry.
/// assert_eq!(
///     handle.url().to_string(),
///     "s3://trades/lake/a%20b/part.parquet"
/// );
///
/// // The same name on another store is the same key under another scheme.
/// let google = s3::file_at(Provider::Google, "trades", "lake/part.parquet")?;
/// assert_eq!(google.url().to_string(), "gs://trades/lake/part.parquet");
/// # Ok(())
/// # }
/// ```
///
/// # Errors
///
/// Returns a refusal when `container` is empty or cannot form a location.
pub fn file_at(provider: Provider, container: &str, key: &str) -> Result<S3File> {
    file_at_with(provider, container, key, S3Options::default())
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
    options: S3Options,
) -> Result<S3File> {
    let url = key_url(provider, container, key)?;
    let client = Arc::new(Client::new(&url, options)?);
    S3File::new(client, url)
}

/// Hold the prefix `key` names in `container` on `provider`, entries or not.
///
/// The raw-name counterpart of [`folder()`], per [`file_at`].
///
/// # Errors
///
/// Returns a refusal when `container` is empty or cannot form a location.
pub fn folder_at(provider: Provider, container: &str, key: &str) -> Result<S3Folder> {
    folder_at_with(provider, container, key, S3Options::default())
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
    options: S3Options,
) -> Result<S3Folder> {
    let url = key_url(provider, container, key)?;
    let client = Arc::new(Client::new(&url, options)?);
    S3Folder::new(client, url)
}

/// Hold the resource `key` names in `container`, resolving its role when asked.
///
/// The raw-name counterpart of [`located`], per [`file_at`].
///
/// # Errors
///
/// Returns a refusal when `container` is empty or cannot form a location.
pub fn path_at(provider: Provider, container: &str, key: &str) -> Result<S3Path> {
    path_at_with(provider, container, key, S3Options::default())
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
    options: S3Options,
) -> Result<S3Path> {
    let url = key_url(provider, container, key)?;
    let client = Arc::new(Client::new(&url, options)?);
    S3Path::new(client, url)
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
