//! `rust/src/xmla/service.rs`: the provider answering Discover and Execute
//! over catalogs of record media.

use std::path::PathBuf;
use std::sync::Arc;

use arrow_array::{Float64Array, Int64Array, RecordBatch, StringArray};
use yggdryl::holder::Holder;
use yggdryl::media::RecordOptions;
use yggdryl::soap::{Envelope, FaultCode};
use yggdryl::xmla::{
    Catalog, Content, Discover, Execute, PropertyList, Request, RequestType, Response,
    Restrictions, Service, ServiceOptions,
};
use yggdryl::{DataType, Field, IOBase, IOMedia, MimeType, Scalar, StructType};

/// A fresh catalog folder under the temporary directory, named after `label`.
fn catalog_root(label: &str) -> PathBuf {
    let mut root = yggdryl::local::LocalFolder::temporary()
        .expect("a temporary directory")
        .path()
        .expect("a path");
    root.push(format!(
        "yggdryl-xmla-{label}-{}-{}",
        std::process::id(),
        std::thread::current().name().unwrap_or("main").replace("::", "-")
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("the folder is created");
    root
}

/// The trades table: three rows of `symbol`, `price`, `size`.
fn trades_field() -> Field {
    StructType::from_fields([
        DataType::utf8().required_field("symbol"),
        DataType::Float64.required_field("price"),
        DataType::Int64.nullable_field("size"),
    ])
    .map(DataType::from)
    .expect("a valid root")
    .required_field("row")
}

fn trades_batch() -> RecordBatch {
    let schema = trades_field().into_arrow_schema().expect("an Arrow schema");
    RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from(vec!["AAPL", "MSFT", "GOOG"])),
            Arc::new(Float64Array::from(vec![187.5, 410.25, 141.0])),
            Arc::new(Int64Array::from(vec![Some(100), None, Some(25)])),
        ],
    )
    .expect("a batch")
}

/// Write `trades.arrows` under `root`, and an `eu/` schema folder holding
/// `fills.arrows` with the same rows.
fn seed(root: &std::path::Path) {
    for path in ["trades.arrows", "eu/fills.arrows"] {
        let mut leaf = Holder::folder(root)
            .expect("the root holds")
            .child_by_path(path)
            .expect("the child resolves");
        let options = RecordOptions::for_mime_type(&MimeType::ARROW_STREAM).expect("IPC options");
        let batch = trades_batch();
        leaf.overwrite_arrow_reader(
            yggdryl::arrow::batch_reader(batch.schema(), [batch]),
            &options,
        )
        .expect("the table is written");
    }
    std::fs::write(root.join("README.md"), "not a table\n").expect("a stray file");
}

/// A provider over one seeded catalog named `market`.
fn service(label: &str) -> Service {
    let root = catalog_root(label);
    seed(&root);
    Service::new(ServiceOptions::new().with_url("http://localhost:8080/xmla")).with_catalog(
        Catalog::new("market", Holder::folder(&root).expect("the catalog holds"))
            .with_description("the market catalog"),
    )
}

/// Answer `request` and read the response back, a fault panicking with its
/// text.
fn answer(service: &Service, request: impl Into<Request>) -> Response {
    let bytes = service
        .answer(&request.into(), Vec::new())
        .expect("the answer is written");
    Response::from_bytes(&bytes, None)
        .unwrap_or_else(|error| panic!("{error}\n{}", String::from_utf8_lossy(&bytes)))
}

/// Answer `request` and read the fault it earns.
fn fault(service: &Service, request: impl Into<Request>) -> yggdryl::soap::Fault {
    let bytes = service
        .answer(&request.into(), Vec::new())
        .expect("the answer is written");
    Envelope::from_bytes(&bytes)
        .expect("an envelope")
        .fault()
        .cloned()
        .unwrap_or_else(|| panic!("expected a fault:\n{}", String::from_utf8_lossy(&bytes)))
}

/// The rows of a rowset as `(column, text)` pairs per row.
fn cells(response: &Response) -> Vec<Vec<(String, Scalar)>> {
    let rows = response.rows().expect("a rowset");
    let field = response.rowset().expect("a rowset").field().clone();
    (0..rows.len())
        .map(|index| {
            let row = rows.get(index).expect("a row").into_owned();
            let natural = field.into_natural_value(row).expect("a named row");
            natural
                .as_struct()
                .expect("a record")
                .iter()
                .map(|(name, value)| (name.to_string(), value.clone()))
                .collect()
        })
        .collect()
}

fn cell<'a>(row: &'a [(String, Scalar)], column: &str) -> &'a Scalar {
    &row.iter().find(|(name, _)| name == column).expect(column).1
}

#[test]
fn discover_datasources_states_one_tabular_data_source() {
    let service = service("datasources");
    let response = answer(&service, Discover::new(RequestType::DiscoverDatasources));
    let rows = cells(&response);
    assert_eq!(rows.len(), 1);
    assert_eq!(cell(&rows[0], "DataSourceName"), &Scalar::from("yggdryl"));
    assert_eq!(cell(&rows[0], "URL"), &Scalar::from("http://localhost:8080/xmla"));
    assert_eq!(
        cell(&rows[0], "ProviderType"),
        &Scalar::from_sequence([Scalar::from("TDP")])
    );
    assert_eq!(
        cell(&rows[0], "AuthenticationMode"),
        &Scalar::from("Unauthenticated")
    );
}

#[test]
fn discover_schema_rowsets_lists_every_definition_with_its_restrictions() {
    let service = service("schema-rowsets");
    let response = answer(&service, Discover::new(RequestType::DiscoverSchemaRowsets));
    let rows = cells(&response);
    let names: Vec<&str> = rows
        .iter()
        .map(|row| cell(row, "SchemaName").as_str().expect("text"))
        .collect();
    assert!(names.contains(&"DISCOVER_DATASOURCES"));
    assert!(names.contains(&"DBSCHEMA_TABLES"));
    assert!(names.contains(&"DBSCHEMA_COLUMNS"));
    assert!(!names.contains(&"MDSCHEMA_CUBES"), "a tabular provider answers no cubes");
    let tables = rows
        .iter()
        .find(|row| cell(row, "SchemaName").as_str() == Some("DBSCHEMA_TABLES"))
        .expect("the tables rowset");
    let restrictions = cell(tables, "Restrictions")
        .sequence_rows()
        .expect("a sequence")
        .into_owned();
    let restricted: Vec<String> = restrictions
        .iter()
        .map(|entry| {
            entry
                .get_key_str("Name")
                .and_then(Scalar::as_str)
                .expect("a name")
                .to_owned()
        })
        .collect();
    assert_eq!(
        restricted,
        ["TABLE_CATALOG", "TABLE_SCHEMA", "TABLE_NAME", "TABLE_TYPE"]
    );
}

#[test]
fn dbschema_tables_lists_the_leaves_and_the_schema_folder_tables() {
    let service = service("tables");
    let response = answer(&service, Discover::new(RequestType::DbschemaTables));
    let rows = cells(&response);
    let mut tables: Vec<(Option<String>, String)> = rows
        .iter()
        .map(|row| {
            (
                cell(row, "TABLE_SCHEMA").as_str().map(str::to_owned),
                cell(row, "TABLE_NAME").as_str().expect("a name").to_owned(),
            )
        })
        .collect();
    tables.sort();
    assert_eq!(
        tables,
        [
            (None, "trades".to_owned()),
            (Some("eu".to_owned()), "fills".to_owned()),
        ]
    );
    for row in &rows {
        assert_eq!(cell(row, "TABLE_CATALOG"), &Scalar::from("market"));
        assert_eq!(cell(row, "TABLE_TYPE"), &Scalar::from("TABLE"));
        assert!(
            !cell(row, "DATE_MODIFIED").is_null(),
            "a local file states its modification time"
        );
    }
}

#[test]
fn a_restriction_narrows_the_rowset_and_an_unknown_one_is_a_client_fault() {
    let service = service("restrictions");
    let response = answer(
        &service,
        Discover::new(RequestType::DbschemaTables)
            .with_restrictions(Restrictions::new().with("TABLE_NAME", "fills")),
    );
    let rows = cells(&response);
    assert_eq!(rows.len(), 1);
    assert_eq!(cell(&rows[0], "TABLE_SCHEMA"), &Scalar::from("eu"));

    let several = answer(
        &service,
        Discover::new(RequestType::DbschemaTables).with_restrictions(
            Restrictions::new()
                .with("TABLE_NAME", "fills")
                .with("TABLE_NAME", "trades"),
        ),
    );
    assert_eq!(cells(&several).len(), 2, "a repeated restriction admits either");

    let refused = fault(
        &service,
        Discover::new(RequestType::DbschemaTables)
            .with_restrictions(Restrictions::new().with("CUBE_NAME", "x")),
    );
    assert_eq!(refused.code(), &FaultCode::Client);
    assert!(refused.string().contains("CUBE_NAME"), "{}", refused.string());
}

#[test]
fn dbschema_columns_types_a_table_as_ole_db_does() {
    let service = service("columns");
    let response = answer(
        &service,
        Discover::new(RequestType::DbschemaColumns)
            .with_restrictions(Restrictions::new().with("TABLE_NAME", "trades")),
    );
    let rows = cells(&response);
    let columns: Vec<(String, Scalar, Scalar, Scalar)> = rows
        .iter()
        .map(|row| {
            (
                cell(row, "COLUMN_NAME").as_str().expect("a name").to_owned(),
                cell(row, "DATA_TYPE").clone(),
                cell(row, "IS_NULLABLE").clone(),
                cell(row, "ORDINAL_POSITION").clone(),
            )
        })
        .collect();
    assert_eq!(
        columns,
        [
            (
                "symbol".to_owned(),
                Scalar::from(130_u16),
                Scalar::from(false),
                Scalar::from(1_u32)
            ),
            (
                "price".to_owned(),
                Scalar::from(5_u16),
                Scalar::from(false),
                Scalar::from(2_u32)
            ),
            (
                "size".to_owned(),
                Scalar::from(20_u16),
                Scalar::from(true),
                Scalar::from(3_u32)
            ),
        ]
    );
}

#[test]
fn an_unsupported_request_type_is_a_client_fault_naming_the_rowset() {
    let service = service("unsupported");
    let refused = fault(&service, Discover::new(RequestType::MdschemaCubes));
    assert_eq!(refused.code(), &FaultCode::Client);
    assert!(refused.string().contains("MDSCHEMA_CUBES"), "{}", refused.string());
    let errors = yggdryl::xmla::XmlaError::from_fault(&refused);
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].source(), "yggdryl");
}

#[test]
fn execute_runs_a_statement_against_a_catalog_table() {
    let service = service("execute");
    let response = answer(
        &service,
        Execute::statement("select symbol, price from trades where price > 150 order by price desc")
            .with_properties(PropertyList::new().with("Catalog", "market")),
    );
    let rows = cells(&response);
    assert_eq!(rows.len(), 2);
    assert_eq!(cell(&rows[0], "symbol"), &Scalar::from("MSFT"));
    assert_eq!(cell(&rows[1], "symbol"), &Scalar::from("AAPL"));
    assert_eq!(cell(&rows[0], "price"), &Scalar::from(410.25_f64));

    // The one catalog needs no Catalog property, and a dotted path names it.
    let dotted = answer(
        &service,
        Execute::statement("select size from market.eu.fills where size is not null"),
    );
    assert_eq!(cells(&dotted).len(), 2);
}

#[test]
fn execute_refuses_what_it_cannot_run_by_name() {
    let service = service("refusals");
    let unknown = fault(
        &service,
        Execute::statement("select * from nowhere").with_properties(PropertyList::new().with("Catalog", "market")),
    );
    assert_eq!(unknown.code(), &FaultCode::Client);
    assert!(unknown.string().contains("nowhere"), "{}", unknown.string());

    let write = fault(&service, Execute::statement("insert into trades select * from trades"));
    assert!(write.string().contains("read-only"), "{}", write.string());

    let parse = fault(&service, Execute::statement("selec * frm trades"));
    assert_eq!(parse.code(), &FaultCode::Client);

    let outside = fault(&service, Execute::statement("select * from 'file:///etc/passwd'"));
    assert!(outside.string().contains("passwd"), "{}", outside.string());

    let multidimensional = fault(
        &service,
        Execute::statement("select * from trades")
            .with_properties(PropertyList::new().with("Format", "Multidimensional")),
    );
    assert!(multidimensional.string().contains("Tabular"), "{}", multidimensional.string());
}

#[test]
fn content_none_verifies_and_answers_the_empty_root() {
    let service = service("content-none");
    let response = answer(
        &service,
        Execute::statement("select * from trades")
            .with_properties(PropertyList::new().with("Content", Content::None.as_str())),
    );
    assert!(response.rows().is_none());
    assert!(matches!(response.answer(), yggdryl::xmla::Answer::Empty));
}

#[test]
fn a_message_that_is_not_a_request_is_a_client_fault_over_handle() {
    let service = service("handle");
    let bytes = service
        .handle(b"<not-soap/>", Vec::new())
        .expect("the fault is written");
    let envelope = Envelope::from_bytes(&bytes).expect("an envelope");
    let fault = envelope.fault().expect("a fault");
    assert_eq!(fault.code(), &FaultCode::Client);
}
