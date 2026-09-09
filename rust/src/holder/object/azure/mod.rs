//! Azure Blob Storage, and Data Lake Storage Gen2 accounts.
//!
//! Azure puts the storage account in the endpoint rather than in the path, has
//! four ways to authorize a request rather than one, and assembles a large blob
//! from staged blocks rather than from numbered parts. Those three facts are
//! what this dialect owns; everything above it is the backend's.

pub(crate) mod options;

pub use options::{AzureOptions, BlobType, DEFAULT_API_VERSION, DEVELOPMENT_ACCOUNT};
