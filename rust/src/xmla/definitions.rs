//! The rowsets this crate's provider answers, each as the field its rows are,
//! the columns a Discover may restrict it by, and the description
//! `DISCOVER_SCHEMA_ROWSETS` states for it.
//!
//! The provider is a tabular one, so the rowsets are XML for Analysis 1.1's
//! six `DISCOVER_*` rowsets and OLE DB's schema rowsets for catalogs,
//! schemas, tables, columns and datatypes. Column names and types are the
//! specification's: PascalCase for the `DISCOVER_*` rowsets, `UPPER_SNAKE`
//! for OLE DB's. What fills them is [`super::service`]'s.

use std::sync::OnceLock;

use crate::{DataType, Field, StructType, TimeUnit, Timezone};

use super::rowset::Rowset;
use super::vocabulary::RequestType;

/// One rowset the provider answers.
#[derive(Debug)]
pub struct Definition {
    request_type: RequestType,
    description: &'static str,
    rowset: Rowset,
    restrictions: Vec<usize>,
}

impl Definition {
    /// The request type this rowset answers.
    #[must_use]
    pub const fn request_type(&self) -> &RequestType {
        &self.request_type
    }

    /// What `DISCOVER_SCHEMA_ROWSETS` says of it.
    #[must_use]
    pub const fn description(&self) -> &'static str {
        self.description
    }

    /// The rowset: the field its rows are and the elements they are spelled
    /// under.
    #[must_use]
    pub const fn rowset(&self) -> &Rowset {
        &self.rowset
    }

    /// The field its rows are.
    #[must_use]
    pub fn field(&self) -> &Field {
        self.rowset.field()
    }

    /// The columns a Discover may restrict this rowset by, in column order.
    #[must_use]
    pub fn restrictions(&self) -> Vec<&Field> {
        self.restrictions
            .iter()
            .map(|index| &self.field().fields()[*index])
            .collect()
    }

    /// Whether `column` is one this rowset may be restricted by, matched
    /// case-insensitively as a client spells it.
    #[must_use]
    pub fn restricts_by(&self, column: &str) -> Option<&Field> {
        self.restrictions()
            .into_iter()
            .find(|field| field.name().eq_ignore_ascii_case(column))
    }
}

/// A column: its name, datatype, nullability, and whether it restricts.
struct Column(&'static str, DataType, bool, bool);

/// A required column.
fn required(name: &'static str, dtype: DataType) -> Column {
    Column(name, dtype, false, false)
}

/// A nullable column.
fn nullable(name: &'static str, dtype: DataType) -> Column {
    Column(name, dtype, true, false)
}

/// A column that restricts.
fn restricting(column: Column) -> Column {
    Column(column.0, column.1, column.2, true)
}

fn text() -> DataType {
    DataType::utf8()
}

fn instant() -> DataType {
    DataType::DateTime64 {
        unit: TimeUnit::Microsecond,
        timezone: Timezone::UTC,
    }
}

fn texts() -> DataType {
    DataType::serie(Field::new("item", text(), false))
}

fn definition(
    request_type: RequestType,
    description: &'static str,
    columns: Vec<Column>,
) -> Definition {
    let restrictions = columns
        .iter()
        .enumerate()
        .filter(|(_, column)| column.3)
        .map(|(index, _)| index)
        .collect();
    let fields = columns
        .into_iter()
        .map(|Column(name, dtype, nullable, _)| Field::new(name, dtype, nullable));
    let field = Field::new(
        crate::media::DEFAULT_ROOT_NAME,
        DataType::from(StructType::from_fields(fields).expect("distinct column names")),
        false,
    );
    Definition {
        request_type,
        description,
        rowset: Rowset::new(field).expect("every column here has a rowset spelling"),
        restrictions,
    }
}

/// Every rowset the provider answers, in the specification's order - the
/// order [`RequestType::ALL`] states, which is the one place it is written.
pub fn definitions() -> &'static [Definition] {
    static DEFINITIONS: OnceLock<Vec<Definition>> = OnceLock::new();
    DEFINITIONS.get_or_init(|| {
        let mut definitions = vec![
            definition(
                RequestType::DiscoverDatasources,
                "The data sources this provider serves: one per catalog root.",
                vec![
                    restricting(required("DataSourceName", text())),
                    nullable("DataSourceDescription", text()),
                    restricting(nullable("URL", text())),
                    nullable("DataSourceInfo", text()),
                    restricting(nullable("ProviderName", text())),
                    restricting(required("ProviderType", texts())),
                    restricting(required("AuthenticationMode", text())),
                ],
            ),
            definition(
                RequestType::DiscoverProperties,
                "The properties this provider reads and answers, with their current values.",
                vec![
                    restricting(required("PropertyName", text())),
                    required("PropertyDescription", text()),
                    required("PropertyType", text()),
                    required("PropertyAccessType", text()),
                    required("IsRequired", DataType::Boolean),
                    nullable("Value", text()),
                ],
            ),
            definition(
                RequestType::DiscoverSchemaRowsets,
                "The request types this provider answers, each with the columns it may be restricted by.",
                vec![
                    restricting(required("SchemaName", text())),
                    nullable("SchemaGuid", DataType::Uuid),
                    required(
                        "Restrictions",
                        DataType::serie(Field::new(
                            "item",
                            DataType::from(
                                StructType::from_fields([
                                    Field::new("Name", text(), false),
                                    Field::new("Type", text(), false),
                                ])
                                .expect("two distinct names"),
                            ),
                            false,
                        )),
                    ),
                    required("Description", text()),
                ],
            ),
            definition(
                RequestType::DiscoverEnumerators,
                "The enumerations this provider's properties and columns draw their values from.",
                vec![
                    restricting(required("EnumName", text())),
                    nullable("EnumDescription", text()),
                    required("EnumType", text()),
                    required("ElementName", text()),
                    nullable("ElementDescription", text()),
                    nullable("ElementValue", text()),
                ],
            ),
            definition(
                RequestType::DiscoverKeywords,
                "The words the statement grammar reserves.",
                vec![restricting(required("Keyword", text()))],
            ),
            definition(
                RequestType::DiscoverLiterals,
                "The literals the statement grammar spells identifiers and commands with.",
                vec![
                    restricting(required("LiteralName", text())),
                    nullable("LiteralValue", text()),
                    nullable("LiteralInvalidChars", text()),
                    nullable("LiteralInvalidStartingChars", text()),
                    nullable("LiteralMaxLength", DataType::Int32),
                    nullable("LiteralNameEnumValue", DataType::Int32),
                ],
            ),
            definition(
                RequestType::DbschemaCatalogs,
                "The catalogs this provider serves: one per root folder.",
                vec![
                    restricting(required("CATALOG_NAME", text())),
                    nullable("DESCRIPTION", text()),
                    nullable("ROLES", text()),
                    nullable("DATE_MODIFIED", instant()),
                ],
            ),
            definition(
                RequestType::DbschemaSchemata,
                "The schemas of a catalog: one per folder of tables under its root.",
                vec![
                    restricting(required("CATALOG_NAME", text())),
                    restricting(required("SCHEMA_NAME", text())),
                    restricting(nullable("SCHEMA_OWNER", text())),
                ],
            ),
            definition(
                RequestType::DbschemaTables,
                "The tables of a catalog: every leaf a record medium reads, and every folder that reads as one table.",
                vec![
                    restricting(required("TABLE_CATALOG", text())),
                    restricting(nullable("TABLE_SCHEMA", text())),
                    restricting(required("TABLE_NAME", text())),
                    restricting(required("TABLE_TYPE", text())),
                    nullable("TABLE_GUID", DataType::Uuid),
                    nullable("DESCRIPTION", text()),
                    nullable("TABLE_PROPID", DataType::UInt32),
                    nullable("DATE_CREATED", instant()),
                    nullable("DATE_MODIFIED", instant()),
                ],
            ),
            definition(
                RequestType::DbschemaColumns,
                "The columns of the tables of a catalog, typed as OLE DB types them.",
                vec![
                    restricting(required("TABLE_CATALOG", text())),
                    restricting(nullable("TABLE_SCHEMA", text())),
                    restricting(required("TABLE_NAME", text())),
                    restricting(required("COLUMN_NAME", text())),
                    required("ORDINAL_POSITION", DataType::UInt32),
                    nullable("COLUMN_HAS_DEFAULT", DataType::Boolean),
                    required("COLUMN_FLAGS", DataType::UInt32),
                    required("IS_NULLABLE", DataType::Boolean),
                    required("DATA_TYPE", DataType::UInt16),
                    nullable("CHARACTER_MAXIMUM_LENGTH", DataType::UInt32),
                    nullable("CHARACTER_OCTET_LENGTH", DataType::UInt32),
                    nullable("NUMERIC_PRECISION", DataType::UInt16),
                    nullable("NUMERIC_SCALE", DataType::Int16),
                    nullable("DESCRIPTION", text()),
                ],
            ),
            definition(
                RequestType::DbschemaProviderTypes,
                "The datatypes this provider's columns take, as OLE DB types them.",
                vec![
                    required("TYPE_NAME", text()),
                    restricting(required("DATA_TYPE", DataType::UInt16)),
                    required("COLUMN_SIZE", DataType::UInt32),
                    nullable("LITERAL_PREFIX", text()),
                    nullable("LITERAL_SUFFIX", text()),
                    nullable("CREATE_PARAMS", text()),
                    nullable("IS_NULLABLE", DataType::Boolean),
                    nullable("CASE_SENSITIVE", DataType::Boolean),
                    nullable("SEARCHABLE", DataType::UInt32),
                    nullable("UNSIGNED_ATTRIBUTE", DataType::Boolean),
                    nullable("FIXED_PREC_SCALE", DataType::Boolean),
                    nullable("AUTO_UNIQUE_VALUE", DataType::Boolean),
                    nullable("LOCAL_TYPE_NAME", text()),
                    nullable("MINIMUM_SCALE", DataType::Int16),
                    nullable("MAXIMUM_SCALE", DataType::Int16),
                    nullable("IS_LONG", DataType::Boolean),
                    restricting(nullable("BEST_MATCH", DataType::Boolean)),
                    nullable("IS_FIXEDLENGTH", DataType::Boolean),
                ],
            ),
        ];
        definitions.sort_by_key(|definition| {
            RequestType::ALL
                .iter()
                .position(|request_type| request_type == definition.request_type())
        });
        definitions
    })
}

/// The rowset `request_type` names, `None` for one the provider does not
/// answer.
#[must_use]
pub fn definition_of(request_type: &RequestType) -> Option<&'static Definition> {
    definitions()
        .iter()
        .find(|definition| definition.request_type == *request_type)
}
