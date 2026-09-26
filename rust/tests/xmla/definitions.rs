//! `rust/src/xmla/definitions.rs`: the rowsets the provider answers - one
//! definition per request type, each the record field its rows are, the
//! columns a Discover may restrict it by, and the description
//! `DISCOVER_SCHEMA_ROWSETS` states for it.

use std::sync::Arc;

use yggdryl::xmla::definitions::{Definition, definition_of, definitions};
use yggdryl::xmla::{RequestType, Rowset, XsdType, decode_name, encode_name};
use yggdryl::{DataType, Field, Scalar, Serie, StructType, TimeUnit, Timezone};

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
            assert_eq!(
                back.is_nullable(),
                declared.is_nullable(),
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
