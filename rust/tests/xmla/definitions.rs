//! `rust/src/xmla/definitions.rs`: the rowsets the provider answers - one
//! definition per request type, each the record field its rows are, the
//! columns a Discover may restrict it by, and the description
//! `DISCOVER_SCHEMA_ROWSETS` states for it.

use std::path::PathBuf;
use std::sync::Arc;

use yggdryl::holder::Holder;
use yggdryl::xml::{Element, XSI_NAMESPACE};
use yggdryl::xmla::definitions::{Definition, definition_of, definitions};
use yggdryl::xmla::{
    Catalog, Discover, ROWSET_NAMESPACE, Request, RequestType, Response, Rowset, SQL_NAMESPACE,
    Service, ServiceOptions, XsdType, decode_name, encode_name,
};
use yggdryl::{DataType, Field, Scalar, Serie, StructType, TimeUnit, Timezone, Uuid};

/// One column as the specification declares it: its name, datatype and
/// nullability, and whether a Discover may restrict the rowset by it.
struct Column {
    name: &'static str,
    dtype: DataType,
    nullable: bool,
    restricts: bool,
}

/// A column a row always states.
fn required(name: &'static str, dtype: DataType) -> Column {
    Column {
        name,
        dtype,
        nullable: false,
        restricts: false,
    }
}

/// A column a row may leave absent.
fn nullable(name: &'static str, dtype: DataType) -> Column {
    Column {
        name,
        dtype,
        nullable: true,
        restricts: false,
    }
}

/// The same column, one a Discover may restrict by.
fn restricting(column: Column) -> Column {
    Column {
        restricts: true,
        ..column
    }
}

fn text() -> DataType {
    DataType::utf8()
}

/// A UTC instant at microseconds: what `xsd:dateTime` holds.
fn instant() -> DataType {
    DataType::DateTime64 {
        unit: TimeUnit::Microsecond,
        timezone: Timezone::UTC,
    }
}

/// A sequence of required text items.
fn texts() -> DataType {
    DataType::serie(Field::new("item", text(), false))
}

/// The record field `columns` make together: a required struct named `row`.
fn record(columns: &[Column]) -> Field {
    Field::new(
        "row",
        DataType::from(
            StructType::from_fields(
                columns
                    .iter()
                    .map(|column| Field::new(column.name, column.dtype.clone(), column.nullable)),
            )
            .expect("distinct column names"),
        ),
        false,
    )
}

/// The definition `request_type` names, which this provider answers.
fn answered(request_type: &RequestType) -> &'static Definition {
    definition_of(request_type).unwrap_or_else(|| panic!("{request_type} is answered"))
}

/// The column names of `field`, in order.
fn names(field: &Field) -> Vec<&str> {
    field.fields().iter().map(Field::name).collect()
}

/// The restriction column names of `definition`, in order.
fn restriction_names(definition: &Definition) -> Vec<&str> {
    definition
        .restrictions()
        .into_iter()
        .map(Field::name)
        .collect()
}

/// Pins one definition against the columns the specification gives it: the
/// names in order, each datatype and nullability, the restriction columns,
/// and the record field they make together.
fn pin(request_type: &RequestType, columns: &[Column]) {
    let definition = answered(request_type);
    assert_eq!(definition.request_type(), request_type);
    let field = definition.field();
    assert_eq!(
        names(field),
        columns.iter().map(|column| column.name).collect::<Vec<_>>(),
        "{request_type} names its columns in the specification's order"
    );
    for (held, column) in field.fields().iter().zip(columns) {
        assert_eq!(
            held.dtype(),
            &column.dtype,
            "{request_type}.{} is typed as the specification types it",
            column.name
        );
        assert_eq!(
            held.is_nullable(),
            column.nullable,
            "{request_type}.{} is {} as the specification declares it",
            column.name,
            if column.nullable {
                "nullable"
            } else {
                "required"
            }
        );
    }
    assert_eq!(
        restriction_names(definition),
        columns
            .iter()
            .filter(|column| column.restricts)
            .map(|column| column.name)
            .collect::<Vec<_>>(),
        "{request_type} restricts by the specification's restriction columns"
    );
    assert_eq!(field, &record(columns), "{request_type}'s record field");
}

/// The request types this provider answers, in `RequestType::ALL`'s order.
fn covered() -> Vec<RequestType> {
    vec![
        RequestType::DiscoverDatasources,
        RequestType::DiscoverProperties,
        RequestType::DiscoverSchemaRowsets,
        RequestType::DiscoverEnumerators,
        RequestType::DiscoverKeywords,
        RequestType::DiscoverLiterals,
        RequestType::DbschemaCatalogs,
        RequestType::DbschemaColumns,
        RequestType::DbschemaProviderTypes,
        RequestType::DbschemaSchemata,
        RequestType::DbschemaTables,
    ]
}

// Refusals: what the provider does not answer, and what does not restrict.

#[test]
fn a_request_type_the_provider_does_not_answer_has_no_definition() {
    let covered = covered();
    let uncovered: Vec<&RequestType> = RequestType::ALL
        .iter()
        .filter(|request_type| !covered.contains(request_type))
        .collect();
    assert!(uncovered.contains(&&RequestType::DbschemaTablesInfo));
    assert!(uncovered.contains(&&RequestType::MdschemaCubes));
    for request_type in uncovered {
        assert!(
            definition_of(request_type).is_none(),
            "{request_type} is not answered by a tabular provider"
        );
    }
    for spelled in ["DISCOVER_XML_METADATA", "discover_datasources_", "Ünknown"] {
        let other = RequestType::Other(spelled.into());
        assert!(
            definition_of(&other).is_none(),
            "{spelled} is kept as spelled and answers no rowset"
        );
    }
}

#[test]
fn a_request_type_kept_as_spelled_never_matches_a_defined_one() {
    // `Other` holding a defined spelling is still `Other`: the definition is
    // found by the variant a request type resolved to, not by its text.
    let spelled = RequestType::Other("DBSCHEMA_TABLES".into());
    assert_eq!(spelled.as_str(), "DBSCHEMA_TABLES");
    assert!(definition_of(&spelled).is_none());
}

#[test]
fn a_column_the_rowset_does_not_have_restricts_nothing() {
    let tables = answered(&RequestType::DbschemaTables);
    for column in [
        "NO_SUCH_COLUMN",
        "",
        "TABLE",
        "TABLE_NAME_",
        "_TABLE_NAME",
        "TABLE NAME",
        "CATALOG_NAME",
    ] {
        assert!(
            tables.restricts_by(column).is_none(),
            "`{column}` is not a restriction of DBSCHEMA_TABLES"
        );
    }
}

#[test]
fn a_column_that_does_not_restrict_restricts_nothing() {
    for definition in definitions() {
        let restrictions = restriction_names(definition);
        for column in definition.field().fields() {
            if restrictions.contains(&column.name()) {
                continue;
            }
            assert!(
                definition.restricts_by(column.name()).is_none(),
                "{}.{} is a column but not a restriction",
                definition.request_type(),
                column.name()
            );
            assert!(
                definition
                    .restricts_by(&column.name().to_ascii_lowercase())
                    .is_none(),
                "{}.{} does not restrict in any case",
                definition.request_type(),
                column.name()
            );
        }
    }
    let tables = answered(&RequestType::DbschemaTables);
    for column in [
        "TABLE_GUID",
        "DESCRIPTION",
        "TABLE_PROPID",
        "DATE_CREATED",
        "DATE_MODIFIED",
    ] {
        assert!(tables.restricts_by(column).is_none(), "{column}");
    }
    let types = answered(&RequestType::DbschemaProviderTypes);
    assert!(types.restricts_by("TYPE_NAME").is_none());
    assert!(types.restricts_by("IS_FIXEDLENGTH").is_none());
}

#[test]
fn a_padded_or_unicode_folded_name_is_not_the_column() {
    let keywords = answered(&RequestType::DiscoverKeywords);
    assert!(keywords.restricts_by("Keyword").is_some());
    // The match folds ASCII case only, as a client spells an XML name, and
    // takes the name as it is: no padding is trimmed and no Unicode fold
    // (KELVIN SIGN folds to `k`) is applied.
    for column in [" Keyword", "Keyword ", "\tKEYWORD", "\u{212A}eyword"] {
        assert!(
            keywords.restricts_by(column).is_none(),
            "{column:?} is not the Keyword column"
        );
    }
}

// The set of definitions.

#[test]
fn definitions_cover_each_request_type_the_provider_answers_once() {
    let all = definitions();
    assert!(!all.is_empty());
    let mut answered: Vec<RequestType> = all
        .iter()
        .map(|definition| definition.request_type().clone())
        .collect();
    let count = answered.len();
    answered.sort();
    answered.dedup();
    assert_eq!(answered.len(), count, "no request type is answered twice");
    let mut expected = covered();
    expected.sort();
    assert_eq!(answered, expected);
}

#[test]
fn definitions_follow_the_specifications_order() {
    // `definitions()` promises "the specification's order", and the crate
    // states that order once, in `RequestType::ALL`.
    let answered: Vec<&RequestType> = definitions().iter().map(Definition::request_type).collect();
    let specified: Vec<&RequestType> = RequestType::ALL
        .iter()
        .filter(|request_type| answered.contains(request_type))
        .collect();
    assert_eq!(
        answered, specified,
        "the definitions are listed in the order RequestType::ALL states as the specification's"
    );
}

#[test]
fn definitions_open_with_the_discover_rowsets_then_the_ole_db_ones() {
    let all = definitions();
    let discover = all
        .iter()
        .take_while(|definition| definition.request_type().is_discover())
        .count();
    assert_eq!(discover, 6, "the six DISCOVER_* rowsets come first");
    assert!(
        all[discover..]
            .iter()
            .all(|definition| definition.request_type().is_dbschema()),
        "every rowset past them is an OLE DB schema rowset"
    );
    assert!(
        all.iter()
            .all(|definition| !definition.request_type().is_mdschema()),
        "a tabular provider answers no MDSCHEMA_* rowset"
    );
}

#[test]
fn definitions_are_built_once_and_shared() {
    let first = definitions();
    assert!(std::ptr::eq(first, definitions()));
    let addresses: Vec<usize> = (0..4)
        .map(|_| std::thread::spawn(|| definitions().as_ptr() as usize))
        .collect::<Vec<_>>()
        .into_iter()
        .map(|handle| handle.join().expect("the thread finishes"))
        .collect();
    for address in addresses {
        assert_eq!(address, first.as_ptr() as usize);
    }
}

#[test]
fn definition_of_answers_each_definition_itself() {
    for definition in definitions() {
        let found = definition_of(definition.request_type())
            .unwrap_or_else(|| panic!("{} is answered", definition.request_type()));
        assert!(
            std::ptr::eq(found, definition),
            "{} finds its own definition",
            definition.request_type()
        );
    }
}

#[test]
fn a_request_type_read_in_any_case_finds_its_definition() {
    for spelled in ["dbschema_tables", "DbSchema_Tables", " DBSCHEMA_TABLES "] {
        let request_type: RequestType = spelled.parse().expect("a request type");
        let definition = definition_of(&request_type).expect("DBSCHEMA_TABLES is answered");
        assert_eq!(definition.request_type(), &RequestType::DbschemaTables);
    }
}

#[test]
fn every_definition_states_a_distinct_one_sentence_description() {
    let mut descriptions: Vec<&str> = Vec::new();
    for definition in definitions() {
        let description = definition.description();
        let request_type = definition.request_type();
        assert!(!description.is_empty(), "{request_type} is described");
        assert_eq!(
            description.trim(),
            description,
            "{request_type}'s description carries no padding"
        );
        assert!(
            description.starts_with(char::is_uppercase) && description.ends_with('.'),
            "{request_type}'s description is a sentence: {description:?}"
        );
        assert!(
            !description.contains('\n') && !description[..description.len() - 1].contains(". "),
            "{request_type}'s description is one sentence: {description:?}"
        );
        assert_ne!(
            description,
            request_type.as_str(),
            "{request_type}'s description says more than its name"
        );
        assert!(
            !descriptions.contains(&description),
            "{request_type}'s description is its own"
        );
        descriptions.push(description);
    }
}

// The record field each definition's rows are.

#[test]
fn every_rowset_is_a_required_record_named_row() {
    for definition in definitions() {
        let request_type = definition.request_type();
        let field = definition.field();
        assert!(
            std::ptr::eq(field, definition.rowset().field()),
            "{request_type}'s field is its rowset's"
        );
        assert!(std::ptr::eq(
            field,
            definition.rowset().shared_field().as_ref()
        ));
        assert_eq!(field.name(), "row");
        assert_eq!(field.name(), yggdryl::media::DEFAULT_ROOT_NAME);
        assert!(!field.is_nullable(), "{request_type}'s rows are required");
        assert!(
            matches!(field.dtype(), DataType::Struct(_)),
            "{request_type}'s rows are a struct"
        );
        assert!(field.is_struct());
        assert!(!field.fields().is_empty(), "{request_type} has columns");
        assert_eq!(
            &Rowset::new(field.clone()).expect("a rowset"),
            definition.rowset(),
            "{request_type}'s rowset is the one its field makes, nothing added"
        );
    }
}

#[test]
fn discover_datasources_declares_its_columns() {
    pin(
        &RequestType::DiscoverDatasources,
        &[
            restricting(required("DataSourceName", text())),
            nullable("DataSourceDescription", text()),
            restricting(nullable("URL", text())),
            nullable("DataSourceInfo", text()),
            restricting(nullable("ProviderName", text())),
            restricting(required("ProviderType", texts())),
            restricting(required("AuthenticationMode", text())),
        ],
    );
}

#[test]
fn discover_properties_declares_its_columns() {
    pin(
        &RequestType::DiscoverProperties,
        &[
            restricting(required("PropertyName", text())),
            required("PropertyDescription", text()),
            required("PropertyType", text()),
            required("PropertyAccessType", text()),
            required("IsRequired", DataType::Boolean),
            nullable("Value", text()),
        ],
    );
}

#[test]
fn discover_schema_rowsets_declares_its_columns() {
    let restriction = DataType::from(
        StructType::from_fields([
            Field::new("Name", text(), false),
            Field::new("Type", text(), false),
        ])
        .expect("two distinct names"),
    );
    pin(
        &RequestType::DiscoverSchemaRowsets,
        &[
            restricting(required("SchemaName", text())),
            nullable("SchemaGuid", DataType::Uuid),
            required(
                "Restrictions",
                DataType::serie(Field::new("item", restriction, false)),
            ),
            required("Description", text()),
        ],
    );
}

#[test]
fn discover_schema_rowsets_restrictions_are_a_sequence_of_name_and_type() {
    let definition = answered(&RequestType::DiscoverSchemaRowsets);
    let restrictions = definition
        .field()
        .get_field("Restrictions")
        .expect("a Restrictions column");
    let item = restrictions
        .dtype()
        .serie_item()
        .expect("Restrictions is a sequence");
    assert!(!item.is_nullable(), "every restriction is stated");
    assert_eq!(names(item), ["Name", "Type"]);
    assert!(
        item.fields()
            .iter()
            .all(|child| child.dtype() == &text() && !child.is_nullable())
    );
}

#[test]
fn discover_enumerators_declares_its_columns() {
    pin(
        &RequestType::DiscoverEnumerators,
        &[
            restricting(required("EnumName", text())),
            nullable("EnumDescription", text()),
            required("EnumType", text()),
            required("ElementName", text()),
            nullable("ElementDescription", text()),
            nullable("ElementValue", text()),
        ],
    );
}

#[test]
fn discover_keywords_declares_its_columns() {
    pin(
        &RequestType::DiscoverKeywords,
        &[restricting(required("Keyword", text()))],
    );
}

#[test]
fn discover_literals_declares_its_columns() {
    pin(
        &RequestType::DiscoverLiterals,
        &[
            restricting(required("LiteralName", text())),
            nullable("LiteralValue", text()),
            nullable("LiteralInvalidChars", text()),
            nullable("LiteralInvalidStartingChars", text()),
            nullable("LiteralMaxLength", DataType::Int32),
            nullable("LiteralNameEnumValue", DataType::Int32),
        ],
    );
}

#[test]
fn dbschema_catalogs_declares_its_columns() {
    pin(
        &RequestType::DbschemaCatalogs,
        &[
            restricting(required("CATALOG_NAME", text())),
            nullable("DESCRIPTION", text()),
            nullable("ROLES", text()),
            nullable("DATE_MODIFIED", instant()),
        ],
    );
}

#[test]
fn dbschema_schemata_declares_its_columns() {
    pin(
        &RequestType::DbschemaSchemata,
        &[
            restricting(required("CATALOG_NAME", text())),
            restricting(required("SCHEMA_NAME", text())),
            restricting(nullable("SCHEMA_OWNER", text())),
        ],
    );
}

#[test]
fn dbschema_tables_declares_its_columns() {
    pin(
        &RequestType::DbschemaTables,
        &[
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
    );
}

#[test]
fn dbschema_columns_declares_its_columns() {
    pin(
        &RequestType::DbschemaColumns,
        &[
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
    );
}

#[test]
fn dbschema_provider_types_declares_its_columns() {
    pin(
        &RequestType::DbschemaProviderTypes,
        &[
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
    );
}

// Restrictions.

#[test]
fn restrictions_are_rowset_columns_in_column_order() {
    for definition in definitions() {
        let request_type = definition.request_type();
        let columns = definition.field().fields();
        let mut last = None;
        for restriction in definition.restrictions() {
            let index = columns
                .iter()
                .position(|column| std::ptr::eq(column, restriction))
                .unwrap_or_else(|| {
                    panic!(
                        "{request_type}.{} is borrowed from the rowset's columns",
                        restriction.name()
                    )
                });
            assert!(
                last.is_none_or(|last| last < index),
                "{request_type}'s restrictions follow column order"
            );
            last = Some(index);
        }
        assert!(
            last.is_some(),
            "{request_type} may be restricted by at least one column"
        );
    }
}

#[test]
fn restricts_by_answers_the_restriction_column_in_any_ascii_case() {
    for definition in definitions() {
        for restriction in definition.restrictions() {
            let name = restriction.name();
            for spelled in [
                name.to_owned(),
                name.to_ascii_lowercase(),
                name.to_ascii_uppercase(),
            ] {
                let found = definition.restricts_by(&spelled).unwrap_or_else(|| {
                    panic!(
                        "{}.{name} restricts when spelled `{spelled}`",
                        definition.request_type()
                    )
                });
                assert!(
                    std::ptr::eq(found, restriction),
                    "`{spelled}` answers the column itself"
                );
            }
        }
    }
}

#[test]
fn dbschema_tables_restricts_by_catalog_schema_name_and_type() {
    let tables = answered(&RequestType::DbschemaTables);
    assert_eq!(
        restriction_names(tables),
        ["TABLE_CATALOG", "TABLE_SCHEMA", "TABLE_NAME", "TABLE_TYPE"]
    );
    let name = tables
        .restricts_by("table_name")
        .expect("TABLE_NAME restricts");
    assert_eq!(name.name(), "TABLE_NAME");
    assert_eq!(name.dtype(), &text());
    assert!(!name.is_nullable());
    let schema = tables
        .restricts_by("Table_Schema")
        .expect("TABLE_SCHEMA restricts");
    assert_eq!(schema.name(), "TABLE_SCHEMA");
    assert!(schema.is_nullable(), "a table may sit in no schema");
    assert_eq!(
        tables.restricts_by("tAbLe_TyPe").map(Field::name),
        Some("TABLE_TYPE")
    );
    assert_eq!(
        tables.restricts_by("TABLE_CATALOG").map(Field::name),
        Some("TABLE_CATALOG")
    );
}

#[test]
fn dbschema_columns_restricts_by_catalog_schema_table_and_column() {
    let columns = answered(&RequestType::DbschemaColumns);
    assert_eq!(
        restriction_names(columns),
        ["TABLE_CATALOG", "TABLE_SCHEMA", "TABLE_NAME", "COLUMN_NAME"]
    );
    assert_eq!(
        columns.restricts_by("column_name").map(Field::name),
        Some("COLUMN_NAME")
    );
    for column in [
        "ORDINAL_POSITION",
        "DATA_TYPE",
        "IS_NULLABLE",
        "DESCRIPTION",
    ] {
        assert!(
            columns.restricts_by(column).is_none(),
            "{column} is a DBSCHEMA_COLUMNS column but not a restriction"
        );
    }
}

#[test]
fn the_discover_rowsets_restrict_by_their_specified_columns() {
    let expected: [(RequestType, &[&str]); 6] = [
        (
            RequestType::DiscoverDatasources,
            &[
                "DataSourceName",
                "URL",
                "ProviderName",
                "ProviderType",
                "AuthenticationMode",
            ],
        ),
        (RequestType::DiscoverProperties, &["PropertyName"]),
        (RequestType::DiscoverSchemaRowsets, &["SchemaName"]),
        (RequestType::DiscoverEnumerators, &["EnumName"]),
        (RequestType::DiscoverKeywords, &["Keyword"]),
        (RequestType::DiscoverLiterals, &["LiteralName"]),
    ];
    for (request_type, restrictions) in expected {
        assert_eq!(
            restriction_names(answered(&request_type)),
            restrictions,
            "{request_type}"
        );
    }
    assert_eq!(
        restriction_names(answered(&RequestType::DbschemaCatalogs)),
        ["CATALOG_NAME"]
    );
    assert_eq!(
        restriction_names(answered(&RequestType::DbschemaSchemata)),
        ["CATALOG_NAME", "SCHEMA_NAME", "SCHEMA_OWNER"]
    );
    assert_eq!(
        restriction_names(answered(&RequestType::DbschemaProviderTypes)),
        ["DATA_TYPE", "BEST_MATCH"]
    );
}

// Element names.

/// Whether `name` is an XML name with no colon, spelled in ASCII.
fn is_ascii_ncname(name: &str) -> bool {
    let mut characters = name.chars();
    characters
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
        && characters.all(|rest| rest.is_ascii_alphanumeric() || matches!(rest, '_' | '-' | '.'))
}

#[test]
fn every_column_is_spelled_under_its_encoded_name() {
    for definition in definitions() {
        let request_type = definition.request_type();
        let elements = definition.rowset().element_names();
        let columns = definition.field().fields();
        assert_eq!(elements.len(), columns.len(), "{request_type}");
        for (element, column) in elements.iter().zip(columns) {
            assert_eq!(
                *element,
                encode_name(column.name()).as_str(),
                "{request_type}.{} is spelled under its encoded name",
                column.name()
            );
            assert_eq!(
                decode_name(element).as_str(),
                column.name(),
                "{request_type}.{} reads back out of its element name",
                column.name()
            );
        }
    }
}

#[test]
fn every_specified_column_name_is_already_an_xml_name() {
    // The specification's column names need no `_xHHHH_` escape: each is
    // its own element name, so a client matching elements by column name
    // finds every one.
    for definition in definitions() {
        let request_type = definition.request_type();
        assert_eq!(
            definition.rowset().element_names(),
            names(definition.field()),
            "{request_type} spells every column under its own name"
        );
        for element in definition.rowset().element_names() {
            assert!(
                is_ascii_ncname(element),
                "{request_type}.{element} is an XML name"
            );
            assert!(
                !element.contains("_x"),
                "{request_type}.{element} holds no literal `_x` an encoder escapes"
            );
        }
    }
}

// Round trips through the rowset writer.

#[test]
fn every_rowset_schema_reads_back_its_columns() {
    for definition in definitions() {
        let request_type = definition.request_type();
        let mut document = Vec::new();
        definition
            .rowset()
            .write_root(
                &mut document,
                std::iter::empty::<yggdryl::arrow::Result<Serie>>(),
                true,
                false,
            )
            .unwrap_or_else(|error| panic!("{request_type}'s schema is written: {error}"));
        let value = yggdryl::from_xml_scalar(&document).expect("the rowset is XML");
        let root = yggdryl::xml::Element::root(&value).expect("a root element");
        let (read, rows) = Rowset::read_root(&root, None)
            .unwrap_or_else(|error| panic!("{request_type}'s schema reads back: {error}"));
        assert_eq!(rows.len(), 0, "{request_type}: a schema alone has no rows");
        assert_eq!(read.element_names(), definition.rowset().element_names());
        assert_eq!(names(read.field()), names(definition.field()));
        for (back, declared) in read
            .field()
            .fields()
            .iter()
            .zip(definition.field().fields())
        {
            // A sequence of no items is spelled by no element, so its schema
            // states zero occurrences and reads back as a column that may
            // be absent; every other column keeps what it declared.
            assert_eq!(
                back.is_nullable(),
                declared.is_nullable() || declared.dtype().serie_item().is_some(),
                "{request_type}.{}'s nullability reads back",
                declared.name()
            );
            match (XsdType::of(declared.dtype()), declared.dtype().serie_item()) {
                (Some(xsd), _) => assert_eq!(
                    back.dtype(),
                    &xsd.datatype(),
                    "{request_type}.{} reads back as its XML Schema type's datatype",
                    declared.name()
                ),
                (None, Some(item)) => {
                    let back_item = back
                        .dtype()
                        .serie_item()
                        .unwrap_or_else(|| panic!("{request_type}.{} repeats", declared.name()));
                    assert_eq!(names(back_item), names(item));
                }
                (None, None) => panic!("{request_type}.{} has an element", declared.name()),
            }
        }
    }
}

#[test]
fn a_dbschema_tables_row_reads_back_through_its_rowset() {
    let tables = answered(&RequestType::DbschemaTables);
    let row = Scalar::from_struct([
        ("TABLE_CATALOG", Scalar::from("market")),
        ("TABLE_SCHEMA", Scalar::Null),
        ("TABLE_NAME", Scalar::from("trädes <&> €")),
        ("TABLE_TYPE", Scalar::from("TABLE")),
        ("TABLE_GUID", Scalar::Null),
        ("DESCRIPTION", Scalar::from("")),
        ("TABLE_PROPID", Scalar::from(7_u32)),
        ("DATE_CREATED", Scalar::Null),
        ("DATE_MODIFIED", Scalar::Null),
    ])
    .expect("a named row");
    let rows =
        Serie::from_scalars(Arc::clone(tables.rowset().shared_field()), [row]).expect("one row");
    let mut document = Vec::new();
    tables
        .rowset()
        .write_root(&mut document, std::iter::once(Ok(rows.clone())), true, true)
        .expect("the rowset is written");
    let text = String::from_utf8(document.clone()).expect("UTF-8");
    assert!(
        text.contains("<TABLE_NAME>trädes &lt;&amp;&gt; €</TABLE_NAME>"),
        "{text}"
    );
    assert!(
        !text.contains("<TABLE_SCHEMA>"),
        "an absent cell writes no element"
    );
    let value = yggdryl::from_xml_scalar(&document).expect("the rowset is XML");
    let root = yggdryl::xml::Element::root(&value).expect("a root element");
    let (read, back) = Rowset::read_root(&root, Some(tables.field())).expect("the rows read back");
    assert_eq!(read.field(), tables.field());
    assert_eq!(back, rows);
}

// Literal documents: what a client or another provider writes, read under a
// definition's field.

const ID: u128 = 0x0123_4567_89ab_cdef_0123_4567_89ab_cdef;
const ID_TEXT: &str = "01234567-89ab-cdef-0123-456789abcdef";

/// One named row.
fn row<const N: usize>(entries: [(&str, Scalar); N]) -> Scalar {
    Scalar::from_struct(entries).expect("a named row")
}

/// `rows` laid out under `definition`'s record field.
fn laid_out(definition: &Definition, rows: impl IntoIterator<Item = Scalar>) -> Serie {
    Serie::from_scalars(Arc::clone(definition.rowset().shared_field()), rows)
        .unwrap_or_else(|error| panic!("{} lays out its rows: {error}", definition.request_type()))
}

/// The refusal laying `rows` out under `definition`'s record field answers.
fn layout_refusal(definition: &Definition, rows: impl IntoIterator<Item = Scalar>) -> String {
    Serie::from_scalars(Arc::clone(definition.rowset().shared_field()), rows)
        .expect_err("the rows are refused")
        .to_string()
}

/// Read a literal `root` document under `definition`'s field.
fn read_literal(definition: &Definition, document: &str) -> yggdryl::Result<Serie> {
    let value = yggdryl::from_xml_scalar(document)?;
    let root = Element::root(&value)?;
    let (read, rows) = Rowset::read_root(&root, Some(definition.field()))?;
    assert_eq!(
        read.field(),
        definition.field(),
        "{} reads under its own field",
        definition.request_type()
    );
    Ok(rows)
}

/// The refusal reading a literal `root` document under `definition` answers.
fn literal_refusal(definition: &Definition, document: &str) -> String {
    read_literal(definition, document)
        .expect_err("the document is refused")
        .to_string()
}

/// A schema-less `root` document in the rowset namespace holding `rows`.
fn rowset_document(rows: &str) -> String {
    format!("<root xmlns=\"{ROWSET_NAMESPACE}\" xmlns:xsi=\"{XSI_NAMESPACE}\">{rows}</root>")
}

/// The UTC instant at `hours:minutes` on 2024-01-01, to the microsecond.
fn utc_at(hours: i64, minutes: i64, micros: i64) -> Scalar {
    // 2024-01-01T00:00:00Z is 19_723 days after the epoch.
    let count = 19_723_i64 * 86_400_000_000 + (hours * 3_600 + minutes * 60) * 1_000_000 + micros;
    Scalar::datetime64(count, TimeUnit::Microsecond, Timezone::UTC).expect("a UTC instant")
}

/// The whole document `definition`'s rowset writes: its schema and `rows`.
fn written(definition: &Definition, rows: &Serie) -> String {
    let mut document = Vec::new();
    definition
        .rowset()
        .write_root(&mut document, std::iter::once(Ok(rows.clone())), true, true)
        .unwrap_or_else(|error| panic!("{} is written: {error}", definition.request_type()));
    String::from_utf8(document).expect("UTF-8")
}

/// The rows a document `definition`'s rowset wrote reads back as, under its
/// own field.
fn read_back(definition: &Definition, document: &str) -> Serie {
    read_literal(definition, document)
        .unwrap_or_else(|error| panic!("{} reads back: {error}", definition.request_type()))
}

#[test]
fn a_literal_row_missing_a_required_column_is_refused_naming_the_row_and_column() {
    let tables = answered(&RequestType::DbschemaTables);
    let refusal = literal_refusal(
        tables,
        &rowset_document(
            "<row><TABLE_CATALOG>market</TABLE_CATALOG><TABLE_NAME>trades</TABLE_NAME>\
             <TABLE_TYPE>TABLE</TABLE_TYPE></row>\
             <row><TABLE_CATALOG>market</TABLE_CATALOG><TABLE_TYPE>TABLE</TABLE_TYPE></row>",
        ),
    );
    assert!(
        refusal.contains("$[1]"),
        "the second row is named: {refusal}"
    );
    assert!(
        refusal.contains("`TABLE_NAME`"),
        "the column is named: {refusal}"
    );
    assert!(refusal.contains("required"), "{refusal}");
    for (request_type, required) in [
        (RequestType::DiscoverKeywords, "Keyword"),
        (RequestType::DbschemaCatalogs, "CATALOG_NAME"),
    ] {
        let refusal = literal_refusal(answered(&request_type), &rowset_document("<row/>"));
        assert!(refusal.contains("$[0]"), "{request_type}: {refusal}");
        assert!(
            refusal.contains(&format!("`{required}`")),
            "{request_type} names {required}: {refusal}"
        );
    }
}

#[test]
fn a_literal_nil_required_cell_is_refused_naming_its_column() {
    let keywords = answered(&RequestType::DiscoverKeywords);
    let refusal = literal_refusal(
        keywords,
        &rowset_document(
            "<row><Keyword>select</Keyword></row><row><Keyword xsi:nil=\"true\"/></row>",
        ),
    );
    assert!(refusal.contains("$[1]"), "{refusal}");
    assert!(
        refusal.contains("$.row.Keyword") && refusal.contains("null"),
        "{refusal}"
    );
    // A self-closed element is null too, and an emptied one the empty text.
    let refusal = literal_refusal(keywords, &rowset_document("<row><Keyword/></row>"));
    assert!(
        refusal.contains("$[0]") && refusal.contains("Keyword"),
        "{refusal}"
    );
    let rows = read_literal(keywords, &rowset_document("<row><Keyword></Keyword></row>"))
        .unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(
        rows,
        laid_out(keywords, [row([("Keyword", Scalar::from(""))])])
    );
}

#[test]
fn a_literal_value_outside_the_specifications_width_is_refused_naming_its_column() {
    let columns = answered(&RequestType::DbschemaColumns);
    let cell = |column: &str, value: &str| {
        let mut cells = vec![
            ("TABLE_CATALOG", "market".to_owned()),
            ("TABLE_NAME", "trades".to_owned()),
            ("COLUMN_NAME", "price".to_owned()),
            ("ORDINAL_POSITION", "1".to_owned()),
            ("COLUMN_FLAGS", "0".to_owned()),
            ("IS_NULLABLE", "true".to_owned()),
            ("DATA_TYPE", "5".to_owned()),
        ];
        match cells.iter_mut().find(|(name, _)| *name == column) {
            Some((_, held)) => *held = value.to_owned(),
            None => cells.push((column, value.to_owned())),
        }
        let row: String = cells
            .iter()
            .map(|(name, value)| format!("<{name}>{value}</{name}>"))
            .collect();
        rowset_document(&format!("<row>{row}</row>"))
    };
    // OLE DB states DATA_TYPE as a WORD, NUMERIC_PRECISION as a USHORT,
    // NUMERIC_SCALE as a SHORT, and the ordinal, flags and lengths as ULONGs.
    for (column, value) in [
        ("DATA_TYPE", "65536"),
        ("DATA_TYPE", "-1"),
        ("NUMERIC_PRECISION", "65536"),
        ("NUMERIC_SCALE", "32768"),
        ("NUMERIC_SCALE", "-32769"),
        ("ORDINAL_POSITION", "4294967296"),
        ("COLUMN_FLAGS", "-1"),
        ("CHARACTER_MAXIMUM_LENGTH", "4294967296"),
        ("IS_NULLABLE", "maybe"),
        ("COLUMN_HAS_DEFAULT", "yes"),
    ] {
        let refusal = literal_refusal(columns, &cell(column, value));
        assert!(
            refusal.contains("$[0]") && refusal.contains(&format!("$.row.{column}:")),
            "{column} = {value} is refused naming the row and the column: {refusal}"
        );
    }
    for (column, value) in [
        ("DATA_TYPE", "65535"),
        ("DATA_TYPE", "0"),
        ("NUMERIC_PRECISION", "65535"),
        ("NUMERIC_SCALE", "-32768"),
        ("NUMERIC_SCALE", "32767"),
        ("ORDINAL_POSITION", "4294967295"),
        ("CHARACTER_OCTET_LENGTH", "4294967295"),
        ("IS_NULLABLE", "false"),
    ] {
        let rows = read_literal(columns, &cell(column, value))
            .unwrap_or_else(|error| panic!("{column} = {value} reads: {error}"));
        assert_eq!(rows.len(), 1, "{column} = {value}");
    }
    let tables = answered(&RequestType::DbschemaTables);
    for (column, value, expected) in [
        ("TABLE_GUID", "not-a-uuid", "uuid"),
        ("TABLE_PROPID", "-7", "uint32"),
        ("DATE_CREATED", "yesterday", "ISO"),
    ] {
        let refusal = literal_refusal(
            tables,
            &rowset_document(&format!(
                "<row><TABLE_CATALOG>market</TABLE_CATALOG><TABLE_NAME>trades</TABLE_NAME>\
                 <TABLE_TYPE>TABLE</TABLE_TYPE><{column}>{value}</{column}></row>"
            )),
        );
        assert!(
            refusal.contains(&format!("$.row.{column}:")) && refusal.contains(expected),
            "{column} = {value} is refused naming the column and what it holds: {refusal}"
        );
    }
}

#[test]
fn a_literal_cell_the_rowset_does_not_have_is_refused_naming_it() {
    let keywords = answered(&RequestType::DiscoverKeywords);
    let refusal = literal_refusal(
        keywords,
        &rowset_document("<row><Keyword>select</Keyword><Synonym>pick</Synonym></row>"),
    );
    assert!(refusal.contains("Synonym"), "{refusal}");
    assert!(refusal.contains("$[0]"), "{refusal}");
}

#[test]
fn a_literal_dbschema_tables_rowset_reads_under_its_definition() {
    let tables = answered(&RequestType::DbschemaTables);
    // Another provider's spelling: the rowset namespace and the instance
    // namespace under prefixes of its own, indentation between elements,
    // character references, an offset instant and one in UTC.
    let document = format!(
        "<r:root xmlns:r=\"{ROWSET_NAMESPACE}\" xmlns:i=\"{XSI_NAMESPACE}\">\n  \
         <r:row>\n    \
         <r:TABLE_CATALOG>market</r:TABLE_CATALOG>\n    \
         <r:TABLE_SCHEMA i:nil=\"true\"/>\n    \
         <r:TABLE_NAME>tr&#xE4;des &lt;&amp;&gt; \u{20AC}</r:TABLE_NAME>\n    \
         <r:TABLE_TYPE>TABLE</r:TABLE_TYPE>\n    \
         <r:TABLE_GUID>{ID_TEXT}</r:TABLE_GUID>\n    \
         <r:DESCRIPTION>Every fill</r:DESCRIPTION>\n    \
         <r:TABLE_PROPID>4294967295</r:TABLE_PROPID>\n    \
         <r:DATE_CREATED>2024-01-01T10:00:00Z</r:DATE_CREATED>\n    \
         <r:DATE_MODIFIED>2024-01-01T12:30:00.000001+02:00</r:DATE_MODIFIED>\n  \
         </r:row>\n  \
         <r:row><r:TABLE_CATALOG>market</r:TABLE_CATALOG><r:TABLE_NAME>quotes</r:TABLE_NAME>\
         <r:TABLE_TYPE>VIEW</r:TABLE_TYPE></r:row>\n  \
         <r:row><r:TABLE_CATALOG>market</r:TABLE_CATALOG><r:TABLE_NAME>  padded  </r:TABLE_NAME>\
         <r:TABLE_TYPE>SYSTEM TABLE</r:TABLE_TYPE><r:DESCRIPTION></r:DESCRIPTION>\
         <r:TABLE_PROPID> 7 </r:TABLE_PROPID><r:DATE_CREATED/><r:DATE_MODIFIED></r:DATE_MODIFIED>\
         </r:row>\n\
         </r:root>"
    );
    let rows = read_literal(tables, &document).unwrap_or_else(|error| panic!("{error}"));
    let expected = laid_out(
        tables,
        [
            row([
                ("TABLE_CATALOG", Scalar::from("market")),
                ("TABLE_SCHEMA", Scalar::Null),
                ("TABLE_NAME", Scalar::from("trädes <&> €")),
                ("TABLE_TYPE", Scalar::from("TABLE")),
                ("TABLE_GUID", Scalar::Uuid(Uuid::new(ID))),
                ("DESCRIPTION", Scalar::from("Every fill")),
                ("TABLE_PROPID", Scalar::from(u32::MAX)),
                ("DATE_CREATED", utc_at(10, 0, 0)),
                ("DATE_MODIFIED", utc_at(10, 30, 1)),
            ]),
            row([
                ("TABLE_CATALOG", Scalar::from("market")),
                ("TABLE_SCHEMA", Scalar::Null),
                ("TABLE_NAME", Scalar::from("quotes")),
                ("TABLE_TYPE", Scalar::from("VIEW")),
                ("TABLE_GUID", Scalar::Null),
                ("DESCRIPTION", Scalar::Null),
                ("TABLE_PROPID", Scalar::Null),
                ("DATE_CREATED", Scalar::Null),
                ("DATE_MODIFIED", Scalar::Null),
            ]),
            // A text cell keeps its padding and an emptied one is the empty
            // text; a number is read past its padding and an emptied or
            // self-closed non-text cell is absent.
            row([
                ("TABLE_CATALOG", Scalar::from("market")),
                ("TABLE_SCHEMA", Scalar::Null),
                ("TABLE_NAME", Scalar::from("  padded  ")),
                ("TABLE_TYPE", Scalar::from("SYSTEM TABLE")),
                ("TABLE_GUID", Scalar::Null),
                ("DESCRIPTION", Scalar::from("")),
                ("TABLE_PROPID", Scalar::from(7_u32)),
                ("DATE_CREATED", Scalar::Null),
                ("DATE_MODIFIED", Scalar::Null),
            ]),
        ],
    );
    assert_eq!(rows, expected);
}

#[test]
fn a_literal_discover_schema_rowsets_rowset_reads_its_restrictions_as_a_sequence() {
    let schema_rowsets = answered(&RequestType::DiscoverSchemaRowsets);
    let document = rowset_document(&format!(
        "<row><SchemaName>DBSCHEMA_SCHEMATA</SchemaName><SchemaGuid>{ID_TEXT}</SchemaGuid>\
         <Restrictions><Name>CATALOG_NAME</Name><Type>xsd:string</Type></Restrictions>\
         <Restrictions><Name>SCHEMA_NAME</Name><Type>xsd:string</Type></Restrictions>\
         <Description>Schemas</Description></row>\
         <row><SchemaName>DISCOVER_KEYWORDS</SchemaName>\
         <Restrictions><Name>Keyword</Name><Type>xsd:string</Type></Restrictions>\
         <Description>Keywords</Description></row>\
         <row><SchemaName>DISCOVER_NOTHING</SchemaName><Description></Description></row>"
    ));
    let rows = read_literal(schema_rowsets, &document).unwrap_or_else(|error| panic!("{error}"));
    let restriction = |name: &str| {
        row([
            ("Name", Scalar::from(name)),
            ("Type", Scalar::from("xsd:string")),
        ])
    };
    let expected = laid_out(
        schema_rowsets,
        [
            row([
                ("SchemaName", Scalar::from("DBSCHEMA_SCHEMATA")),
                ("SchemaGuid", Scalar::Uuid(Uuid::new(ID))),
                (
                    "Restrictions",
                    Scalar::from_sequence([
                        restriction("CATALOG_NAME"),
                        restriction("SCHEMA_NAME"),
                    ]),
                ),
                ("Description", Scalar::from("Schemas")),
            ]),
            row([
                ("SchemaName", Scalar::from("DISCOVER_KEYWORDS")),
                ("SchemaGuid", Scalar::Null),
                (
                    "Restrictions",
                    Scalar::from_sequence([restriction("Keyword")]),
                ),
                ("Description", Scalar::from("Keywords")),
            ]),
            row([
                ("SchemaName", Scalar::from("DISCOVER_NOTHING")),
                ("SchemaGuid", Scalar::Null),
                ("Restrictions", Scalar::from_sequence([])),
                ("Description", Scalar::from("")),
            ]),
        ],
    );
    assert_eq!(rows, expected);
}

#[test]
fn a_literal_discover_datasources_rowset_reads_provider_type_as_a_sequence() {
    let datasources = answered(&RequestType::DiscoverDatasources);
    let document = rowset_document(
        "<row><DataSourceName>yggdryl</DataSourceName><URL>http://localhost:8080/xmla</URL>\
         <ProviderName>yggdryl</ProviderName>\
         <ProviderType>TDP</ProviderType><ProviderType>MDP</ProviderType>\
         <AuthenticationMode>Unauthenticated</AuthenticationMode></row>",
    );
    let rows = read_literal(datasources, &document).unwrap_or_else(|error| panic!("{error}"));
    let expected = laid_out(
        datasources,
        [row([
            ("DataSourceName", Scalar::from("yggdryl")),
            ("DataSourceDescription", Scalar::Null),
            ("URL", Scalar::from("http://localhost:8080/xmla")),
            ("DataSourceInfo", Scalar::Null),
            ("ProviderName", Scalar::from("yggdryl")),
            (
                "ProviderType",
                Scalar::from_sequence([Scalar::from("TDP"), Scalar::from("MDP")]),
            ),
            ("AuthenticationMode", Scalar::from("Unauthenticated")),
        ])],
    );
    assert_eq!(rows, expected);
}

// Rows laid out under a definition's field.

#[test]
fn a_required_column_refuses_a_null_naming_it_and_a_nullable_one_holds_it() {
    for definition in definitions() {
        let request_type = definition.request_type();
        let columns = definition.field().fields();
        // Every column at its field's default: absent where it may be.
        let defaults: Vec<(&str, Scalar)> = columns
            .iter()
            .map(|column| {
                let value = column
                    .default_value()
                    .unwrap_or_else(|error| panic!("{request_type}.{}: {error}", column.name()));
                assert_eq!(
                    value.is_null(),
                    column.is_nullable(),
                    "{request_type}.{}'s default is absent exactly when it may be",
                    column.name()
                );
                (column.name(), value)
            })
            .collect();
        assert_eq!(
            laid_out(definition, [row_of(&defaults)]).len(),
            1,
            "{request_type}"
        );
        for (index, column) in columns.iter().enumerate() {
            let mut cells = defaults.clone();
            cells[index].1 = Scalar::Null;
            let name = column.name();
            if column.is_nullable() {
                let rows = laid_out(definition, [row_of(&cells)]);
                let back = rows.scalar(0).expect("the row");
                assert!(
                    back.get(index).is_some_and(|cell| cell.is_null()),
                    "{request_type}.{name} holds an absent value"
                );
            } else {
                let refusal = layout_refusal(definition, [row_of(&cells)]);
                assert!(
                    refusal.contains(&format!("$.row.{name}")) && refusal.contains("null"),
                    "{request_type}.{name} refuses a null by name: {refusal}"
                );
            }
        }
    }
}

/// One named row from owned cells.
fn row_of(cells: &[(&str, Scalar)]) -> Scalar {
    Scalar::from_struct(cells.iter().map(|(name, value)| (*name, value.clone())))
        .expect("a named row")
}

#[test]
fn a_row_naming_a_column_the_rowset_does_not_have_is_refused_naming_it() {
    let keywords = answered(&RequestType::DiscoverKeywords);
    let refusal = layout_refusal(
        keywords,
        [row([
            ("Keyword", Scalar::from("select")),
            ("KEYWORD", Scalar::from("SELECT")),
        ])],
    );
    assert!(refusal.contains("KEYWORD"), "{refusal}");
}

#[test]
fn every_rowset_with_no_rows_reads_back_as_no_rows_under_its_field() {
    for definition in definitions() {
        let request_type = definition.request_type();
        let empty = laid_out(definition, []);
        assert_eq!(empty.len(), 0);
        let document = written(definition, &empty);
        assert!(
            !document.contains("<row>"),
            "{request_type}: no row is written: {document}"
        );
        let back = read_back(definition, &document);
        assert_eq!(back.len(), 0, "{request_type}");
        assert_eq!(back, empty, "{request_type}");
    }
}

#[test]
fn an_empty_root_reads_as_no_rows_under_every_definition() {
    for definition in definitions() {
        let request_type = definition.request_type();
        for document in [
            format!("<root xmlns=\"{ROWSET_NAMESPACE}\"/>"),
            format!("<x:root xmlns:x=\"{ROWSET_NAMESPACE}\">\n</x:root>"),
            "<root></root>".to_owned(),
        ] {
            let rows = read_literal(definition, &document)
                .unwrap_or_else(|error| panic!("{request_type} reads {document}: {error}"));
            assert_eq!(rows, laid_out(definition, []), "{request_type}: {document}");
        }
    }
}

#[test]
fn rows_in_no_namespace_read_as_rows_of_the_rowset() {
    let catalogs = answered(&RequestType::DbschemaCatalogs);
    let rows = read_literal(
        catalogs,
        "<root><row><CATALOG_NAME>market</CATALOG_NAME><ROLES>reader</ROLES></row>\
         <row><CATALOG_NAME>\u{65E5}\u{672C}</CATALOG_NAME>\
         <DATE_MODIFIED>2024-01-01T00:00:00Z</DATE_MODIFIED></row></root>",
    )
    .unwrap_or_else(|error| panic!("{error}"));
    let expected = laid_out(
        catalogs,
        [
            row([
                ("CATALOG_NAME", Scalar::from("market")),
                ("DESCRIPTION", Scalar::Null),
                ("ROLES", Scalar::from("reader")),
                ("DATE_MODIFIED", Scalar::Null),
            ]),
            row([
                ("CATALOG_NAME", Scalar::from("\u{65E5}\u{672C}")),
                ("DESCRIPTION", Scalar::Null),
                ("ROLES", Scalar::Null),
                ("DATE_MODIFIED", utc_at(0, 0, 0)),
            ]),
        ],
    );
    assert_eq!(rows, expected);
}

#[test]
fn the_schema_rowsets_rowset_carries_every_definition_in_order_and_reads_back() {
    // The rows DISCOVER_SCHEMA_ROWSETS answers are the definitions
    // themselves: one per definition, each restriction as its name and type.
    let schema_rowsets = answered(&RequestType::DiscoverSchemaRowsets);
    let rows = laid_out(
        schema_rowsets,
        definitions().iter().map(|definition| {
            row([
                (
                    "SchemaName",
                    Scalar::from(definition.request_type().as_str()),
                ),
                ("SchemaGuid", Scalar::Null),
                (
                    "Restrictions",
                    Scalar::from_sequence(definition.restrictions().into_iter().map(|field| {
                        row([
                            ("Name", Scalar::from(field.name())),
                            (
                                "Type",
                                Scalar::from(
                                    XsdType::of(field.dtype())
                                        .unwrap_or(XsdType::String)
                                        .as_str(),
                                ),
                            ),
                        ])
                    })),
                ),
                ("Description", Scalar::from(definition.description())),
            ])
        }),
    );
    assert_eq!(rows.len(), definitions().len());
    let document = written(schema_rowsets, &rows);
    assert!(
        document.contains(
            "<row><SchemaName>DBSCHEMA_PROVIDER_TYPES</SchemaName>\
             <Restrictions><Name>DATA_TYPE</Name><Type>xsd:unsignedShort</Type></Restrictions>\
             <Restrictions><Name>BEST_MATCH</Name><Type>xsd:boolean</Type></Restrictions>\
             <Description>The datatypes this provider's columns take, as OLE DB types them.\
             </Description></row>"
        ),
        "{document}"
    );
    let order: Vec<usize> = definitions()
        .iter()
        .map(|definition| {
            document
                .find(&format!(
                    "<SchemaName>{}</SchemaName>",
                    definition.request_type()
                ))
                .unwrap_or_else(|| panic!("{} is listed", definition.request_type()))
        })
        .collect();
    assert!(
        order.windows(2).all(|pair| pair[0] < pair[1]),
        "the rows follow the definitions' order"
    );
    assert_eq!(read_back(schema_rowsets, &document), rows);
}

#[test]
fn the_ole_db_widths_round_trip_at_their_bounds() {
    let columns = answered(&RequestType::DbschemaColumns);
    let at = |data_type: u16, scale: i16, position: u32, nullable: bool| {
        row([
            ("TABLE_CATALOG", Scalar::from("market")),
            ("TABLE_SCHEMA", Scalar::Null),
            ("TABLE_NAME", Scalar::from("trades")),
            ("COLUMN_NAME", Scalar::from("price")),
            ("ORDINAL_POSITION", Scalar::from(position)),
            ("COLUMN_HAS_DEFAULT", Scalar::Null),
            ("COLUMN_FLAGS", Scalar::from(0_u32)),
            ("IS_NULLABLE", Scalar::from(nullable)),
            ("DATA_TYPE", Scalar::from(data_type)),
            ("CHARACTER_MAXIMUM_LENGTH", Scalar::Null),
            ("CHARACTER_OCTET_LENGTH", Scalar::Null),
            ("NUMERIC_PRECISION", Scalar::from(u16::MAX)),
            ("NUMERIC_SCALE", Scalar::from(scale)),
            ("DESCRIPTION", Scalar::Null),
        ])
    };
    let rows = laid_out(
        columns,
        [
            at(u16::MAX, i16::MIN, u32::MAX, false),
            at(0, i16::MAX, 1, true),
        ],
    );
    let document = written(columns, &rows);
    for cell in [
        "<DATA_TYPE>65535</DATA_TYPE>",
        "<NUMERIC_SCALE>-32768</NUMERIC_SCALE>",
        "<ORDINAL_POSITION>4294967295</ORDINAL_POSITION>",
        "<IS_NULLABLE>false</IS_NULLABLE>",
        "<IS_NULLABLE>true</IS_NULLABLE>",
        "<DATA_TYPE>0</DATA_TYPE>",
        "<NUMERIC_SCALE>32767</NUMERIC_SCALE>",
    ] {
        assert!(document.contains(cell), "{cell} in {document}");
    }
    assert!(
        !document.contains("<TABLE_SCHEMA"),
        "an absent schema writes no element"
    );
    assert_eq!(read_back(columns, &document), rows);
}

// Restriction names across rowsets.

#[test]
fn a_column_restricts_only_the_rowsets_that_declare_it_a_restriction() {
    let restricts = |request_type: RequestType, column: &str| {
        answered(&request_type).restricts_by(column).is_some()
    };
    assert!(restricts(RequestType::DbschemaProviderTypes, "data_type"));
    assert!(!restricts(RequestType::DbschemaColumns, "data_type"));
    assert!(restricts(RequestType::DbschemaCatalogs, "catalog_name"));
    assert!(restricts(RequestType::DbschemaSchemata, "catalog_name"));
    assert!(!restricts(RequestType::DbschemaTables, "catalog_name"));
    assert!(restricts(RequestType::DbschemaTables, "table_catalog"));
    assert!(!restricts(RequestType::DbschemaCatalogs, "table_catalog"));
    for definition in definitions() {
        assert!(
            definition.restricts_by("DESCRIPTION").is_none()
                && definition.restricts_by("Description").is_none(),
            "{} is never restricted by its description",
            definition.request_type()
        );
    }
}

#[test]
fn no_two_columns_of_a_rowset_differ_only_in_ascii_case() {
    // `restricts_by` folds ASCII case, so two such columns would make one
    // spelling name either.
    for definition in definitions() {
        let mut folded: Vec<String> = names(definition.field())
            .into_iter()
            .map(str::to_ascii_lowercase)
            .collect();
        let count = folded.len();
        folded.sort();
        folded.dedup();
        assert_eq!(folded.len(), count, "{}", definition.request_type());
    }
}

#[test]
fn a_restriction_spelled_with_a_non_ascii_look_alike_is_not_the_column() {
    let literals = answered(&RequestType::DiscoverLiterals);
    assert!(literals.restricts_by("literalname").is_some());
    // DOTLESS I, a Latin capital I with a dot, and a full-width L each fold
    // to an ASCII letter under Unicode rules and to nothing under ASCII's.
    for column in [
        "L\u{131}teralName",
        "L\u{130}TERALNAME",
        "\u{FF2C}iteralName",
    ] {
        assert!(
            literals.restricts_by(column).is_none(),
            "{column:?} is not LiteralName"
        );
    }
}

// The naming rule and the XML Schema the definitions are declared as.

/// Every column name of `record`, a nested struct's children and a
/// sequence's struct item's children included.
fn every_name(record: &Field) -> Vec<&str> {
    let mut all = Vec::new();
    for column in record.fields() {
        all.push(column.name());
        let inner = column.dtype().serie_item().unwrap_or(column);
        if inner.is_struct() {
            all.extend(every_name(inner));
        }
    }
    all
}

#[test]
fn discover_columns_are_pascal_case_and_ole_db_columns_upper_snake() {
    for definition in definitions() {
        let request_type = definition.request_type();
        for name in every_name(definition.field()) {
            if request_type.is_discover() {
                assert!(
                    name.starts_with(|first: char| first.is_ascii_uppercase())
                        && name.chars().all(|held| held.is_ascii_alphanumeric()),
                    "{request_type}.{name} is PascalCase"
                );
            } else {
                assert!(
                    !name.starts_with('_')
                        && !name.ends_with('_')
                        && !name.contains("__")
                        && name
                            .chars()
                            .all(|held| held.is_ascii_uppercase() || held == '_'),
                    "{request_type}.{name} is UPPER_SNAKE"
                );
            }
        }
    }
}

/// The `row` type declarations a rowset schema writes for leaf columns:
/// `(name, XML Schema type, nullable)`.
fn declarations(columns: &[(&str, &str, bool)]) -> String {
    columns
        .iter()
        .map(|(name, xsd, nullable)| {
            format!(
                "<xsd:element sql:field=\"{name}\" name=\"{name}\" type=\"{xsd}\"{}/>",
                if *nullable { " minOccurs=\"0\"" } else { "" }
            )
        })
        .collect()
}

#[test]
fn every_rowset_declares_its_columns_under_the_specifications_xml_schema_types() {
    const S: &str = "xsd:string";
    const B: &str = "xsd:boolean";
    const US: &str = "xsd:unsignedShort";
    const UI: &str = "xsd:unsignedInt";
    const SH: &str = "xsd:short";
    const DT: &str = "xsd:dateTime";
    let expected: Vec<(RequestType, String)> = vec![
        (
            RequestType::DiscoverDatasources,
            [
                declarations(&[
                    ("DataSourceName", S, false),
                    ("DataSourceDescription", S, true),
                    ("URL", S, true),
                    ("DataSourceInfo", S, true),
                    ("ProviderName", S, true),
                ]),
                "<xsd:element sql:field=\"ProviderType\" name=\"ProviderType\" \
                 type=\"xsd:string\" minOccurs=\"0\" maxOccurs=\"unbounded\"/>"
                    .to_owned(),
                declarations(&[("AuthenticationMode", S, false)]),
            ]
            .concat(),
        ),
        (
            RequestType::DiscoverProperties,
            declarations(&[
                ("PropertyName", S, false),
                ("PropertyDescription", S, false),
                ("PropertyType", S, false),
                ("PropertyAccessType", S, false),
                ("IsRequired", B, false),
                ("Value", S, true),
            ]),
        ),
        (
            RequestType::DiscoverSchemaRowsets,
            [
                declarations(&[("SchemaName", S, false), ("SchemaGuid", "uuid", true)]),
                format!(
                    "<xsd:element sql:field=\"Restrictions\" name=\"Restrictions\" \
                     minOccurs=\"0\" maxOccurs=\"unbounded\"><xsd:complexType><xsd:sequence>{}\
                     </xsd:sequence></xsd:complexType></xsd:element>",
                    declarations(&[("Name", S, false), ("Type", S, false)])
                ),
                declarations(&[("Description", S, false)]),
            ]
            .concat(),
        ),
        (
            RequestType::DiscoverEnumerators,
            declarations(&[
                ("EnumName", S, false),
                ("EnumDescription", S, true),
                ("EnumType", S, false),
                ("ElementName", S, false),
                ("ElementDescription", S, true),
                ("ElementValue", S, true),
            ]),
        ),
        (
            RequestType::DiscoverKeywords,
            declarations(&[("Keyword", S, false)]),
        ),
        (
            RequestType::DiscoverLiterals,
            declarations(&[
                ("LiteralName", S, false),
                ("LiteralValue", S, true),
                ("LiteralInvalidChars", S, true),
                ("LiteralInvalidStartingChars", S, true),
                ("LiteralMaxLength", "xsd:int", true),
                ("LiteralNameEnumValue", "xsd:int", true),
            ]),
        ),
        (
            RequestType::DbschemaCatalogs,
            declarations(&[
                ("CATALOG_NAME", S, false),
                ("DESCRIPTION", S, true),
                ("ROLES", S, true),
                ("DATE_MODIFIED", DT, true),
            ]),
        ),
        (
            RequestType::DbschemaColumns,
            declarations(&[
                ("TABLE_CATALOG", S, false),
                ("TABLE_SCHEMA", S, true),
                ("TABLE_NAME", S, false),
                ("COLUMN_NAME", S, false),
                ("ORDINAL_POSITION", UI, false),
                ("COLUMN_HAS_DEFAULT", B, true),
                ("COLUMN_FLAGS", UI, false),
                ("IS_NULLABLE", B, false),
                ("DATA_TYPE", US, false),
                ("CHARACTER_MAXIMUM_LENGTH", UI, true),
                ("CHARACTER_OCTET_LENGTH", UI, true),
                ("NUMERIC_PRECISION", US, true),
                ("NUMERIC_SCALE", SH, true),
                ("DESCRIPTION", S, true),
            ]),
        ),
        (
            RequestType::DbschemaProviderTypes,
            declarations(&[
                ("TYPE_NAME", S, false),
                ("DATA_TYPE", US, false),
                ("COLUMN_SIZE", UI, false),
                ("LITERAL_PREFIX", S, true),
                ("LITERAL_SUFFIX", S, true),
                ("CREATE_PARAMS", S, true),
                ("IS_NULLABLE", B, true),
                ("CASE_SENSITIVE", B, true),
                ("SEARCHABLE", UI, true),
                ("UNSIGNED_ATTRIBUTE", B, true),
                ("FIXED_PREC_SCALE", B, true),
                ("AUTO_UNIQUE_VALUE", B, true),
                ("LOCAL_TYPE_NAME", S, true),
                ("MINIMUM_SCALE", SH, true),
                ("MAXIMUM_SCALE", SH, true),
                ("IS_LONG", B, true),
                ("BEST_MATCH", B, true),
                ("IS_FIXEDLENGTH", B, true),
            ]),
        ),
        (
            RequestType::DbschemaSchemata,
            declarations(&[
                ("CATALOG_NAME", S, false),
                ("SCHEMA_NAME", S, false),
                ("SCHEMA_OWNER", S, true),
            ]),
        ),
        (
            RequestType::DbschemaTables,
            declarations(&[
                ("TABLE_CATALOG", S, false),
                ("TABLE_SCHEMA", S, true),
                ("TABLE_NAME", S, false),
                ("TABLE_TYPE", S, false),
                ("TABLE_GUID", "uuid", true),
                ("DESCRIPTION", S, true),
                ("TABLE_PROPID", UI, true),
                ("DATE_CREATED", DT, true),
                ("DATE_MODIFIED", DT, true),
            ]),
        ),
    ];
    assert_eq!(
        expected
            .iter()
            .map(|(request_type, _)| request_type.clone())
            .collect::<Vec<_>>(),
        covered(),
        "every definition is pinned"
    );
    for (request_type, declared) in expected {
        let mut schema = Vec::new();
        answered(&request_type)
            .rowset()
            .write_schema(&mut schema)
            .unwrap_or_else(|error| panic!("{request_type}'s schema is written: {error}"));
        let schema = String::from_utf8(schema).expect("UTF-8");
        assert!(
            schema.starts_with(&format!(
                "<xsd:schema targetNamespace=\"{ROWSET_NAMESPACE}\" xmlns:sql=\"{SQL_NAMESPACE}\""
            )),
            "{request_type}: {schema}"
        );
        let row_type = format!(
            "<xsd:complexType name=\"row\"><xsd:sequence>{declared}</xsd:sequence>\
             </xsd:complexType></xsd:schema>"
        );
        assert!(
            schema.ends_with(&row_type),
            "{request_type} declares\n{row_type}\nin\n{schema}"
        );
    }
}

// Sharing.

#[test]
fn a_definition_is_shared_across_threads_and_debugs_as_itself() {
    fn shared<T: Send + Sync + 'static>(_: &T) {}
    let tables = answered(&RequestType::DbschemaTables);
    shared(tables);
    let debug = format!("{tables:?}");
    assert!(debug.starts_with("Definition {"), "{debug}");
    assert!(debug.contains("DbschemaTables"), "{debug}");
    assert!(
        debug.contains(&format!("{:?}", tables.description())),
        "the description: {debug}"
    );
    assert!(
        debug.contains("TABLE_NAME"),
        "the rowset's columns: {debug}"
    );
    assert_ne!(
        debug,
        format!("{:?}", answered(&RequestType::DbschemaColumns))
    );
}

// What the provider states: the definitions answered by the service that
// fills them.

/// A fresh, empty catalog folder under the temporary directory.
fn catalog_folder(label: &str) -> PathBuf {
    let mut root = yggdryl::local::LocalFolder::temporary()
        .expect("a temporary directory")
        .path()
        .expect("a path");
    root.push(format!(
        "yggdryl-xmla-definitions-{label}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("the folder is created");
    root
}

/// A provider over one empty catalog folder per name in `catalogs`.
fn provider(label: &str, catalogs: &[&str]) -> Service {
    catalogs
        .iter()
        .fold(Service::new(ServiceOptions::new()), |service, name| {
            let root = catalog_folder(&format!("{label}-{name}"));
            service.with_catalog(Catalog::new(
                *name,
                Holder::folder(&root).expect("the catalog holds"),
            ))
        })
}

/// The rows `service` answers a Discover of `request_type` with, read under
/// that request type's definition.
fn discovered(service: &Service, request_type: RequestType) -> Serie {
    let definition = answered(&request_type);
    let bytes = service
        .answer(&Request::from(Discover::new(request_type)), Vec::new())
        .expect("the answer is written");
    Response::from_bytes(&bytes, Some(definition.field()))
        .unwrap_or_else(|error| panic!("{error}\n{}", String::from_utf8_lossy(&bytes)))
        .rows()
        .expect("a rowset")
        .clone()
}

/// Cell `column` of row `index`.
fn cell_at(rows: &Serie, index: usize, column: usize) -> Scalar {
    rows.scalar(index)
        .expect("the row")
        .get(column)
        .expect("the cell")
        .into_owned()
}

#[test]
fn discover_schema_rowsets_states_each_definition_its_restrictions_and_its_description() {
    let service = provider("schema-rowsets", &["market"]);
    let rows = discovered(&service, RequestType::DiscoverSchemaRowsets);
    assert_eq!(rows.len(), definitions().len(), "one row per definition");
    for (index, definition) in definitions().iter().enumerate() {
        let request_type = definition.request_type();
        assert_eq!(
            cell_at(&rows, index, 0),
            Scalar::from(request_type.as_str()),
            "row {index} names {request_type}, in the definitions' order"
        );
        let restrictions = cell_at(&rows, index, 2);
        let stated: Vec<Scalar> = restrictions
            .sequence_rows()
            .expect("Restrictions is a sequence")
            .iter()
            .map(|restriction| restriction.get(0).expect("a Name").into_owned())
            .collect();
        let expected: Vec<Scalar> = restriction_names(definition)
            .into_iter()
            .map(Scalar::from)
            .collect();
        assert_eq!(stated, expected, "{request_type}'s restrictions");
        assert_eq!(
            cell_at(&rows, index, 3),
            Scalar::from(definition.description()),
            "{request_type} is described as its definition describes it"
        );
    }
}

#[test]
fn dbschema_catalogs_answers_one_row_per_root_folder_as_its_description_states() {
    let definition = answered(&RequestType::DbschemaCatalogs);
    assert!(definition.description().contains("one per root folder"));
    let service = provider("catalogs", &["market", "reference"]);
    let rows = discovered(&service, RequestType::DbschemaCatalogs);
    assert_eq!(rows.len(), 2, "{}", definition.description());
    assert_eq!(cell_at(&rows, 0, 0), Scalar::from("market"));
    assert_eq!(cell_at(&rows, 1, 0), Scalar::from("reference"));
}

#[test]
fn discover_datasources_answers_what_its_description_states() {
    // `DISCOVER_SCHEMA_ROWSETS` describes DISCOVER_DATASOURCES as "The data
    // sources this provider serves: one per catalog root."
    let definition = answered(&RequestType::DiscoverDatasources);
    let description = definition.description();
    let service = provider("datasources", &["market", "reference"]);
    let rows = discovered(&service, RequestType::DiscoverDatasources);
    let claims_one_per_root = description.contains("one per catalog root");
    assert!(
        !claims_one_per_root || rows.len() == 2,
        "{description:?}, yet a provider over two catalog roots answers {} data source row(s)",
        rows.len()
    );
}
