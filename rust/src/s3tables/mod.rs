//! Amazon S3 Tables: the catalog of a table bucket.
//!
//! S3 Tables is Amazon's managed Iceberg store: a *table bucket* holds
//! namespaces, a namespace holds Iceberg tables, and each table's files live
//! at an `s3:` warehouse location ending `--table-s3` that the service
//! chooses and the [`s3`](crate::s3) backend reads and writes. The service,
//! not a listing, names a table's current metadata file. This module is the
//! client of that control plane: it creates, describes, lists and deletes the
//! three levels, and moves a table's metadata location - a commit.
//!
//! [`S3Tables`] is the door. Every request is an
//! [`http::Request`](crate::http::Request) signed by
//! [`Request::with_sigv4`](crate::http::Request::with_sigv4) for `s3tables`
//! as an [`aws::Session`](crate::aws::Session) answers, sent over the
//! session's own HTTP client to the endpoint the client states, else
//! [`Session::service_endpoint`](crate::aws::Session::service_endpoint). The
//! service takes no anonymous request, so a session answering no credential
//! set is refused before anything is sent:
//!
//! ```
//! use yggdryl::Arn;
//! use yggdryl::aws::{Credentials, Session};
//! use yggdryl::s3tables::S3Tables;
//!
//! # fn main() -> yggdryl::Result<()> {
//! // Nothing is read and nothing is reached until a verb sends a request.
//! let session = Session::new()
//!     .with_environment(false)
//!     .with_credentials(Credentials::new("AKIAIOSFODNN7EXAMPLE", "a-secret"));
//! let tables = S3Tables::new(session.clone());
//!
//! // A table bucket is addressed by its ARN, which names its region, and the
//! // region names the host.
//! let lake = Arn::from_str("arn:aws:s3tables:eu-west-3:123456789012:bucket/lake")?;
//! let region = tables.region_of(Some(&lake))?;
//! assert_eq!(region, "eu-west-3");
//! assert_eq!(tables.endpoint_url(&region)?, "https://s3tables.eu-west-3.amazonaws.com");
//!
//! // A name the service's own model refuses costs no request.
//! assert!(tables.create_table_bucket("Lake").is_err());
//! assert!(tables.get_namespace(&lake, "no-hyphens").is_err());
//!
//! // An endpoint the caller states is where every request goes: the FIPS and
//! // dual-stack switches choose among the published hosts, and botocore
//! // turns both off beside one.
//! let gateway = S3Tables::new(session.with_use_fips_endpoint(true))
//!     .try_with_endpoint_url("http://localhost:4566")?;
//! assert_eq!(gateway.endpoint_url(&region)?, "http://localhost:4566");
//! # Ok(())
//! # }
//! ```
//!
//! # The request count
//!
//! Every verb is one request, and a listing one per page: nothing probes
//! before it acts or reads back after a write.
//!
//! | verb | request |
//! | --- | --- |
//! | building a client, a listing, or any value | none |
//! | [`create_table_bucket`](S3Tables::create_table_bucket) | 1 `PUT /buckets` |
//! | [`get_table_bucket`](S3Tables::get_table_bucket) | 1 `GET /buckets/{arn}` |
//! | [`table_buckets`](S3Tables::table_buckets) | 1 `GET /buckets` per page of 250 |
//! | [`remove_table_bucket`](S3Tables::remove_table_bucket) | 1 `DELETE /buckets/{arn}` |
//! | [`create_namespace`](S3Tables::create_namespace) | 1 `PUT /namespaces/{arn}` |
//! | [`get_namespace`](S3Tables::get_namespace) | 1 `GET /namespaces/{arn}/{namespace}` |
//! | [`namespaces`](S3Tables::namespaces) | 1 `GET /namespaces/{arn}` per page of 250 |
//! | [`remove_namespace`](S3Tables::remove_namespace) | 1 `DELETE /namespaces/{arn}/{namespace}` |
//! | [`create_table`](S3Tables::create_table) | 1 `PUT /tables/{arn}/{namespace}` |
//! | [`get_table`](S3Tables::get_table), [`get_table_by_arn`](S3Tables::get_table_by_arn) | 1 `GET /get-table` |
//! | [`tables`](S3Tables::tables) | 1 `GET /tables/{arn}` per page of 250 |
//! | [`rename_table`](S3Tables::rename_table) | 1 `PUT /tables/{arn}/{namespace}/{name}/rename` |
//! | [`remove_table`](S3Tables::remove_table) | 1 `DELETE /tables/{arn}/{namespace}/{name}` |
//! | [`get_table_metadata_location`](S3Tables::get_table_metadata_location) | 1 `GET .../metadata-location` |
//! | [`update_table_metadata_location`](S3Tables::update_table_metadata_location) | 1 `PUT .../metadata-location` |
//!
//! Two things add a request. A read or a removal that meets a `5xx`, a
//! `429`, an error type botocore reads as throttling or a transport failure
//! is sent again under the HTTP client's attempts and backoff; a `PUT` that
//! creates, renames or commits is sent once, a throttled one included -
//! where botocore would send it again - because the service may have acted
//! on it. And a request refused because its key lapsed or is unknown is
//! signed and sent once more when the session then answers another set -
//! [`Request::with_sigv4`](crate::http::Request::with_sigv4)'s rule.
//!
//! # What an answer means
//!
//! The error type decides, never the status alone - a gateway's `404` says
//! nothing about a table. A `get_*` verb answered `NotFoundException` is
//! [`Error::Absent`](crate::Error::Absent), a `remove_*` verb so answered
//! succeeds, and a `create_*` verb answered `ConflictException` is
//! [`Error::Conflict`](crate::Error::Conflict). Every other refusal is
//! [`Error::Remote`](crate::Error::Remote) with the service's status, error
//! type and message - a `NotFoundException` from a create, rename, commit or
//! listing included, since there the bucket or the namespace above may be
//! what is missing.
//!
//! # A commit
//!
//! [`get_table_metadata_location`](S3Tables::get_table_metadata_location)
//! answers the current metadata file, the warehouse the next one is written
//! into, and a version token;
//! [`update_table_metadata_location`](S3Tables::update_table_metadata_location)
//! names the next file current under that token. The token moves on every
//! change, so a commit under a stale one is the service's `409`, never a
//! replaced winner.
//!
//! # Names
//!
//! The values are descriptions, never handles - [`TableDescription`],
//! [`NamespaceSummary`], [`TableSummary`] are the model's names - so none
//! shadows the warehouse's [`Table`](crate::Table) and
//! [`Namespace`](crate::Namespace).
//!
//! Behind the non-default `s3tables` feature, which implies `s3` and
//! `iceberg`.

mod bucket;
mod catalog;
mod client;
mod listing;
mod namespace;
mod table;

pub use bucket::TableBucket;
pub use catalog::{S3TablesCatalog, S3TablesNamespace};
pub(crate) use catalog::{create, locate, not_a_table, open_or_create};
pub use client::S3Tables;
pub use listing::{NamespaceSummaries, TableBuckets, TableSummaries};
pub use namespace::NamespaceSummary;
pub use table::{TableDescription, TableMetadataLocation, TableSummary, TableVersion};
