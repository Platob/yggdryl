//! XML for Analysis 1.1: the SOAP protocol analytical clients discover
//! metadata and run statements over, as a record medium and as a server.
//!
//! This crate is the medium and the provider over the `yggdryl` core: the
//! core's dispatch learns the `.xmla` rowset document by [`register`] -
//! after it, a handle named `application/xmla+xml` composes [`Xmla`] and
//! reads and writes rowsets through the core's generic record doors - and
//! [`Xmla::new`] wraps any handle explicitly whether or not it ran.
//!
//! XMLA has two methods. `Discover` asks a provider about itself - its data
//! sources, its properties, the request types it answers - and about a
//! catalog - its tables, their columns, the datatypes it speaks - and is
//! answered with a *rowset*: a table spelled as XML, its columns declared once
//! in an XML Schema and its rows written under it. `Execute` runs a command
//! and, for a tabular provider, is answered with a rowset too. Both travel in
//! a SOAP 1.1 envelope over HTTP `POST`, and a failure is a SOAP fault whose
//! detail carries the protocol's own `Error` element.
//!
//! | Layer | Owns |
//! | --- | --- |
//! | [`vocabulary`] | the request types, the properties and their enumerations, the restrictions |
//! | [`request`] | `Discover`, `Execute`, the session header, one `Request` read out of and written into an envelope |
//! | [`response`] | the rowset, empty and dataset answers, the XMLA `Error` a fault carries, the streaming response writer |
//! | [`rowset`] | a `Field` as the rowset's XML Schema, a record `Serie` as its rows, and both read back; the `_xHHHH_` element names; the XML Schema type table |
//! | [`dbtype`] | OLE DB's `DBTYPE_*` indicators, what `DBSCHEMA_COLUMNS` states a column as |
//! | [`options`], [`media`] | the `.xmla` record medium: [`XmlaOptions`] and [`Xmla`] |
//! | [`definitions`] | the rowsets this crate's provider answers, each as a `Field` with its restriction columns |
//! | [`service`] | the provider over a [`Warehouse`](yggdryl::Warehouse): every Discover answered from the catalogs it serves through the warehouse traits, every Execute run through the expression grammar's `Plan` against that warehouse; under the `http` feature, `Service::route` answers it on an `http::Server` |
//!
//! The SOAP envelope and fault are XML's own, in [`yggdryl::soap`], and the
//! HTTP it travels over is the crate's `http` server and
//! client; this module speaks XMLA over them.
//!
//! The provider is a *tabular* one: its data sources are the catalogs of a
//! [`Warehouse`](yggdryl::Warehouse) - any [`Catalog`](yggdryl::Catalog), a
//! folder read as namespaces and tables among them - and a statement is the
//! expression grammar's plan, such as `select symbol, price from trades
//! where price is not null limit 10`, run against that warehouse. The
//! multidimensional rowsets and the MDX a multidimensional provider answers
//! are read and refused by name, never answered.

#![deny(unsafe_code)]

pub mod dbtype;
pub mod definitions;
pub mod media;
pub mod options;
pub mod request;
pub mod response;
pub mod rowset;
#[cfg(feature = "http")]
mod server;
pub mod service;
pub mod vocabulary;

pub use dbtype::DbType;
pub use media::{Xmla, XmlaEncoding, overwrite_arrow_reader, read_batch_reader, read_field};
pub use options::XmlaOptions;
pub use request::{Command, Discover, Execute, Request, RequestMethod, Session};
pub use response::{Answer, Response, XmlaError, fault, write_empty, write_fault, write_rowset};
pub use rowset::{Rowset, XsdType, decode_name, encode_name};
pub use service::{Execution, Service, ServiceOptions};
pub use vocabulary::{
    Access, AuthenticationMode, AxisFormat, Content, Format, MdxSupport, Method, PropertyList,
    ProviderType, RequestType, Restrictions, StateSupport, property,
};

/// The namespace of the two methods, their arguments and their responses.
pub const NAMESPACE: &str = "urn:schemas-microsoft-com:xml-analysis";

/// The namespace of a rowset's `root`.
pub const ROWSET_NAMESPACE: &str = "urn:schemas-microsoft-com:xml-analysis:rowset";

/// The namespace of a multidimensional dataset's `root`.
pub const MDDATASET_NAMESPACE: &str = "urn:schemas-microsoft-com:xml-analysis:mddataset";

/// The namespace of the `root` a command that answers nothing is answered
/// with.
pub const EMPTY_NAMESPACE: &str = "urn:schemas-microsoft-com:xml-analysis:empty";

/// The namespace of the `Error` element a fault's detail carries.
pub const EXCEPTION_NAMESPACE: &str = "urn:schemas-microsoft-com:xml-analysis:exception";

/// The namespace of the `sql:field` attribute a rowset schema names a column
/// with.
pub const SQL_NAMESPACE: &str = "urn:schemas-microsoft-com:xml-sql";

pub(crate) use request::invalid;

/// Register the XMLA encoding with the core, so a handle whose media type
/// is `application/xmla+xml` - a `.xmla` name - reads and writes rowset
/// documents through the core's generic record doors: `Holder::from_url`
/// composes [`Xmla`], `RecordOptions::for_media_type` answers the XMLA
/// options, and a folder catalog lists a `.xmla` leaf as the table it is.
/// Idempotent, and never needed to wrap a handle explicitly with
/// [`Xmla::new`].
pub fn register() {
    yggdryl::media::register(std::sync::Arc::new(XmlaEncoding));
}
