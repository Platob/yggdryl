//! Which store a location addresses, and the shape that store has.
//!
//! One backend serves Amazon S3, Google Cloud Storage, and Azure Blob Storage,
//! so exactly one value says which of the three is answering. It is the sole
//! dispatcher: every place the three differ - a hostname, a header prefix, a
//! request's path, an upload protocol, a limit - reads it, and no other value
//! carries the same fact.
//!
//! What it does *not* do is widen: a store's own capability keeps that store's
//! spelling, in that store's options. This names the store, and nothing here
//! pretends the three are the same store.

use crate::{Error, Result, Scheme};

/// One of the three object stores this backend speaks to.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Provider {
    /// Amazon S3, and every store that answers its REST API.
    Aws,
    /// Google Cloud Storage.
    Google,
    /// Azure Blob Storage, including Data Lake Storage Gen2 accounts.
    Azure,
}

impl Provider {
    /// Every provider, in the order their schemes are listed.
    pub const ALL: [Self; 3] = [Self::Aws, Self::Google, Self::Azure];

    /// The store `scheme` addresses.
    ///
    /// `s3`, `s3a`, `s3n` name the first; `gs` and `gcs` the second; `az`,
    /// `abfs`, `abfss`, `wasb`, `wasbs` the third. A scheme that names no store
    /// answers `None`.
    pub const fn from_scheme(scheme: &Scheme) -> Option<Self> {
        if scheme.is_s3() {
            return Some(Self::Aws);
        }
        if scheme.is_gs() {
            return Some(Self::Google);
        }
        if scheme.is_az() {
            return Some(Self::Azure);
        }
        None
    }

    /// The scheme a location this store holds is canonically written with.
    pub const fn scheme(&self) -> Scheme {
        match self {
            Self::Aws => Scheme::S3,
            Self::Google => Scheme::GS,
            Self::Azure => Scheme::AZ,
        }
    }

    /// The store's name in a refusal, which is the canonical scheme.
    ///
    /// [`Error::Remote`](crate::Error::Remote) names the service that refused,
    /// and it is `&'static str`, so the three names live here.
    pub const fn service(&self) -> &'static str {
        match self {
            Self::Aws => "s3",
            Self::Google => "gs",
            Self::Azure => "az",
        }
    }

    /// The store's own display name, for a refusal a caller reads.
    pub const fn described(&self) -> &'static str {
        match self {
            Self::Aws => "Amazon S3",
            Self::Google => "Google Cloud Storage",
            Self::Azure => "Azure Blob Storage",
        }
    }

    /// What the store calls the thing a bucket is.
    ///
    /// Two of the three say bucket and the third says container, and a refusal
    /// that says the wrong one sends a caller to the wrong documentation.
    pub const fn container_word(&self) -> &'static str {
        match self {
            Self::Aws | Self::Google => "bucket",
            Self::Azure => "container",
        }
    }

    /// The prefix user metadata travels under.
    pub const fn metadata_prefix(&self) -> &'static str {
        match self {
            Self::Aws => "x-amz-meta-",
            Self::Google => "x-goog-meta-",
            Self::Azure => "x-ms-meta-",
        }
    }

    /// The prefix every header the store defines for itself carries.
    pub const fn header_prefix(&self) -> &'static str {
        match self {
            Self::Aws => "x-amz-",
            Self::Google => "x-goog-",
            Self::Azure => "x-ms-",
        }
    }

    /// The smallest part the store accepts for every part but the last.
    ///
    /// S3 refuses a part below 5 MiB. Google's resumable upload takes any
    /// multiple of 256 KiB, and Azure takes any block at all, so the floor is
    /// the granularity each of those actually requires.
    pub const fn min_part_size(&self) -> u64 {
        match self {
            Self::Aws => 5 * 1024 * 1024,
            Self::Google => 256 * 1024,
            Self::Azure => 64 * 1024,
        }
    }

    /// The largest part the store accepts.
    pub const fn max_part_size(&self) -> u64 {
        match self {
            // One `UploadPart`, and also the largest single `PutObject`.
            Self::Aws => 5 * 1024 * 1024 * 1024,
            // A resumable upload's chunk is bounded only by the object.
            Self::Google => 5 * 1024 * 1024 * 1024 * 1024,
            // One `Put Block`.
            Self::Azure => 4000 * 1024 * 1024,
        }
    }

    /// The largest value the store takes in a single request.
    pub const fn max_single_put(&self) -> u64 {
        match self {
            Self::Aws => 5 * 1024 * 1024 * 1024,
            Self::Google => 5 * 1024 * 1024 * 1024 * 1024,
            // One `Put Blob` of a block blob.
            Self::Azure => 5000 * 1024 * 1024,
        }
    }

    /// The most keys one bulk delete carries.
    ///
    /// S3 takes a thousand in one `DeleteObjects`; the other two batch
    /// sub-requests into one HTTP request and cap the batch lower.
    pub const fn max_delete_batch(&self) -> usize {
        match self {
            Self::Aws => 1000,
            Self::Google => 100,
            Self::Azure => 256,
        }
    }

    /// The most entries the store returns in one listing page.
    pub const fn max_list_page(&self) -> u16 {
        match self {
            Self::Aws | Self::Azure => 5000,
            Self::Google => 1000,
        }
    }

    /// The listing page size to ask for when nothing else is said.
    ///
    /// A thousand on every store: it is S3's own maximum, Google's maximum, and
    /// the size Azure's SDKs ask for, so one number is the default everywhere
    /// and the cost model reads the same.
    pub const fn default_list_page(&self) -> u16 {
        1000
    }

    /// Whether the store puts a container in the endpoint hostname by default.
    ///
    /// S3 does on AWS itself. Google's JSON API never does, and Azure's
    /// container is always a path segment.
    pub const fn defaults_to_virtual_hosting(&self) -> bool {
        matches!(self, Self::Aws)
    }

    /// Refuse `operation`, which this store does not have.
    pub(super) fn unsupported(&self, operation: &'static str) -> Error {
        Error::unsupported(operation, self.described())
    }
}

impl std::fmt::Display for Provider {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.described())
    }
}

impl std::str::FromStr for Provider {
    type Err = Error;

    /// Read a store's name in any spelling a configuration file uses.
    fn from_str(value: &str) -> Result<Self> {
        let lowered = value.trim().to_ascii_lowercase();
        match lowered.as_str() {
            "aws" | "s3" | "s3a" | "s3n" | "amazon" | "minio" => Ok(Self::Aws),
            "google" | "gs" | "gcs" | "gcp" => Ok(Self::Google),
            "azure" | "az" | "abfs" | "abfss" | "wasb" | "wasbs" | "adls" | "blob" => {
                Ok(Self::Azure)
            }
            _ => Err(Error::Parse {
                target: "object store provider",
                position: 0,
                reason: format!("expected aws, google, or azure, got {value:?}").into(),
            }),
        }
    }
}
