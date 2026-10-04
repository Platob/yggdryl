//! Amazon S3 Tables: the catalog of a table bucket.
//!
//! S3 Tables is Amazon's managed Iceberg store. A *table bucket* holds
//! namespaces, a namespace holds tables, and each table is an Iceberg table
//! whose files live at a warehouse location the service chooses - an `s3:`
//! location ending `--table-s3`, read and written through the
//! [`s3`](crate::s3) backend like any other - and whose current metadata
//! file is named by the service rather than found by listing. This module is
//! the client of that control plane: it creates, describes, lists and
//! deletes the three levels, and moves a table's metadata location, which is
//! what a commit to one of these tables is.
//!
//! [`S3Tables`] is the door. Every request it sends is an
//! [`http::Request`](crate::http::Request) signed by
//! [`Request::with_sigv4`](crate::http::Request::with_sigv4) for the
//! `s3tables` service as an [`aws::Session`](crate::aws::Session) answers,
//! sent over the session's own HTTP client to
//! [`Session::service_endpoint`](crate::aws::Session::service_endpoint) -
//! unless the client states its own. The service takes no anonymous
//! request, so a session that answers no credential set is refused by name
//! before anything is sent:
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
//! Every verb is one request, and a listing is one per page: nothing probes
//! before it acts, and nothing is read back after a write. The suite counts
//! them against a fake of the service rather than taking this table's word.
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
//! Two things add a request, and both are the service's doing. A read or a
//! removal - the operations its model marks safe to send again - that meets
//! a `5xx`, a `429`, an error type botocore reads as throttling or a
//! transport failure is sent again under the HTTP client's attempts and
//! backoff; a `PUT` that creates, renames or commits is sent once, because
//! the service may have acted on the one it did not answer - a throttled one
//! included, where botocore would send it again. And a request the service
//! refuses because the key that signed it lapsed or is not recognized is
//! signed and sent once more, only when the session then answers another
//! set - [`Request::with_sigv4`](crate::http::Request::with_sigv4)'s rule.
//!
//! # What an answer means
//!
//! Act once, then read the failure: a `get_*` verb the service answers
//! `NotFoundException` is [`Error::Absent`](crate::Error::Absent); a
//! `remove_*` verb it answers so has nothing left to do, which is success; a
//! `create_*` verb it answers `ConflictException` is
//! [`Error::Conflict`](crate::Error::Conflict). Every other refusal is
//! [`Error::Remote`](crate::Error::Remote) with the service's own status,
//! error type and message - a `NotFoundException` from a verb that creates,
//! renames, commits or lists included, because there the missing thing may
//! be the table bucket or the namespace above what was addressed, and only
//! the service's message says which. The error type decides, never the
//! status alone: a `404` from a gateway is not the service saying a table
//! is gone.
//!
//! # A commit
//!
//! [`get_table_metadata_location`](S3Tables::get_table_metadata_location)
//! answers where a table's current metadata file is, the warehouse the next
//! one is written into, and a version token. Writing the next metadata file
//! is the [`s3`](crate::s3) backend's work;
//! [`update_table_metadata_location`](S3Tables::update_table_metadata_location)
//! then names it as the table's own under that token. The token moves on
//! every change to the table, so a commit made under one the table has moved
//! past is refused with the service's `409` instead of replacing the commit
//! that won.
//!
//! # Names
//!
//! The values are descriptions, never handles: [`TableDescription`] is what
//! `GetTable` answers, [`NamespaceSummary`] and [`TableSummary`] are the
//! model's own names for what a namespace reading and a listing answer, so
//! none of them shadows the warehouse's [`Table`](crate::Table) and
//! [`Namespace`](crate::Namespace), which hold one. The verbs are the
//! crate's: `create_*`, `get_*`, `remove_*`, and the two the service names
//! for itself, `rename_table` and `update_table_metadata_location`.
//!
//! The module is behind the non-default `s3tables` feature, which implies
//! `s3` and `iceberg`.

mod bucket;
mod catalog;
pub(crate) mod client;
mod listing;
mod namespace;
mod table;

pub use bucket::TableBucket;
pub use catalog::{S3TablesCatalog, S3TablesNamespace};
pub(crate) use catalog::{create, locate};
pub use client::S3Tables;
pub use listing::{NamespaceSummaries, TableBuckets, TableSummaries};
pub use namespace::NamespaceSummary;
pub use table::{TableDescription, TableMetadataLocation, TableSummary, TableVersion};
