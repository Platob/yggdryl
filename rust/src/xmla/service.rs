//! The provider: what every request is answered with.
//!
//! A [`Service`] holds the catalogs it serves and answers a [`Request`]. A
//! Discover is answered from the [`definitions`](super::definitions) - the
//! provider's own rowsets filled from its options, the catalog rowsets filled
//! by listing the catalogs - narrowed by the request's restrictions. An
//! Execute parses its statement as the expression grammar's [`Plan`],
//! resolves the table it names inside a catalog, runs it, and streams the
//! rows back as a rowset. Anything that cannot be answered is a SOAP fault
//! carrying the protocol's `Error`, `Client` when the request is at fault and
//! `Server` when this side is.
//!
//! The service is the protocol without its transport: [`Service::answer`]
//! writes the response for a parsed request into any sink, and
//! [`Service::handle`] does the same for the bytes of one message, which is
//! what [`super::server`] puts on a socket and what a test drives directly.

use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use smol_str::{SmolStr, format_smolstr};

use crate::expression::{Location, Plan, Source, Target};
use crate::soap::{Fault, FaultCode, Fragment};
use crate::{ArrowCastOptions, DataType, Error, Field, Result, Scalar, Serie, SerieReader, Uuid};

use super::catalog::{Catalog, Table};
use super::dbtype::DbType;
use super::definitions::{Definition, definition_of, definitions};
use super::request::{Command, Discover, Execute, Request, RequestMethod, Session};
use super::response::{ACTOR, XmlaError, fault};
use super::rowset::{Rowset, XsdType};
use super::vocabulary::{
    Access, AuthenticationMode, AxisFormat, Content, Format, Method, PropertyList, ProviderType,
    RequestType, StateSupport, property,
};

/// The error codes this provider's faults carry.
pub mod code {
    /// The request is not XML for Analysis this provider reads.
    pub const BAD_REQUEST: u32 = 0x0001;
    /// The request type is one this provider does not answer.
    pub const UNSUPPORTED_REQUEST_TYPE: u32 = 0x0002;
    /// A restriction names a column the rowset is not restricted by.
    pub const BAD_RESTRICTION: u32 = 0x0003;
    /// A property carries a value it cannot take.
    pub const BAD_PROPERTY: u32 = 0x0004;
    /// The `Format` asks for a shape this provider does not answer.
    pub const UNSUPPORTED_FORMAT: u32 = 0x0005;
    /// The statement does not parse.
    pub const BAD_STATEMENT: u32 = 0x0006;
    /// The statement names a catalog or table that is not here.
    pub const UNKNOWN_TABLE: u32 = 0x0007;
    /// The statement writes, and this provider is read-only.
    pub const READ_ONLY: u32 = 0x0008;
    /// The command is not a statement.
    pub const UNSUPPORTED_COMMAND: u32 = 0x0009;
    /// Running the statement failed.
    pub const EXECUTION_FAILED: u32 = 0x000A;
    /// Listing or reading a catalog failed.
    pub const CATALOG_FAILED: u32 = 0x000B;
    /// A session header names a session this provider does not hold.
    pub const BAD_SESSION: u32 = 0x000C;
}

/// What the provider says about itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServiceOptions {
    /// The `ProviderName` property and `DISCOVER_DATASOURCES`'s.
    pub provider_name: String,
    /// The `ProviderVersion` and `DBMSVersion` properties.
    pub provider_version: String,
    /// The one data source's `DataSourceName`.
    pub data_source_name: String,
    /// The data source's description.
    pub data_source_description: String,
    /// The `URL` a client invokes the methods at, when known.
    pub url: Option<String>,
    /// Whether a statement may write: `insert`, `upsert`, `delete`,
    /// `create`. Off, every write is refused by name.
    pub writable: bool,
}

impl ServiceOptions {
    /// The defaults: this crate's name and version, one data source named
    /// after the crate, read-only.
    #[must_use]
    pub fn new() -> Self {
        Self {
            provider_name: "yggdryl".to_owned(),
            provider_version: env!("CARGO_PKG_VERSION").to_owned(),
            data_source_name: "yggdryl".to_owned(),
            data_source_description: "Catalogs of record media, served over XML for Analysis"
                .to_owned(),
            url: None,
            writable: false,
        }
    }

    /// Return these options naming the endpoint a client invokes.
    #[must_use]
    pub fn with_url(mut self, url: impl Into<String>) -> Self {
        self.url = Some(url.into());
        self
    }

    /// Return these options letting statements write.
    #[must_use]
    pub const fn with_writable(mut self, writable: bool) -> Self {
        self.writable = writable;
        self
    }
}

impl Default for ServiceOptions {
    fn default() -> Self {
        Self::new()
    }
}

/// The rows an Execute answers.
// One value per Execute, moved once into the response writer; boxing the
// stream would buy an allocation and nothing else.
#[allow(clippy::large_enum_variant)]
pub enum Execution {
    /// A rowset: its columns and the stream of its rows.
    Rowset {
        /// The columns.
        rowset: Rowset,
        /// The rows, one record column per batch.
        rows: SerieReader,
    },
    /// Nothing: a `Content` of `None`, or a write.
    Empty,
}

/// The provider over a set of catalogs.
#[derive(Debug)]
pub struct Service {
    catalogs: Vec<Catalog>,
    options: ServiceOptions,
    sessions: AtomicU64,
}

impl Service {
    /// A provider with no catalog yet.
    #[must_use]
    pub const fn new(options: ServiceOptions) -> Self {
        Self {
            catalogs: Vec::new(),
            options,
            sessions: AtomicU64::new(0),
        }
    }

    /// Return this provider serving `catalog` too.
    #[must_use]
    pub fn with_catalog(mut self, catalog: Catalog) -> Self {
        self.catalogs.push(catalog);
        self
    }

    /// The options.
    #[must_use]
    pub const fn options(&self) -> &ServiceOptions {
        &self.options
    }

    /// The catalogs, in the order added.
    #[must_use]
    pub fn catalogs(&self) -> &[Catalog] {
        &self.catalogs
    }

    /// The catalog `name` names, matched exactly.
    #[must_use]
    pub fn catalog(&self, name: &str) -> Option<&Catalog> {
        self.catalogs.iter().find(|catalog| catalog.name() == name)
    }

    /// Answer the bytes of one message with the bytes of its response: a
    /// message that is not a request is answered with a `Client` fault, and
    /// a request that cannot be answered with the fault it earns.
    ///
    /// # Errors
    ///
    /// Returns only a failure writing to `writer`.
    pub fn handle<W: Write>(&self, message: &[u8], writer: W) -> Result<W> {
        match Request::from_bytes(message) {
            Ok(request) => self.answer(&request, writer),
            Err(error) => super::response::write_fault(
                writer,
                &client_fault(code::BAD_REQUEST, format!("{error}")),
            ),
        }
    }

    /// Answer one request, writing the whole response - or the fault it
    /// earns - to `writer`.
    ///
    /// # Errors
    ///
    /// Returns only a failure writing to `writer`.
    pub fn answer<W: Write>(&self, request: &Request, writer: W) -> Result<W> {
        if let Some(fault) = not_understood(request) {
            return super::response::write_fault(writer, &fault);
        }
        let header = match self.session_header(request) {
            Ok(header) => header,
            Err(fault) => return super::response::write_fault(writer, &fault),
        };
        match request.method() {
            RequestMethod::Discover(discover) => match self.discover(discover) {
                Ok((definition, rows)) => super::response::write_rowset(
                    writer,
                    &header,
                    Method::Discover,
                    definition.rowset(),
                    std::iter::once(Ok(rows)),
                    content_of(discover.properties()).unwrap_or(Content::DEFAULT),
                ),
                Err(fault) => super::response::write_fault(writer, &fault),
            },
            RequestMethod::Execute(execute) => match self.execute(execute) {
                // Streamed as the rows arrive: a batch failing once the answer
                // has begun is reported inside the rowset, and the document
                // is closed rather than cut.
                Ok(Execution::Rowset { rowset, rows }) => super::response::write_rowset_reporting(
                    writer,
                    &header,
                    Method::Execute,
                    &rowset,
                    rows,
                    content_of(execute.properties()).unwrap_or(Content::DEFAULT),
                )
                .map(|(writer, _)| writer),
                Ok(Execution::Empty) => {
                    super::response::write_empty(writer, &header, Method::Execute)
                }
                Err(fault) => super::response::write_fault(writer, &fault),
            },
        }
    }

    /// The header blocks a response carries: the `Session` a `BeginSession`
    /// opened, or the one a `Session` or an `EndSession` names - echoed the
    /// way the reference providers echo it, without `mustUnderstand`.
    ///
    /// Sessions carry no state here, so every identifier this provider hands
    /// out is honoured and one it did not is refused by name. A fault about a
    /// header entry carries no detail, as SOAP 1.1 has it.
    fn session_header(&self, request: &Request) -> std::result::Result<Vec<Fragment>, Fault> {
        let session = request
            .session()
            .map_err(|error| Fault::client(error.to_string()).with_actor(ACTOR))?;
        let block = match session {
            None => return Ok(Vec::new()),
            Some(Session::Begin) => Session::Continue(self.new_session_id()),
            Some(Session::Continue(id) | Session::End(id)) => {
                if crate::uuid_parse(id.as_bytes()).is_err() {
                    return Err(Fault::client(format!(
                        "the session {id:?} is not one this provider opened"
                    ))
                    .with_actor(ACTOR));
                }
                Session::Continue(id)
            }
        };
        block
            .into_answer_fragment()
            .map(|fragment| vec![fragment])
            .map_err(|error| Fault::client(error.to_string()).with_actor(ACTOR))
    }

    /// A fresh session identifier: a UUIDv7 of the instant and a counter.
    fn new_session_id(&self) -> SmolStr {
        let micros = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| i64::try_from(elapsed.as_micros()).unwrap_or(i64::MAX))
            .unwrap_or(0);
        let count = self.sessions.fetch_add(1, Ordering::Relaxed);
        let payload = crate::xxhash::xxh3(&count.to_le_bytes());
        Uuid::from_v7(micros, payload).map_or_else(
            |_| SmolStr::new(format!("{count:032x}")),
            |uuid| SmolStr::new(uuid.to_string()),
        )
    }

    /// Answer a Discover: the rowset its request type names, narrowed by its
    /// restrictions.
    ///
    /// # Errors
    ///
    /// Returns the fault the request earns: an unsupported request type, a
    /// restriction naming no column, a format other than tabular, a catalog
    /// that cannot be listed.
    pub fn discover(
        &self,
        discover: &Discover,
    ) -> std::result::Result<(&'static Definition, Serie), Fault> {
        let format = discover
            .properties()
            .format()
            .map_err(|error| client_fault_or_server(code::BAD_PROPERTY, error))?;
        if format == Format::Multidimensional {
            return Err(client_fault(
                code::UNSUPPORTED_FORMAT,
                "a Discover is answered as a rowset: Format must be Tabular or Native",
            ));
        }
        let definition = definition_of(discover.request_type()).ok_or_else(|| {
            client_fault(
                code::UNSUPPORTED_REQUEST_TYPE,
                format!(
                    "this provider does not answer {}; DISCOVER_SCHEMA_ROWSETS lists what it does",
                    discover.request_type()
                ),
            )
        })?;
        for (column, _) in discover.restrictions().entries() {
            if definition.restricts_by(column).is_none() {
                return Err(client_fault(
                    code::BAD_RESTRICTION,
                    format!(
                        "{} is not restricted by {column:?}; its restrictions are {}",
                        definition.request_type(),
                        definition
                            .restrictions()
                            .iter()
                            .map(|field| field.name())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                ));
            }
        }
        let rows = self
            .rows_of(definition, discover)
            .map_err(|error| client_fault_or_server(code::CATALOG_FAILED, error))?;
        let kept: Vec<Scalar> = rows
            .into_iter()
            .filter(|row| admits(row, discover.restrictions()))
            .collect();
        let serie = Serie::from_scalars(Arc::clone(definition.rowset().shared_field()), kept)
            .map_err(|error| server_fault(code::CATALOG_FAILED, error))?;
        Ok((definition, serie))
    }

    /// Every row of `definition`, as the named records the rowset's field
    /// reads.
    fn rows_of(&self, definition: &Definition, discover: &Discover) -> Result<Vec<Scalar>> {
        let properties = discover.properties();
        match definition.request_type() {
            RequestType::DiscoverDatasources => Ok(vec![record([
                ("DataSourceName", text(&self.options.data_source_name)),
                (
                    "DataSourceDescription",
                    text(&self.options.data_source_description),
                ),
                (
                    "URL",
                    self.options.url.as_deref().map_or(Scalar::Null, text),
                ),
                (
                    "DataSourceInfo",
                    text(&format!(
                        "Provider={};Data Source={}",
                        self.options.provider_name, self.options.data_source_name
                    )),
                ),
                ("ProviderName", text(&self.options.provider_name)),
                (
                    "ProviderType",
                    Scalar::from_sequence([text(ProviderType::Tdp.as_str())]),
                ),
                (
                    "AuthenticationMode",
                    text(AuthenticationMode::Unauthenticated.as_str()),
                ),
            ])?]),
            RequestType::DiscoverProperties => self.property_rows(properties),
            RequestType::DiscoverSchemaRowsets => definitions()
                .iter()
                .map(|definition| {
                    record([
                        ("SchemaName", text(definition.request_type().as_str())),
                        ("SchemaGuid", Scalar::Null),
                        (
                            "Restrictions",
                            Scalar::from_sequence(
                                definition
                                    .restrictions()
                                    .into_iter()
                                    .map(|field| {
                                        record([
                                            ("Name", text(field.name())),
                                            (
                                                "Type",
                                                text(
                                                    XsdType::of(field.dtype())
                                                        .unwrap_or(XsdType::String)
                                                        .as_str(),
                                                ),
                                            ),
                                        ])
                                    })
                                    .collect::<Result<Vec<_>>>()?,
                            ),
                        ),
                        ("Description", text(definition.description())),
                    ])
                })
                .collect(),
            RequestType::DiscoverEnumerators => enumerator_rows(),
            RequestType::DiscoverKeywords => KEYWORDS
                .iter()
                .map(|keyword| record([("Keyword", text(keyword))]))
                .collect(),
            RequestType::DiscoverLiterals => literal_rows(),
            RequestType::DbschemaCatalogs => self
                .catalogs
                .iter()
                .map(|catalog| {
                    record([
                        ("CATALOG_NAME", text(catalog.name())),
                        (
                            "DESCRIPTION",
                            catalog.description().map_or(Scalar::Null, text),
                        ),
                        ("ROLES", Scalar::Null),
                        ("DATE_MODIFIED", instant(catalog.modified())?),
                    ])
                })
                .collect(),
            RequestType::DbschemaSchemata => {
                let mut rows = Vec::new();
                for catalog in self.catalogs_named(properties.catalog()) {
                    for schema in catalog.schemas()? {
                        rows.push(record([
                            ("CATALOG_NAME", text(catalog.name())),
                            ("SCHEMA_NAME", text(&schema)),
                            ("SCHEMA_OWNER", Scalar::Null),
                        ])?);
                    }
                }
                Ok(rows)
            }
            RequestType::DbschemaTables => {
                let mut rows = Vec::new();
                for catalog in self.catalogs_named(properties.catalog()) {
                    for table in catalog.tables()? {
                        rows.push(record([
                            ("TABLE_CATALOG", text(table.catalog())),
                            ("TABLE_SCHEMA", table.schema().map_or(Scalar::Null, text)),
                            ("TABLE_NAME", text(table.name())),
                            ("TABLE_TYPE", text(table.table_type())),
                            ("TABLE_GUID", Scalar::Null),
                            ("DESCRIPTION", text(&table.description())),
                            ("TABLE_PROPID", Scalar::Null),
                            ("DATE_CREATED", Scalar::Null),
                            ("DATE_MODIFIED", instant(table.modified())?),
                        ])?);
                    }
                }
                Ok(rows)
            }
            RequestType::DbschemaColumns => {
                let mut rows = Vec::new();
                let wanted = discover.restrictions();
                for catalog in self.catalogs_named(properties.catalog()) {
                    for table in catalog.tables()? {
                        // A table the restrictions exclude is never read: its
                        // schema is the one answer that costs a read.
                        if !wanted
                            .get("TABLE_NAME")
                            .is_none_or(|names| names.iter().any(|name| name == table.name()))
                            || !wanted.get("TABLE_SCHEMA").is_none_or(|schemas| {
                                schemas
                                    .iter()
                                    .any(|schema| Some(schema.as_str()) == table.schema())
                            })
                        {
                            continue;
                        }
                        let field = table.field()?;
                        for (position, column) in field.fields().iter().enumerate() {
                            rows.push(column_row(&table, position, column)?);
                        }
                    }
                }
                Ok(rows)
            }
            RequestType::DbschemaProviderTypes => provider_type_rows(),
            other => Err(Error::unsupported(
                "answering this request type",
                other.as_str(),
            )),
        }
    }

    /// The catalogs a request addresses: the one its `Catalog` property
    /// names, else every one.
    fn catalogs_named<'a>(&'a self, name: Option<&'a str>) -> impl Iterator<Item = &'a Catalog> {
        self.catalogs
            .iter()
            .filter(move |catalog| name.is_none_or(|name| catalog.name() == name))
    }

    /// `DISCOVER_PROPERTIES`: every property, with the value the request
    /// itself sets where it sets one.
    fn property_rows(&self, current: &PropertyList) -> Result<Vec<Scalar>> {
        let value = |name: &str, default: &str| -> Scalar {
            current.get(name).map_or_else(|| text(default), text)
        };
        let rows: Vec<(&str, &str, XsdType, Access, bool, Scalar)> = vec![
            (
                property::AXIS_FORMAT,
                "How an MDDataSet spells its axes; a tabular provider answers rowsets.",
                XsdType::String,
                Access::Write,
                false,
                value(property::AXIS_FORMAT, AxisFormat::TupleFormat.as_str()),
            ),
            (
                property::BEGIN_RANGE,
                "The first cell ordinal an MDDataSet answers; -1 is unbounded.",
                XsdType::Int,
                Access::Write,
                false,
                value(property::BEGIN_RANGE, "-1"),
            ),
            (
                property::CATALOG,
                "The catalog a request addresses: a table's first path part.",
                XsdType::String,
                Access::ReadWrite,
                false,
                value(property::CATALOG, ""),
            ),
            (
                property::CONTENT,
                "Which of the schema and the rows a result carries.",
                XsdType::String,
                Access::Write,
                false,
                value(property::CONTENT, Content::DEFAULT.as_str()),
            ),
            (
                property::CUBE,
                "The cube a command runs against; this provider has tables, not cubes.",
                XsdType::String,
                Access::ReadWrite,
                false,
                value(property::CUBE, ""),
            ),
            (
                property::DATA_SOURCE_INFO,
                "The connection text DISCOVER_DATASOURCES states.",
                XsdType::String,
                Access::ReadWrite,
                false,
                value(
                    property::DATA_SOURCE_INFO,
                    &format!(
                        "Provider={};Data Source={}",
                        self.options.provider_name, self.options.data_source_name
                    ),
                ),
            ),
            (
                property::DBMS_VERSION,
                "The version of the engine behind the provider.",
                XsdType::String,
                Access::Read,
                false,
                text(&self.options.provider_version),
            ),
            (
                property::END_RANGE,
                "The last cell ordinal an MDDataSet answers; -1 is unbounded.",
                XsdType::Int,
                Access::Write,
                false,
                value(property::END_RANGE, "-1"),
            ),
            (
                property::FORMAT,
                "The shape a result takes: Tabular or Native, both a rowset here.",
                XsdType::String,
                Access::Write,
                false,
                value(property::FORMAT, Format::Tabular.as_str()),
            ),
            (
                property::LOCALE_IDENTIFIER,
                "The numeric locale of the request; unread.",
                XsdType::UnsignedInt,
                Access::ReadWrite,
                false,
                value(property::LOCALE_IDENTIFIER, ""),
            ),
            (
                property::PASSWORD,
                "Deprecated in XMLA 1.1; accepted and ignored.",
                XsdType::String,
                Access::Write,
                false,
                text(""),
            ),
            (
                property::PROVIDER_NAME,
                "The provider's name.",
                XsdType::String,
                Access::Read,
                false,
                text(&self.options.provider_name),
            ),
            (
                property::PROVIDER_VERSION,
                "The provider's version.",
                XsdType::String,
                Access::Read,
                false,
                text(&self.options.provider_version),
            ),
            (
                property::STATE_SUPPORT,
                "Session headers are honoured; no state is kept between requests.",
                XsdType::String,
                Access::Read,
                false,
                text(StateSupport::Sessions.as_str()),
            ),
            (
                property::TIMEOUT,
                "Seconds to wait for a request to succeed; unread.",
                XsdType::UnsignedInt,
                Access::ReadWrite,
                false,
                value(property::TIMEOUT, ""),
            ),
            (
                property::USER_NAME,
                "The user name the provider associates with the request; none.",
                XsdType::String,
                Access::Read,
                false,
                text(""),
            ),
            (
                property::VISUAL_MODE,
                "How visual totals behave; a tabular provider has none.",
                XsdType::Int,
                Access::Write,
                false,
                value(property::VISUAL_MODE, "0"),
            ),
        ];
        rows.into_iter()
            .map(|(name, description, xsd, access, required, value)| {
                record([
                    ("PropertyName", text(name)),
                    ("PropertyDescription", text(description)),
                    // `string`, `int`: the XML Schema name without its prefix,
                    // as the reference providers spell a property's type.
                    ("PropertyType", text(xsd.local_name())),
                    ("PropertyAccessType", text(access.as_str())),
                    ("IsRequired", Scalar::from(required)),
                    ("Value", value),
                ])
            })
            .collect()
    }

    /// Run an Execute: parse its statement as a plan, resolve the table it
    /// names inside a catalog, and answer the stream of its rows.
    ///
    /// # Errors
    ///
    /// Returns the fault the request earns: a command that is not a
    /// statement, a statement that does not parse, a write against a
    /// read-only provider, a table that is not here, a format other than
    /// tabular, or the plan's own failure to run.
    pub fn execute(&self, execute: &Execute) -> std::result::Result<Execution, Fault> {
        let properties = execute.properties();
        let format = properties
            .format()
            .map_err(|error| client_fault_or_server(code::BAD_PROPERTY, error))?;
        if format == Format::Multidimensional {
            return Err(client_fault(
                code::UNSUPPORTED_FORMAT,
                "this provider answers rowsets: Format must be Tabular or Native",
            ));
        }
        let content = content_of(properties)
            .map_err(|error| client_fault_or_server(code::BAD_PROPERTY, error))?;
        let statement = match execute.command() {
            Command::Statement(statement) => statement,
            Command::Other(fragment) => {
                return Err(client_fault(
                    code::UNSUPPORTED_COMMAND,
                    format!(
                        "this provider runs a Statement; `{}` is not one",
                        fragment.name()
                    ),
                ));
            }
        };
        let plan: Plan = statement
            .as_str()
            .parse::<crate::Expression>()
            .and_then(crate::expression::IntoPlan::into_plan)
            .map_err(|error| {
                if looks_like_mdx(statement.as_str()) {
                    return client_fault(
                        code::BAD_STATEMENT,
                        format!(
                            "the statement is MDX, which this tabular provider does not speak; \
                             a statement is the expression grammar's, `select ... from ...`: {error}"
                        ),
                    );
                }
                client_fault_or_server(code::BAD_STATEMENT, error)
            })?;
        if !self.options.writable
            && (plan.write_section().is_some() || plan.create_target().is_some())
        {
            return Err(client_fault(
                code::READ_ONLY,
                "this provider is read-only: a statement may select, never create or write",
            ));
        }
        let plan = self
            .resolved(plan, properties.catalog())
            .map_err(|error| client_fault_or_server(code::UNKNOWN_TABLE, error))?;
        if content == Content::None {
            // Verified and not run, as the specification says of `None`.
            return Ok(Execution::Empty);
        }
        let batches = plan
            .execute()
            .map_err(|error| server_fault(code::EXECUTION_FAILED, error))?;
        let rows = SerieReader::from_arrow_reader(None, batches, ArrowCastOptions::default())
            .map_err(|error| server_fault(code::EXECUTION_FAILED, error))?;
        let rowset = Rowset::new(rows.field().clone())
            .map_err(|error| server_fault(code::EXECUTION_FAILED, error))?;
        if plan.write_section().is_some() || plan.create_target().is_some() {
            // A write yields the empty stream under the schema it wrote.
            return Ok(Execution::Empty);
        }
        Ok(Execution::Rowset { rowset, rows })
    }

    /// The plan with every table it names resolved to the location that
    /// holds it, under the catalog `default` names when a name has no
    /// catalog part.
    fn resolved(&self, plan: Plan, default: Option<&str>) -> Result<Plan> {
        let source = match plan.source() {
            Some(Source::Target(target)) => {
                Some(Source::Target(self.resolve_target(target, default)?))
            }
            Some(Source::Plan(inner)) => Some(Source::Plan(Box::new(
                self.resolved(inner.as_ref().clone(), default)?,
            ))),
            None => None,
        };
        let plan = match source {
            Some(source) => plan.read_from(source),
            None => plan,
        };
        Ok(plan)
    }

    /// One target resolved: a dotted path to the table it names, a URL kept
    /// only when it lies under a catalog this provider serves.
    fn resolve_target(&self, target: &Target, default: Option<&str>) -> Result<Target> {
        let table = match target.location() {
            Location::Url(url) => {
                let text = url.to_string();
                let inside = self.catalogs.iter().any(|catalog| {
                    catalog.url().is_some_and(|root| {
                        text.starts_with(root.to_string().trim_end_matches('/'))
                    })
                });
                if !inside {
                    return Err(Error::absent("table", format_smolstr!("{url}")));
                }
                return Ok(target.clone());
            }
            Location::Parts(parts) => self.table_of(parts, default)?,
        };
        let Some(url) = table.url() else {
            return Err(Error::absent("table", format_smolstr!("{table}")));
        };
        Ok(Target::url(url.clone()).with_properties(
            target
                .properties()
                .iter()
                .map(|(name, value)| (name.clone(), value.clone())),
        ))
    }

    /// The table a dotted path names: `table` under the default catalog,
    /// `catalog.table` or, under a default catalog, `schema.table`, and
    /// `catalog.schema.table`.
    fn table_of(&self, parts: &[SmolStr], default: Option<&str>) -> Result<Table> {
        let default_catalog = || -> Result<&Catalog> {
            match default {
                Some(name) => self
                    .catalog(name)
                    .ok_or_else(|| super::catalog::no_catalog(name)),
                None => match self.catalogs.as_slice() {
                    [only] => Ok(only),
                    [] => Err(Error::absent("catalog", "the provider serves none")),
                    _ => Err(Error::InvalidRecord {
                        path: SmolStr::new_static("$.Catalog"),
                        reason: format_smolstr!(
                            "a table name without a catalog part needs the Catalog property; \
                             the catalogs are {}",
                            self.catalogs
                                .iter()
                                .map(Catalog::name)
                                .collect::<Vec<_>>()
                                .join(", ")
                        ),
                    }),
                },
            }
        };
        match parts {
            [table] => default_catalog()?.table(None, table),
            [first, table] => match (self.catalog(first), default) {
                (Some(catalog), _) => catalog.table(None, table),
                (None, Some(_)) => default_catalog()?.table(Some(first), table),
                (None, None) => Err(super::catalog::no_catalog(first)),
            },
            [catalog, schema, table] => self
                .catalog(catalog)
                .ok_or_else(|| super::catalog::no_catalog(catalog))?
                .table(Some(schema), table),
            _ => Err(Error::InvalidRecord {
                path: SmolStr::new_static("$.from"),
                reason: format_smolstr!(
                    "expected `table`, `catalog.table` or `catalog.schema.table`, got {} parts",
                    parts.len()
                ),
            }),
        }
    }
}

/// The words the statement grammar reserves, as `DISCOVER_KEYWORDS` lists
/// them.
pub const KEYWORDS: &[&str] = &[
    "and",
    "append",
    "as",
    "asc",
    "between",
    "by",
    "case",
    "cast",
    "create",
    "delete",
    "desc",
    "distinct",
    "else",
    "end",
    "except",
    "exclude",
    "false",
    "first",
    "from",
    "glob",
    "ilike",
    "in",
    "insert",
    "into",
    "is",
    "last",
    "like",
    "limit",
    "merge",
    "not",
    "null",
    "nulls",
    "offset",
    "on",
    "or",
    "order",
    "overwrite",
    "replace",
    "select",
    "struct",
    "table",
    "then",
    "to",
    "true",
    "try_cast",
    "upsert",
    "view",
    "when",
    "where",
    "with",
];

/// Whether `row` passes every restriction: each restricted column's value -
/// any item of a sequence column - spelled as text, is one the restriction
/// admits.
fn admits(row: &Scalar, restrictions: &super::vocabulary::Restrictions) -> bool {
    let Some(entries) = row.as_struct() else {
        return false;
    };
    restrictions.entries().iter().all(|(column, admitted)| {
        let value = entries
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(column))
            .map(|(_, value)| value);
        let Some(value) = value else {
            return false;
        };
        let matches = |value: &Scalar| {
            let spelled = spelled(value);
            admitted.contains(&spelled)
        };
        match value.sequence_rows() {
            Some(items) => items.iter().any(matches),
            None => matches(value),
        }
    })
}

/// The `Content` a request asks for.
fn content_of(properties: &PropertyList) -> Result<Content> {
    properties.content()
}

/// A value as the text a rowset cell spells it, which is what a restriction
/// is compared against: the client restricts by what it read.
fn spelled(value: &Scalar) -> String {
    if let Some(text) = value.as_str() {
        return text.to_owned();
    }
    let mut spelled = Vec::new();
    if crate::xml::write_leaf_text(&mut spelled, value, "restriction").is_err() {
        return String::new();
    }
    String::from_utf8(spelled).unwrap_or_default()
}

/// `DISCOVER_ENUMERATORS`: every enumeration the vocabulary has.
fn enumerator_rows() -> Result<Vec<Scalar>> {
    let mut rows = Vec::new();
    let mut push = |name: &str, description: &str, members: Vec<(&str, &str)>| -> Result<()> {
        for (index, (element, element_description)) in members.into_iter().enumerate() {
            // The member's OLE DB ordinal: `Access` counts from one
            // (`DBPROPFLAGS_READ` is 1), every other enumeration from zero.
            let ordinal = if name == "Access" { index + 1 } else { index };
            rows.push(record([
                ("EnumName", text(name)),
                ("EnumDescription", text(description)),
                ("EnumType", text(XsdType::String.local_name())),
                ("ElementName", text(element)),
                ("ElementDescription", text(element_description)),
                ("ElementValue", text(&ordinal.to_string())),
            ])?);
        }
        Ok(())
    };
    push(
        "ProviderType",
        "The types of data a provider supports.",
        ProviderType::ALL
            .iter()
            .map(|member| (member.as_str(), member.description()))
            .collect(),
    )?;
    push(
        "AuthenticationMode",
        "How a data source authenticates.",
        AuthenticationMode::ALL
            .iter()
            .map(|member| (member.as_str(), member.description()))
            .collect(),
    )?;
    push(
        "Access",
        "How a property may be used.",
        Access::ALL
            .iter()
            .map(|member| (member.as_str(), member.description()))
            .collect(),
    )?;
    push(
        "Format",
        "The shape a result takes.",
        Format::ALL
            .iter()
            .map(|member| (member.as_str(), member.description()))
            .collect(),
    )?;
    push(
        "Content",
        "Which of the schema and the rows a result carries.",
        Content::ALL
            .iter()
            .map(|member| (member.as_str(), member.description()))
            .collect(),
    )?;
    push(
        "AxisFormat",
        "How an MDDataSet spells its axes.",
        AxisFormat::ALL
            .iter()
            .map(|member| (member.as_str(), member.description()))
            .collect(),
    )?;
    push(
        "StateSupport",
        "Whether sessions are kept.",
        StateSupport::ALL
            .iter()
            .map(|member| (member.as_str(), member.description()))
            .collect(),
    )?;
    Ok(rows)
}

/// Whether a statement the grammar refused reads as MDX: bracketed members
/// (`[Measures].[Sales]`), the `.MEMBERS` and `.CHILDREN` functions, an axis
/// clause - `ON` and the axis it names, `COLUMNS`, `ROWS`, `PAGES`,
/// `SECTIONS`, `CHAPTERS`, `AXIS(n)` or the bare ordinal `n`, on one line or
/// the next - a `WITH MEMBER` or `WITH SET` opening: the spellings no
/// expression of the grammar carries, so the refusal can say which language
/// the client spoke.
fn looks_like_mdx(statement: &str) -> bool {
    let folded = statement.to_ascii_uppercase();
    if folded.contains("].[")
        || folded.contains("[MEASURES]")
        || folded.contains(".MEMBERS")
        || folded.contains(".CHILDREN")
    {
        return true;
    }
    let mut tokens = folded.split_whitespace();
    if matches!(
        (tokens.next(), tokens.next()),
        (Some("WITH"), Some("MEMBER" | "SET"))
    ) {
        return true;
    }
    let mut tokens = folded.split_whitespace().peekable();
    while let Some(token) = tokens.next() {
        if token != "ON" {
            continue;
        }
        if let Some(next) = tokens.peek() {
            let axis = next.trim_end_matches(',');
            if matches!(axis, "COLUMNS" | "ROWS" | "PAGES" | "SECTIONS" | "CHAPTERS")
                || axis.starts_with("AXIS(")
                || axis.starts_with(|character: char| character.is_ascii_digit())
            {
                return true;
            }
        }
    }
    false
}

/// `DISCOVER_LITERALS`: how the statement grammar spells its identifiers, as
/// OLE DB's `DBLITERAL` values.
fn literal_rows() -> Result<Vec<Scalar>> {
    /// One literal: its name, its value, the characters it may not carry,
    /// the characters it may not open with, its maximum length and OLE DB's
    /// `DBLITERAL` number.
    type Literal = (
        &'static str,
        Option<&'static str>,
        Option<&'static str>,
        Option<&'static str>,
        i32,
        i32,
    );
    let literals: &[Literal] = &[
        (
            "DBLITERAL_CATALOG_NAME",
            None,
            Some("."),
            Some("0123456789"),
            -1,
            2,
        ),
        ("DBLITERAL_CATALOG_SEPARATOR", Some("."), None, None, 1, 3),
        (
            "DBLITERAL_COLUMN_ALIAS",
            None,
            Some("."),
            Some("0123456789"),
            -1,
            5,
        ),
        (
            "DBLITERAL_COLUMN_NAME",
            None,
            Some("."),
            Some("0123456789"),
            -1,
            6,
        ),
        ("DBLITERAL_QUOTE_PREFIX", Some("\""), None, None, 1, 15),
        ("DBLITERAL_QUOTE_SUFFIX", Some("\""), None, None, 1, 28),
        (
            "DBLITERAL_SCHEMA_NAME",
            None,
            Some("."),
            Some("0123456789"),
            -1,
            16,
        ),
        ("DBLITERAL_SCHEMA_SEPARATOR", Some("."), None, None, 1, 27),
        (
            "DBLITERAL_TABLE_NAME",
            None,
            Some("."),
            Some("0123456789"),
            -1,
            17,
        ),
        ("DBLITERAL_TEXT_COMMAND", None, None, None, -1, 18),
        ("DBLITERAL_USER_NAME", None, None, None, 0, 19),
    ];
    literals
        .iter()
        .map(|(name, value, invalid, starting, max, ordinal)| {
            record([
                ("LiteralName", text(name)),
                ("LiteralValue", value.map_or(Scalar::Null, text)),
                ("LiteralInvalidChars", invalid.map_or(Scalar::Null, text)),
                (
                    "LiteralInvalidStartingChars",
                    starting.map_or(Scalar::Null, text),
                ),
                ("LiteralMaxLength", Scalar::from(*max)),
                ("LiteralNameEnumValue", Scalar::from(*ordinal)),
            ])
        })
        .collect()
}

/// `DBSCHEMA_PROVIDER_TYPES`: one row per datatype family a column may take,
/// under its canonical name, typed as OLE DB types it.
fn provider_type_rows() -> Result<Vec<Scalar>> {
    let types: &[(&str, DataType)] = &[
        ("boolean", DataType::Boolean),
        ("int8", DataType::Int8),
        ("int16", DataType::Int16),
        ("int32", DataType::Int32),
        ("int64", DataType::Int64),
        ("uint8", DataType::UInt8),
        ("uint16", DataType::UInt16),
        ("uint32", DataType::UInt32),
        ("uint64", DataType::UInt64),
        ("float32", DataType::Float32),
        ("float64", DataType::Float64),
        ("decimal128(38,10)", DataType::decimal128(38, 10)?),
        ("date32", DataType::Date32),
        (
            "time64(us)",
            DataType::time64(crate::TimeUnit::Microsecond)?,
        ),
        (
            "datetime64(us, UTC)",
            DataType::DateTime64 {
                unit: crate::TimeUnit::Microsecond,
                timezone: crate::Timezone::UTC,
            },
        ),
        ("utf8", DataType::utf8()),
        ("binary", DataType::binary()),
        ("uuid", DataType::Uuid),
    ];
    types
        .iter()
        .map(|(name, dtype)| {
            let indicator = DbType::of(dtype);
            let textual = indicator == DbType::Wstr;
            record([
                ("TYPE_NAME", text(name)),
                ("DATA_TYPE", Scalar::from(indicator.code())),
                (
                    "COLUMN_SIZE",
                    Scalar::from(indicator.column_size().unwrap_or(u32::MAX)),
                ),
                (
                    "LITERAL_PREFIX",
                    if textual { text("'") } else { Scalar::Null },
                ),
                (
                    "LITERAL_SUFFIX",
                    if textual { text("'") } else { Scalar::Null },
                ),
                ("CREATE_PARAMS", Scalar::Null),
                ("IS_NULLABLE", Scalar::from(true)),
                ("CASE_SENSITIVE", Scalar::from(textual)),
                // DB_SEARCHABLE: usable in every `where` clause.
                ("SEARCHABLE", Scalar::from(4_u32)),
                (
                    "UNSIGNED_ATTRIBUTE",
                    indicator.is_unsigned().map_or(Scalar::Null, Scalar::from),
                ),
                (
                    "FIXED_PREC_SCALE",
                    Scalar::from(indicator.is_fixed_precision()),
                ),
                ("AUTO_UNIQUE_VALUE", Scalar::from(false)),
                ("LOCAL_TYPE_NAME", text(name)),
                ("MINIMUM_SCALE", Scalar::Null),
                ("MAXIMUM_SCALE", Scalar::Null),
                ("IS_LONG", Scalar::from(false)),
                ("BEST_MATCH", Scalar::from(true)),
                (
                    "IS_FIXEDLENGTH",
                    Scalar::from(indicator.column_size().is_some()),
                ),
            ])
        })
        .collect()
}

/// One `DBSCHEMA_COLUMNS` row.
fn column_row(table: &Table, position: usize, column: &Field) -> Result<Scalar> {
    // OLE DB's `DBCOLUMNFLAGS` bits, as `oledb.h` numbers them - not the
    // layout olap4j's comment describes, which olap4j itself never writes.
    /// `DBCOLUMNFLAGS_ISFIXEDLENGTH`.
    const FIXED_LENGTH: u32 = 0x10;
    /// `DBCOLUMNFLAGS_ISNULLABLE`.
    const NULLABLE: u32 = 0x20;
    /// `DBCOLUMNFLAGS_MAYBENULL`.
    const MAY_BE_NULL: u32 = 0x40;
    let dtype = column.dtype();
    let indicator = DbType::of(dtype);
    let mut flags = 0;
    if column.is_nullable() {
        flags |= NULLABLE | MAY_BE_NULL;
    }
    if indicator.column_size().is_some() || dtype.fixed_byte_width().is_some() {
        flags |= FIXED_LENGTH;
    }
    // A character or binary column states its maximum, `0` for none, as OLE
    // DB has it; every other column states no length. The bound this crate
    // keeps counts bytes, which is what both lengths carry.
    let bound = match (dtype.string_parameters(), dtype.bytes_parameters()) {
        (Some(leaf), _) => Some(leaf.bound().unwrap_or(0)),
        (None, Some(leaf)) => Some(leaf.bound().unwrap_or(0)),
        (None, None) => None,
    };
    // A decimal states its precision and scale; an integer or a float its
    // maximum decimal precision, as `DBSCHEMA_PROVIDER_TYPES` states it.
    let (precision, scale) = match dtype {
        DataType::Decimal32 { precision, scale }
        | DataType::Decimal64 { precision, scale }
        | DataType::Decimal128 { precision, scale }
        | DataType::Decimal256 { precision, scale } => (
            Scalar::from(u16::from(*precision)),
            Scalar::from(i16::from(*scale)),
        ),
        _ if indicator.is_unsigned().is_some() => (
            indicator
                .column_size()
                .and_then(|digits| u16::try_from(digits).ok())
                .map_or(Scalar::Null, Scalar::from),
            Scalar::Null,
        ),
        _ => (Scalar::Null, Scalar::Null),
    };
    record([
        ("TABLE_CATALOG", text(table.catalog())),
        ("TABLE_SCHEMA", table.schema().map_or(Scalar::Null, text)),
        ("TABLE_NAME", text(table.name())),
        ("COLUMN_NAME", text(column.name())),
        (
            "ORDINAL_POSITION",
            Scalar::from(u32::try_from(position + 1).unwrap_or(u32::MAX)),
        ),
        ("COLUMN_HAS_DEFAULT", Scalar::Null),
        ("COLUMN_FLAGS", Scalar::from(flags)),
        ("IS_NULLABLE", Scalar::from(column.is_nullable())),
        ("DATA_TYPE", Scalar::from(indicator.code())),
        (
            "CHARACTER_MAXIMUM_LENGTH",
            bound.map_or(Scalar::Null, Scalar::from),
        ),
        (
            "CHARACTER_OCTET_LENGTH",
            bound.map_or(Scalar::Null, Scalar::from),
        ),
        ("NUMERIC_PRECISION", precision),
        ("NUMERIC_SCALE", scale),
        ("DESCRIPTION", text(&dtype.to_string())),
    ])
}

fn text(value: &str) -> Scalar {
    Scalar::from(value)
}

/// A UTC instant from nanoseconds since the epoch, null when unknown.
fn instant(nanos: Option<i64>) -> Result<Scalar> {
    match nanos {
        Some(nanos) => Scalar::datetime64(
            nanos.div_euclid(1_000),
            crate::TimeUnit::Microsecond,
            crate::Timezone::UTC,
        ),
        None => Ok(Scalar::Null),
    }
}

fn record<const N: usize>(entries: [(&str, Scalar); N]) -> Result<Scalar> {
    Scalar::from_struct(entries)
}

/// The `MustUnderstand` fault a header block earns when it demands to be
/// understood - `mustUnderstand="1"`, SOAP-qualified or as Excel spells it -
/// and is none of the session blocks this provider processes; no detail, as
/// SOAP 1.1 has it for a fault about a header entry.
fn not_understood(request: &Request) -> Option<Fault> {
    request.header().iter().find_map(|block| {
        let element = block.element();
        let demanded = element
            .attribute_in(Some(crate::soap::ENVELOPE_NAMESPACE), "mustUnderstand")
            .or_else(|| element.attribute_in(None, "mustUnderstand"))
            .is_some_and(|value| value.trim() == "1");
        let session = matches!(
            element.local_name(),
            "BeginSession" | "Session" | "EndSession"
        ) && element.namespace().is_none_or(|namespace| namespace == super::NAMESPACE);
        (demanded && !session).then(|| {
            Fault::new(
                FaultCode::MustUnderstand,
                format!(
                    "the header block `{}` demands to be understood, and this provider processes only the session blocks",
                    element.name()
                ),
            )
            .with_actor(ACTOR)
        })
    })
}

/// A `Client` fault carrying one XMLA error.
fn client_fault(code: u32, description: impl Into<String>) -> Fault {
    let description = description.into();
    fault(FaultCode::Client, XmlaError::new(code, description.clone()))
        .unwrap_or_else(|_| Fault::client(description))
}

/// The fault `error` earns: `Server` when the provider's own storage or
/// processing failed - a store that could not be read or listed, a remote
/// that refused, an Arrow or table-format failure - and `Client` when the
/// request is what is wrong, which every other refusal says.
fn client_fault_or_server(code: u32, error: Error) -> Fault {
    match error {
        Error::Io(_) | Error::Remote { .. } | Error::Arrow(_) => server_fault(code, error),
        error => client_fault(code, error.to_string()),
    }
}

/// A `Server` fault for `error`.
fn server_fault(code: u32, error: impl std::fmt::Display) -> Fault {
    fault(FaultCode::Server, XmlaError::new(code, error.to_string()))
        .unwrap_or_else(|_| Fault::server(error.to_string()))
}
