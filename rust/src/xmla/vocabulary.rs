//! The words XML for Analysis 1.1 defines: the request types a Discover
//! answers, the properties a PropertyList carries and the enumerations their
//! values come from, and the restrictions a Discover narrows by.
//!
//! Every enumeration here is the specification's, spelled as the
//! specification spells it, read case-insensitively and written canonically.
//! A name the specification does not define is kept as it was spelled rather
//! than refused: which request types a *server* answers is the server's to
//! say, and it says so in `DISCOVER_SCHEMA_ROWSETS`.

use std::fmt;
use std::str::FromStr;

use smol_str::{SmolStr, format_smolstr};

use crate::{Error, Result};

/// The two methods XML for Analysis defines.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Method {
    /// Metadata: a request type, its restrictions, and the rowset it answers.
    Discover,
    /// A command - a statement - and the rowset or dataset it answers.
    Execute,
}

impl Method {
    /// Both methods, in the order the specification lists them.
    pub const ALL: [Self; 2] = [Self::Discover, Self::Execute];

    /// The element name the method is invoked as.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Discover => "Discover",
            Self::Execute => "Execute",
        }
    }

    /// The element name the method's answer is written under.
    #[must_use]
    pub const fn response_name(self) -> &'static str {
        match self {
            Self::Discover => "DiscoverResponse",
            Self::Execute => "ExecuteResponse",
        }
    }

    /// The `SOAPAction` header a request of this method carries.
    #[must_use]
    pub const fn soap_action(self) -> &'static str {
        match self {
            Self::Discover => "urn:schemas-microsoft-com:xml-analysis:Discover",
            Self::Execute => "urn:schemas-microsoft-com:xml-analysis:Execute",
        }
    }
}

impl FromStr for Method {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        let value = value.trim();
        Self::ALL
            .into_iter()
            .find(|method| method.as_str().eq_ignore_ascii_case(value))
            .ok_or_else(|| unknown("method", value, "Discover or Execute"))
    }
}

impl fmt::Display for Method {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// One enumeration of the specification: a closed set of spellings, each
/// member with a description, read case-insensitively.
macro_rules! enumeration {
    (
        $(#[$doc:meta])*
        $name:ident, $what:literal, [$(($variant:ident, $spelled:literal, $description:literal)),+ $(,)?]
    ) => {
        $(#[$doc])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum $name {
            $(#[doc = $description] $variant,)+
        }

        impl $name {
            /// Every member, in the specification's order.
            pub const ALL: &'static [Self] = &[$(Self::$variant,)+];

            /// The spelling the specification gives this member.
            #[must_use]
            pub const fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $spelled,)+
                }
            }

            /// What the specification says of this member.
            #[must_use]
            pub const fn description(self) -> &'static str {
                match self {
                    $(Self::$variant => $description,)+
                }
            }
        }

        impl FromStr for $name {
            type Err = Error;

            fn from_str(value: &str) -> Result<Self> {
                let value = value.trim();
                Self::ALL
                    .iter()
                    .copied()
                    .find(|member| member.as_str().eq_ignore_ascii_case(value))
                    .ok_or_else(|| {
                        unknown(
                            $what,
                            value,
                            &Self::ALL
                                .iter()
                                .map(|member| member.as_str())
                                .collect::<Vec<_>>()
                                .join(", "),
                        )
                    })
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(self.as_str())
            }
        }
    };
}

enumeration!(
    /// The `Format` property: the shape a result takes.
    Format,
    "Format",
    [
        (Tabular, "Tabular", "A flat or hierarchical rowset, the one shape a Discover answers."),
        (Multidimensional, "Multidimensional", "An MDDataSet, the shape a multidimensional Execute answers."),
        (Native, "Native", "Whatever shape suits the request; the result's namespace says which."),
    ]
);

impl Format {
    /// The default, as the specification declares it.
    pub const DEFAULT: Self = Self::Native;
}

enumeration!(
    /// The `Content` property: which of the schema and the data a result
    /// carries.
    Content,
    "Content",
    [
        (None, "None", "Verify the command, execute nothing, answer no rows."),
        (Schema, "Schema", "The XML Schema describing the result's columns, and no rows."),
        (Data, "Data", "The rows, and no schema."),
        (SchemaData, "SchemaData", "The schema, then the rows: the default."),
    ]
);

impl Content {
    /// The default, as the specification declares it.
    pub const DEFAULT: Self = Self::SchemaData;

    /// Whether a result of this content carries its schema.
    #[must_use]
    pub const fn has_schema(self) -> bool {
        matches!(self, Self::Schema | Self::SchemaData)
    }

    /// Whether a result of this content carries its rows.
    #[must_use]
    pub const fn has_data(self) -> bool {
        matches!(self, Self::Data | Self::SchemaData)
    }
}

enumeration!(
    /// The `AxisFormat` property: how an MDDataSet spells its axes.
    AxisFormat,
    "AxisFormat",
    [
        (TupleFormat, "TupleFormat", "Each axis as its tuples: the default."),
        (ClusterFormat, "ClusterFormat", "Each axis as clusters of members."),
        (CustomFormat, "CustomFormat", "A layout the provider defines."),
    ]
);

enumeration!(
    /// What a data source provides, as `DISCOVER_DATASOURCES` states it.
    ProviderType,
    "ProviderType",
    [
        (Tdp, "TDP", "A tabular data provider: rows answered to a text command."),
        (Mdp, "MDP", "A multidimensional data provider: cubes answered to MDX."),
        (Dmp, "DMP", "A data mining provider, per OLE DB for Data Mining."),
    ]
);

enumeration!(
    /// How a data source authenticates, as `DISCOVER_DATASOURCES` states it.
    AuthenticationMode,
    "AuthenticationMode",
    [
        (Unauthenticated, "Unauthenticated", "No user name or password is sent."),
        (Authenticated, "Authenticated", "A user name and password are sent in the properties."),
        (Integrated, "Integrated", "The transport authenticates: HTTP or the operating system."),
    ]
);

enumeration!(
    /// How a property may be used, as `DISCOVER_PROPERTIES` states it.
    Access,
    "PropertyAccessType",
    [
        (Read, "Read", "The provider answers the property; a client cannot set it."),
        (Write, "Write", "A client sets the property; the provider does not answer it."),
        (ReadWrite, "ReadWrite", "Set by the client and answered by the provider."),
    ]
);

enumeration!(
    /// The `StateSupport` property: whether sessions are kept.
    StateSupport,
    "StateSupport",
    [
        (None, "None", "No sessions: every request stands alone."),
        (Sessions, "Sessions", "BeginSession, Session and EndSession headers are honoured."),
    ]
);

enumeration!(
    /// The `MDXSupport` property: how much MDX a provider speaks.
    MdxSupport,
    "MDXSupport",
    [(Core, "Core", "The core MDX grammar, the one value the specification defines."),]
);

/// The request types XML for Analysis 1.1 defines, and any other a provider
/// answers.
///
/// The specification's own `DISCOVER_*` rowsets describe the provider; the
/// `DBSCHEMA_*` rowsets are OLE DB's schema rowsets, what a tabular provider
/// answers; the `MDSCHEMA_*` rowsets are OLE DB for OLAP's, what a
/// multidimensional provider answers.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RequestType {
    /// The data sources the provider serves.
    DiscoverDatasources,
    /// The properties the provider knows, and their current values.
    DiscoverProperties,
    /// The request types the provider answers, and each one's restrictions.
    DiscoverSchemaRowsets,
    /// The enumerations the provider's properties and columns draw on.
    DiscoverEnumerators,
    /// The words the provider's command language reserves.
    DiscoverKeywords,
    /// The literals the provider's command language spells identifiers with.
    DiscoverLiterals,
    /// The catalogs of a data source.
    DbschemaCatalogs,
    /// The columns of the tables of a catalog.
    DbschemaColumns,
    /// The datatypes the provider's columns take.
    DbschemaProviderTypes,
    /// The schemas of a catalog.
    DbschemaSchemata,
    /// The tables of a catalog.
    DbschemaTables,
    /// Bookmark and cardinality facts about the tables of a catalog.
    DbschemaTablesInfo,
    /// The actions defined on a cube.
    MdschemaActions,
    /// The cubes of a catalog.
    MdschemaCubes,
    /// The dimensions of a cube.
    MdschemaDimensions,
    /// The functions the MDX grammar offers.
    MdschemaFunctions,
    /// The hierarchies of a dimension.
    MdschemaHierarchies,
    /// The levels of a hierarchy.
    MdschemaLevels,
    /// The measures of a cube.
    MdschemaMeasures,
    /// The members of a level.
    MdschemaMembers,
    /// The properties of members.
    MdschemaProperties,
    /// The named sets of a cube.
    MdschemaSets,
    /// The KPIs of a cube.
    MdschemaKpis,
    /// The measure groups of a cube.
    MdschemaMeasuregroups,
    /// The dimensions of a measure group.
    MdschemaMeasuregroupDimensions,
    /// The input data sources of a cube.
    MdschemaInputDatasources,
    /// A request type the specification does not define, kept as spelled.
    Other(SmolStr),
}

impl RequestType {
    /// Every request type the specification defines, in its order.
    pub const ALL: &'static [Self] = &[
        Self::DiscoverDatasources,
        Self::DiscoverProperties,
        Self::DiscoverSchemaRowsets,
        Self::DiscoverEnumerators,
        Self::DiscoverKeywords,
        Self::DiscoverLiterals,
        Self::DbschemaCatalogs,
        Self::DbschemaColumns,
        Self::DbschemaProviderTypes,
        Self::DbschemaSchemata,
        Self::DbschemaTables,
        Self::DbschemaTablesInfo,
        Self::MdschemaActions,
        Self::MdschemaCubes,
        Self::MdschemaDimensions,
        Self::MdschemaFunctions,
        Self::MdschemaHierarchies,
        Self::MdschemaLevels,
        Self::MdschemaMeasures,
        Self::MdschemaMembers,
        Self::MdschemaProperties,
        Self::MdschemaSets,
        Self::MdschemaKpis,
        Self::MdschemaMeasuregroups,
        Self::MdschemaMeasuregroupDimensions,
        Self::MdschemaInputDatasources,
    ];

    /// The spelling the specification gives this request type, or the one it
    /// arrived with.
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::DiscoverDatasources => "DISCOVER_DATASOURCES",
            Self::DiscoverProperties => "DISCOVER_PROPERTIES",
            Self::DiscoverSchemaRowsets => "DISCOVER_SCHEMA_ROWSETS",
            Self::DiscoverEnumerators => "DISCOVER_ENUMERATORS",
            Self::DiscoverKeywords => "DISCOVER_KEYWORDS",
            Self::DiscoverLiterals => "DISCOVER_LITERALS",
            Self::DbschemaCatalogs => "DBSCHEMA_CATALOGS",
            Self::DbschemaColumns => "DBSCHEMA_COLUMNS",
            Self::DbschemaProviderTypes => "DBSCHEMA_PROVIDER_TYPES",
            Self::DbschemaSchemata => "DBSCHEMA_SCHEMATA",
            Self::DbschemaTables => "DBSCHEMA_TABLES",
            Self::DbschemaTablesInfo => "DBSCHEMA_TABLES_INFO",
            Self::MdschemaActions => "MDSCHEMA_ACTIONS",
            Self::MdschemaCubes => "MDSCHEMA_CUBES",
            Self::MdschemaDimensions => "MDSCHEMA_DIMENSIONS",
            Self::MdschemaFunctions => "MDSCHEMA_FUNCTIONS",
            Self::MdschemaHierarchies => "MDSCHEMA_HIERARCHIES",
            Self::MdschemaLevels => "MDSCHEMA_LEVELS",
            Self::MdschemaMeasures => "MDSCHEMA_MEASURES",
            Self::MdschemaMembers => "MDSCHEMA_MEMBERS",
            Self::MdschemaProperties => "MDSCHEMA_PROPERTIES",
            Self::MdschemaSets => "MDSCHEMA_SETS",
            Self::MdschemaKpis => "MDSCHEMA_KPIS",
            Self::MdschemaMeasuregroups => "MDSCHEMA_MEASUREGROUPS",
            Self::MdschemaMeasuregroupDimensions => "MDSCHEMA_MEASUREGROUP_DIMENSIONS",
            Self::MdschemaInputDatasources => "MDSCHEMA_INPUT_DATASOURCES",
            Self::Other(spelled) => spelled.as_str(),
        }
    }

    /// Whether this is one of the `DISCOVER_*` rowsets that describe the
    /// provider itself.
    #[must_use]
    pub fn is_discover(&self) -> bool {
        family(self.as_str(), "DISCOVER_")
    }

    /// Whether this is one of OLE DB's `DBSCHEMA_*` rowsets.
    #[must_use]
    pub fn is_dbschema(&self) -> bool {
        family(self.as_str(), "DBSCHEMA_")
    }

    /// Whether this is one of OLE DB for OLAP's `MDSCHEMA_*` rowsets.
    #[must_use]
    pub fn is_mdschema(&self) -> bool {
        family(self.as_str(), "MDSCHEMA_")
    }
}

/// Whether `name` opens with `prefix` in any case: a defined name is
/// canonical, and an undefined one is kept as it was spelled, so its family
/// is read the way its name was.
fn family(name: &str, prefix: &str) -> bool {
    name.get(..prefix.len())
        .is_some_and(|opening| opening.eq_ignore_ascii_case(prefix))
}

impl FromStr for RequestType {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        let value = value.trim();
        if value.is_empty() {
            return Err(unknown("RequestType", value, "a rowset name"));
        }
        Ok(Self::ALL
            .iter()
            .find(|known| known.as_str().eq_ignore_ascii_case(value))
            .cloned()
            .unwrap_or_else(|| Self::Other(SmolStr::new(value))))
    }
}

impl fmt::Display for RequestType {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// The names of the standard properties, as a PropertyList spells them.
pub mod property {
    /// How an MDDataSet spells its axes.
    pub const AXIS_FORMAT: &str = "AxisFormat";
    /// The first cell ordinal an MDDataSet answers.
    pub const BEGIN_RANGE: &str = "BeginRange";
    /// The catalog a request addresses.
    pub const CATALOG: &str = "Catalog";
    /// Which of the schema and the data a result carries.
    pub const CONTENT: &str = "Content";
    /// The cube a command runs against.
    pub const CUBE: &str = "Cube";
    /// The provider-specific connection text.
    pub const DATA_SOURCE_INFO: &str = "DataSourceInfo";
    /// The version of the engine behind the provider.
    pub const DBMS_VERSION: &str = "DBMSVersion";
    /// The last cell ordinal an MDDataSet answers.
    pub const END_RANGE: &str = "EndRange";
    /// The shape a result takes.
    pub const FORMAT: &str = "Format";
    /// The numeric locale of the request.
    pub const LOCALE_IDENTIFIER: &str = "LocaleIdentifier";
    /// How much MDX the provider speaks.
    pub const MDX_SUPPORT: &str = "MDXSupport";
    /// A password; deprecated in XMLA 1.1 and ignored.
    pub const PASSWORD: &str = "Password";
    /// The provider's name.
    pub const PROVIDER_NAME: &str = "ProviderName";
    /// The provider's version.
    pub const PROVIDER_VERSION: &str = "ProviderVersion";
    /// Whether sessions are kept.
    pub const STATE_SUPPORT: &str = "StateSupport";
    /// Seconds to wait for a request to succeed.
    pub const TIMEOUT: &str = "Timeout";
    /// The user name; deprecated as writable in XMLA 1.1.
    pub const USER_NAME: &str = "UserName";
    /// How visual totals behave.
    pub const VISUAL_MODE: &str = "VisualMode";
}

/// The `PropertyList` of a request: each property once, by name, in the order
/// it was written.
///
/// Names are the specification's, matched case-insensitively on the way in
/// and kept as written; a property the specification does not define
/// travels too, because a provider may define its own.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PropertyList {
    entries: Vec<(SmolStr, String)>,
}

impl PropertyList {
    /// No property.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// The value of `name`, matched case-insensitively.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|(held, _)| held.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    /// Set `name` to `value`, replacing the value it held.
    pub fn set(&mut self, name: impl Into<SmolStr>, value: impl Into<String>) {
        let name = name.into();
        let value = value.into();
        match self
            .entries
            .iter_mut()
            .find(|(held, _)| held.eq_ignore_ascii_case(&name))
        {
            Some(entry) => entry.1 = value,
            None => self.entries.push((name, value)),
        }
    }

    /// Return this list with `name` set to `value`.
    #[must_use]
    pub fn with(mut self, name: impl Into<SmolStr>, value: impl Into<String>) -> Self {
        self.set(name, value);
        self
    }

    /// Remove `name`, answering the value it held.
    pub fn remove(&mut self, name: &str) -> Option<String> {
        let index = self
            .entries
            .iter()
            .position(|(held, _)| held.eq_ignore_ascii_case(name))?;
        Some(self.entries.remove(index).1)
    }

    /// Every property, in the order written.
    #[must_use]
    pub fn entries(&self) -> &[(SmolStr, String)] {
        &self.entries
    }

    /// Whether no property is set.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// How many properties are set.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// The `Format` property, or its default when unset.
    ///
    /// # Errors
    ///
    /// Returns an error naming the value when it is not a `Format`.
    pub fn format(&self) -> Result<Format> {
        self.get(property::FORMAT)
            .map_or(Ok(Format::DEFAULT), str::parse)
    }

    /// The `Content` property, or its default when unset.
    ///
    /// # Errors
    ///
    /// Returns an error naming the value when it is not a `Content`.
    pub fn content(&self) -> Result<Content> {
        self.get(property::CONTENT)
            .map_or(Ok(Content::DEFAULT), str::parse)
    }

    /// The `AxisFormat` property, or its default when unset.
    ///
    /// # Errors
    ///
    /// Returns an error naming the value when it is not an `AxisFormat`.
    pub fn axis_format(&self) -> Result<AxisFormat> {
        self.get(property::AXIS_FORMAT)
            .map_or(Ok(AxisFormat::TupleFormat), str::parse)
    }

    /// The `Catalog` property, `None` when unset or empty.
    #[must_use]
    pub fn catalog(&self) -> Option<&str> {
        self.get(property::CATALOG).filter(|value| !value.is_empty())
    }

    /// The `DataSourceInfo` property, `None` when unset or empty.
    #[must_use]
    pub fn data_source_info(&self) -> Option<&str> {
        self.get(property::DATA_SOURCE_INFO)
            .filter(|value| !value.is_empty())
    }

    /// The `Timeout` property in seconds, `None` when unset.
    ///
    /// # Errors
    ///
    /// Returns an error naming the value when it is not a count of seconds.
    pub fn timeout(&self) -> Result<Option<u32>> {
        self.get(property::TIMEOUT)
            .filter(|value| !value.is_empty())
            .map(|value| {
                value
                    .trim()
                    .parse::<u32>()
                    .map_err(|_| unknown("Timeout", value, "a count of seconds"))
            })
            .transpose()
    }
}

impl<K: Into<SmolStr>, V: Into<String>> FromIterator<(K, V)> for PropertyList {
    fn from_iter<I: IntoIterator<Item = (K, V)>>(entries: I) -> Self {
        let mut list = Self::new();
        for (name, value) in entries {
            list.set(name, value);
        }
        list
    }
}

/// The `RestrictionList` of a Discover: each restriction column with the
/// values it admits, in the order written.
///
/// A restriction written once admits one value; one written several times
/// admits any of them, which is how a client asks for several rows of one
/// rowset in one request.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Restrictions {
    entries: Vec<(SmolStr, Vec<String>)>,
}

impl Restrictions {
    /// No restriction.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// The values `name` admits, matched case-insensitively; `None` when the
    /// column is not restricted.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&[String]> {
        self.entries
            .iter()
            .find(|(held, _)| held.eq_ignore_ascii_case(name))
            .map(|(_, values)| values.as_slice())
    }

    /// Admit one more value for `name`.
    pub fn push(&mut self, name: impl Into<SmolStr>, value: impl Into<String>) {
        let name = name.into();
        let value = value.into();
        match self
            .entries
            .iter_mut()
            .find(|(held, _)| held.eq_ignore_ascii_case(&name))
        {
            Some(entry) => entry.1.push(value),
            None => self.entries.push((name, vec![value])),
        }
    }

    /// Return these restrictions admitting `value` for `name`.
    #[must_use]
    pub fn with(mut self, name: impl Into<SmolStr>, value: impl Into<String>) -> Self {
        self.push(name, value);
        self
    }

    /// Every restriction, in the order written.
    #[must_use]
    pub fn entries(&self) -> &[(SmolStr, Vec<String>)] {
        &self.entries
    }

    /// Whether nothing is restricted.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// How many columns are restricted.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

impl<K: Into<SmolStr>, V: Into<String>> FromIterator<(K, V)> for Restrictions {
    fn from_iter<I: IntoIterator<Item = (K, V)>>(entries: I) -> Self {
        let mut restrictions = Self::new();
        for (name, value) in entries {
            restrictions.push(name, value);
        }
        restrictions
    }
}

/// The refusal every enumeration here raises for a spelling it does not have.
fn unknown(what: &str, value: &str, expected: &str) -> Error {
    Error::InvalidRecord {
        path: format_smolstr!("$.{what}"),
        reason: format_smolstr!(
            "expected {expected}, got {:?}",
            crate::text::elide_to(value, crate::text::ERROR_TEXT_LIMIT)
        ),
    }
}
