//! XML for Analysis 1.1: the SOAP protocol analytical clients discover
//! metadata and run statements over, as a record medium and as a server.
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
//! | [`catalog`] | a catalog over a folder: its schemas and tables as the leaves and table folders under it |
//! | [`service`] | the provider: every Discover answered from the catalogs, every Execute run through the expression grammar's `Plan` |
//! | [`server`] | the HTTP endpoint the provider is reached at |
//!
//! The SOAP envelope, fault and HTTP binding are XML's own, in
//! [`crate::xml::soap`]; this module speaks XMLA over them.
//!
//! The provider is a *tabular* one: its data sources are catalogs of tables,
//! a table being any leaf a record medium reads or any folder that reads as
//! one - a partitioned tree, an Iceberg table - and a statement is the
//! expression grammar's plan, such as `select symbol, price from trades
//! where price is not null limit 10`, run against the catalog. The
//! multidimensional rowsets and the MDX a multidimensional provider answers
//! are read and refused by name, never answered.

pub mod catalog;
pub mod dbtype;
pub mod definitions;
pub mod media;
pub mod options;
pub mod request;
pub mod response;
pub mod rowset;
pub mod server;
pub mod service;
pub mod vocabulary;

pub use catalog::{Catalog, Table};
pub use dbtype::DbType;
pub use media::{Xmla, overwrite_arrow_reader, read_batch_reader, read_field};
pub(crate) use media::row_size;
pub use options::XmlaOptions;
pub use request::{Command, Discover, Execute, Request, RequestMethod, Session};
pub use response::{Answer, Response, XmlaError, fault, write_empty, write_fault, write_rowset};
pub use rowset::{Rowset, XsdType, decode_name, encode_name};
pub use server::{Running, Server, ServerOptions};
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
