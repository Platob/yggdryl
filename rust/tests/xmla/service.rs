//! `rust/src/xmla/service.rs`: the provider answering Discover and Execute
//! over catalogs of record media.

#[cfg(feature = "http")]
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use arrow_array::{Float64Array, Int64Array, RecordBatch, StringArray};
use yggdryl::holder::Holder;
#[cfg(feature = "http")]
use yggdryl::http::{Request as WireRequest, Response as WireResponse};
#[cfg(feature = "http")]
use yggdryl::media::IORecordOptions;
use yggdryl::media::RecordOptions;
use yggdryl::soap::{Envelope, FaultCode, Fragment};
use yggdryl::xmla::{
    Answer, Command, Content, Discover, Execute, PropertyList, Request, RequestType, Response,
    Restrictions, Service, ServiceOptions, Session,
};
use yggdryl::{DataType, Field, FolderCatalog, IOBase, IOMedia, MimeType, Scalar, StructType};

/// A fresh catalog folder under the temporary directory, named after `label`.
fn catalog_root(label: &str) -> PathBuf {
    let mut root = yggdryl::local::LocalFolder::temporary()
        .expect("a temporary directory")
        .path()
        .expect("a path");
    root.push(format!(
        "yggdryl-xmla-{label}-{}-{}",
        std::process::id(),
        std::thread::current()
            .name()
            .unwrap_or("main")
            .replace("::", "-")
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
        FolderCatalog::bound("market", Holder::folder(&root).expect("the catalog holds"))
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
    assert_eq!(
        cell(&rows[0], "URL"),
        &Scalar::from("http://localhost:8080/xmla")
    );
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
    assert!(
        names.contains(&"MDSCHEMA_CUBES"),
        "the one multidimensional rowset the reference clients ask for before the tables"
    );
    assert!(
        !names.contains(&"MDSCHEMA_DIMENSIONS"),
        "a tabular provider answers no dimensions"
    );
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
    assert_eq!(
        cells(&several).len(),
        2,
        "a repeated restriction admits either"
    );

    let refused = fault(
        &service,
        Discover::new(RequestType::DbschemaTables)
            .with_restrictions(Restrictions::new().with("CUBE_NAME", "x")),
    );
    assert_eq!(refused.code(), &FaultCode::Client);
    assert!(
        refused.string().contains("CUBE_NAME"),
        "{}",
        refused.string()
    );
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
                cell(row, "COLUMN_NAME")
                    .as_str()
                    .expect("a name")
                    .to_owned(),
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
fn mdschema_cubes_projects_each_catalog_as_its_one_cube() {
    let service = service("cubes");
    let response = answer(&service, Discover::new(RequestType::MdschemaCubes));
    let rows = cells(&response);
    assert_eq!(rows.len(), 1, "one catalog, one cube");
    let cube = &rows[0];
    assert_eq!(cell(cube, "CATALOG_NAME"), &Scalar::from("market"));
    assert_eq!(cell(cube, "SCHEMA_NAME"), &Scalar::Null);
    assert_eq!(cell(cube, "CUBE_NAME"), &Scalar::from("market"));
    assert_eq!(cell(cube, "CUBE_TYPE"), &Scalar::from("CUBE"));
    assert_eq!(cell(cube, "CUBE_CAPTION"), &Scalar::from("market"));
    assert_eq!(
        cell(cube, "DESCRIPTION"),
        &Scalar::from("the market catalog")
    );
    assert_eq!(cell(cube, "IS_DRILLTHROUGH_ENABLED"), &Scalar::from(true));
    assert_eq!(cell(cube, "IS_LINKABLE"), &Scalar::from(false));
    assert_eq!(
        cell(cube, "IS_WRITE_ENABLED"),
        &Scalar::from(false),
        "the service was not made writable"
    );
    assert_eq!(cell(cube, "IS_SQL_ENABLED"), &Scalar::from(true));
    assert_eq!(cell(cube, "CUBE_SOURCE"), &Scalar::from(1_u16));
    assert_eq!(cell(cube, "PREFERRED_QUERY_PATTERNS"), &Scalar::from(0_u16));
    assert_eq!(
        cell(cube, "LAST_SCHEMA_UPDATE"),
        cell(cube, "LAST_DATA_UPDATE"),
        "both dates are the catalog's modification"
    );
    for absent in [
        "CUBE_GUID",
        "CREATED_ON",
        "SCHEMA_UPDATED_BY",
        "DATA_UPDATED_BY",
        "BASE_CUBE_NAME",
    ] {
        assert_eq!(cell(cube, absent), &Scalar::Null, "{absent}");
    }

    // The restrictions the reference clients send: the catalog and the cube
    // by name, and the cube's source.
    let named = answer(
        &service,
        Discover::new(RequestType::MdschemaCubes).with_restrictions(
            Restrictions::new()
                .with("CATALOG_NAME", "market")
                .with("CUBE_NAME", "market")
                .with("CUBE_SOURCE", "1"),
        ),
    );
    assert_eq!(cells(&named).len(), 1);
    let other = answer(
        &service,
        Discover::new(RequestType::MdschemaCubes)
            .with_restrictions(Restrictions::new().with("CUBE_NAME", "Model")),
    );
    assert!(cells(&other).is_empty(), "no cube is named Model");
    let dimensions = answer(
        &service,
        Discover::new(RequestType::MdschemaCubes)
            .with_restrictions(Restrictions::new().with("CUBE_SOURCE", "2")),
    );
    assert!(
        cells(&dimensions).is_empty(),
        "no table is a cube of its own"
    );
}

#[test]
fn an_unsupported_request_type_is_a_client_fault_naming_the_rowset() {
    let service = service("unsupported");
    let refused = fault(&service, Discover::new(RequestType::MdschemaDimensions));
    assert_eq!(refused.code(), &FaultCode::Client);
    assert!(
        refused.string().contains("MDSCHEMA_DIMENSIONS"),
        "{}",
        refused.string()
    );
    let errors = yggdryl::xmla::XmlaError::from_fault(&refused);
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].source(), "yggdryl");
}

#[test]
fn execute_runs_a_statement_against_a_catalog_table() {
    let service = service("execute");
    let response = answer(
        &service,
        Execute::statement(
            "select symbol, price from trades where price > 150 order by price desc",
        )
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
fn a_join_source_resolves_against_the_catalog_as_from_does() {
    let service = service("join");
    // `trades` and the dotted `market.eu.fills` both resolve to the leaves
    // the catalog holds: the join reads the second through its own location.
    let response = answer(
        &service,
        Execute::statement(
            "select symbol, price, size_right from trades \
             join market.eu.fills using (symbol) where price > 150 order by price desc",
        )
        .with_properties(PropertyList::new().with("Catalog", "market")),
    );
    let rows = cells(&response);
    assert_eq!(rows.len(), 2);
    assert_eq!(cell(&rows[0], "symbol"), &Scalar::from("MSFT"));
    assert_eq!(cell(&rows[1], "symbol"), &Scalar::from("AAPL"));
    assert_eq!(cell(&rows[1], "size_right"), &Scalar::from(100_i64));
    // A join source inside a nested plan resolves the same way.
    let nested = answer(
        &service,
        Execute::statement(
            "select symbol from trades \
             semi join (select symbol from eu.fills where size is null) using (symbol)",
        )
        .with_properties(PropertyList::new().with("Catalog", "market")),
    );
    let rows = cells(&nested);
    assert_eq!(rows.len(), 1);
    assert_eq!(cell(&rows[0], "symbol"), &Scalar::from("MSFT"));
    // A join source is refused as a `from` is: an unknown table, and a URL
    // outside every catalog, by name.
    let unknown = fault(
        &service,
        Execute::statement("select * from trades join nowhere using (symbol)")
            .with_properties(PropertyList::new().with("Catalog", "market")),
    );
    assert_eq!(unknown.code(), &FaultCode::Client);
    assert!(unknown.string().contains("nowhere"), "{}", unknown.string());
    let outside = fault(
        &service,
        Execute::statement("select * from trades join 'file:///etc/passwd' using (symbol)"),
    );
    assert!(outside.string().contains("passwd"), "{}", outside.string());
}

#[test]
fn a_url_beside_a_served_catalog_is_outside_it() {
    // A folder whose name only begins with the catalog's is a sibling, not a
    // child: the catalog holds a URL on a path boundary or not at all.
    let root = catalog_root("sibling");
    seed(&root);
    let service = Service::new(ServiceOptions::new()).with_catalog(FolderCatalog::bound(
        "market",
        Holder::folder(&root).expect("the catalog holds"),
    ));
    let mut sibling = root.clone().into_os_string();
    sibling.push("-evil");
    let sibling = PathBuf::from(sibling);
    let _ = std::fs::remove_dir_all(&sibling);
    let mut leaf = Holder::folder(&sibling)
        .expect("the sibling holds")
        .child_by_path("secrets.arrows")
        .expect("a child path");
    let batch = trades_batch();
    leaf.overwrite_arrow_reader(
        yggdryl::arrow::batch_reader(batch.schema(), [batch]),
        &RecordOptions::for_mime_type(&MimeType::ARROW_STREAM).expect("IPC options"),
    )
    .expect("the sibling's leaf is written");
    let url = yggdryl::Url::from_path(sibling.join("secrets.arrows")).expect("a URL");
    for statement in [
        format!("select * from '{url}'"),
        format!("select * from trades join '{url}' using (symbol)"),
    ] {
        let outside = fault(
            &service,
            Execute::statement(statement.as_str())
                .with_properties(PropertyList::new().with("Catalog", "market")),
        );
        assert!(
            outside.string().contains("-evil"),
            "{statement}: {}",
            outside.string()
        );
    }
    let _ = std::fs::remove_dir_all(&sibling);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn execute_refuses_what_it_cannot_run_by_name() {
    let service = service("refusals");
    let unknown = fault(
        &service,
        Execute::statement("select * from nowhere")
            .with_properties(PropertyList::new().with("Catalog", "market")),
    );
    assert_eq!(unknown.code(), &FaultCode::Client);
    assert!(unknown.string().contains("nowhere"), "{}", unknown.string());

    let write = fault(
        &service,
        Execute::statement("insert into trades select * from trades"),
    );
    assert!(write.string().contains("read-only"), "{}", write.string());

    let parse = fault(&service, Execute::statement("selec * frm trades"));
    assert_eq!(parse.code(), &FaultCode::Client);

    let outside = fault(
        &service,
        Execute::statement("select * from 'file:///etc/passwd'"),
    );
    assert!(outside.string().contains("passwd"), "{}", outside.string());

    let multidimensional = fault(
        &service,
        Execute::statement("select * from trades")
            .with_properties(PropertyList::new().with("Format", "Multidimensional")),
    );
    assert!(
        multidimensional.string().contains("Tabular"),
        "{}",
        multidimensional.string()
    );
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

// --- Sessions, and the properties the reference clients read by name ----------

#[test]
fn a_cancel_answers_empty_since_no_command_is_ever_left_running() {
    // ADOMD.NET cancels on a pooled connection before reusing it, with the
    // engine namespace; every Execute here is answered before the next
    // request on its connection is read, so there is nothing to cancel.
    let service = service("cancel");
    for cancel in [
        Fragment::new("Cancel", Scalar::Null),
        Fragment::in_namespace(
            "Cancel",
            "http://schemas.microsoft.com/analysisservices/2003/engine",
            Scalar::Null,
        )
        .expect("a namespaced element"),
    ] {
        let response = answer(&service, Execute::new(Command::Other(cancel)));
        assert_eq!(response.answer(), &Answer::Empty);
    }
    // Any other command is still refused by name.
    let refused = fault(
        &service,
        Execute::new(Command::Other(Fragment::new("Alter", Scalar::Null))),
    );
    assert!(refused.string().contains("`Alter`"), "{}", refused.string());
}

#[test]
fn a_session_opens_on_an_execute_carrying_begin_session_and_an_empty_statement() {
    // The first message every MSOLAP and ADOMD.NET client sends.
    let service = service("session-opens");
    let request = Request::from(Execute::statement(""))
        .with_session(&Session::Begin)
        .expect("a session block");
    let bytes = service.answer(&request, Vec::new()).expect("answered");
    let text = String::from_utf8_lossy(&bytes);
    assert!(
        text.contains("<Session SessionId=\"")
            && text.contains("xmlns=\"urn:schemas-microsoft-com:xml-analysis\"/>"),
        "{text}"
    );
    let response = Response::from_bytes(&bytes, None).expect("a response");
    assert_eq!(response.answer(), &Answer::Empty);
    assert_eq!(response.header().len(), 1);
}

#[test]
fn a_fault_carries_the_session_block_the_request_earned() {
    let service = service("session-fault");
    let request = Request::from(Discover::new(RequestType::MdschemaDimensions))
        .with_session(&Session::Begin)
        .expect("a session block");
    let bytes = service.answer(&request, Vec::new()).expect("answered");
    let envelope = Envelope::from_bytes(&bytes).expect("an envelope");
    assert!(envelope.fault().is_some());
    assert!(
        envelope
            .header()
            .iter()
            .any(|block| block.name() == "Session"),
        "{}",
        String::from_utf8_lossy(&bytes)
    );
}

#[test]
fn discover_schema_rowsets_states_each_rowset_guid_and_restrictions_mask() {
    let service = service("schema-rowsets");
    let response = answer(&service, Discover::new(RequestType::DiscoverSchemaRowsets));
    let rows = cells(&response);
    assert_eq!(rows.len(), 12);
    for row in &rows {
        let restrictions = cell(row, "Restrictions")
            .sequence_rows()
            .map_or(0, |items| items.len());
        assert_ne!(cell(row, "SchemaGuid"), &Scalar::Null, "{row:?}");
        assert_eq!(
            cell(row, "RestrictionsMask"),
            &Scalar::from((1_u64 << restrictions) - 1),
            "{row:?}"
        );
    }
    let tables = rows
        .iter()
        .find(|row| cell(row, "SchemaName") == &Scalar::from("DBSCHEMA_TABLES"))
        .expect("DBSCHEMA_TABLES");
    assert_eq!(
        cell(tables, "SchemaGuid"),
        &DataType::Uuid
            .scalar("c8b52229-5cf3-11ce-ade5-00aa0044773d")
            .expect("a uuid")
    );
    assert_eq!(cell(tables, "RestrictionsMask"), &Scalar::from(15_u64));
}

#[test]
fn the_properties_the_reference_clients_read_by_name_are_answered() {
    let service = service("client-properties");
    let by_name = |name: &str, properties: PropertyList| {
        let response = answer(
            &service,
            Discover::new(RequestType::DiscoverProperties)
                .with_restrictions(Restrictions::new().with("PropertyName", name))
                .with_properties(properties),
        );
        let rows = cells(&response);
        assert_eq!(rows.len(), 1, "{name}");
        (
            cell(&rows[0], "PropertyType").clone(),
            cell(&rows[0], "PropertyAccessType").clone(),
            cell(&rows[0], "Value").clone(),
        )
    };
    let none = PropertyList::new;
    assert_eq!(
        by_name("ProviderType", none()),
        (Scalar::from("int"), Scalar::from("Read"), Scalar::from("1"))
    );
    assert_eq!(
        by_name("MDXSupport", none()),
        (
            Scalar::from("string"),
            Scalar::from("Read"),
            Scalar::from("Core")
        )
    );
    assert_eq!(by_name("ServerName", none()).2, Scalar::from("yggdryl"));
    // ADOMD.NET reads DBMSVersion first and refuses a server it parses as
    // older than SQL Server 2008 R2 RTM; the provider's own version stays
    // ProviderVersion's.
    assert_eq!(
        by_name("DBMSVersion", none()).2,
        Scalar::from("10.50.1600.1")
    );
    assert_eq!(
        by_name("ProviderVersion", none()).2,
        Scalar::from(env!("CARGO_PKG_VERSION"))
    );
    assert_eq!(by_name("SQLSupport", none()).2, Scalar::from("512"));
    assert_eq!(
        by_name("MdpropMdxSubqueries", none()),
        (Scalar::from("int"), Scalar::from("Read"), Scalar::from("0"))
    );
    assert_eq!(
        by_name("MdpropMdxQueryByProperty", none()),
        (
            Scalar::from("boolean"),
            Scalar::from("Read"),
            Scalar::from("false")
        )
    );
    // A property the client states about itself is echoed, else its own
    // default - and where it has none, the cell is absent rather than an
    // empty text under `int`, which is what stopped MSOLAP's wizard.
    assert_eq!(
        by_name("DbpropMsmdMDXCompatibility", none()).2,
        Scalar::from("0")
    );
    assert_eq!(
        by_name(
            "DbpropMsmdMDXCompatibility",
            PropertyList::new().with("DbpropMsmdMDXCompatibility", "1")
        )
        .2,
        Scalar::from("1")
    );
    assert_eq!(
        by_name("MdxMissingMemberMode", none()).2,
        Scalar::from("Default")
    );
    assert_eq!(
        by_name("ClientProcessID", none()),
        (Scalar::from("int"), Scalar::from("ReadWrite"), Scalar::Null)
    );
    assert_eq!(
        by_name(
            "ClientProcessID",
            PropertyList::new().with("ClientProcessID", "25304")
        )
        .2,
        Scalar::from("25304")
    );
    for echoed in [
        "ApplicationContext",
        "DbpropMsmdActivityID",
        "DbpropMsmdCurrentActivityID",
    ] {
        assert_eq!(
            by_name(echoed, none()),
            (
                Scalar::from("string"),
                Scalar::from("ReadWrite"),
                Scalar::Null
            ),
            "{echoed}"
        );
    }
    assert_eq!(by_name("Timeout", none()).2, Scalar::Null);
    assert_eq!(by_name("LocaleIdentifier", none()).2, Scalar::Null);
    // The catalog a request that names none is read against: the first served.
    assert_eq!(by_name("Catalog", none()).2, Scalar::from("market"));
}

#[test]
fn a_restriction_sent_with_no_value_restricts_nothing() {
    let service = service("empty-restriction");
    let all = cells(&answer(
        &service,
        Discover::new(RequestType::DbschemaTables),
    ));
    let restricted = cells(&answer(
        &service,
        Discover::new(RequestType::DbschemaTables)
            .with_restrictions(Restrictions::new().with("TABLE_NAME", "")),
    ));
    assert!(all.len() >= 2, "{all:?}");
    assert_eq!(restricted.len(), all.len());
}

// --- The reference clients' doors --------------------------------------------
//
// `fixtures/excel/<door>/` holds what one Excel door sent to `yggdryl xmla serve
// --trace` over `C:\data\market` and what it was answered, as it went over the
// wire: `wizard` is MSOLAP's Data Connection Wizard up to the saved `.odc`,
// `pivottable` the PivotTable import that follows it, `pq-query` Power Query's
// `AnalysisServices.Database` with a query. Each request is read off its wire
// file by the crate's own HTTP reader and replayed through the service; the
// answer is checked against the captured one by what a client reads - the
// kind of answer, the columns, the number of rows, the fault code - never by
// its bytes, which carry a session id and a date.

/// The catalog the doors were captured over: `market` with `trades`,
/// `trades_100k`, `trades_1m` and `types` at the root and `venues` under the
/// `reference` schema. The capture read Iceberg folders; the doors list tables
/// and read `trades`, so IPC tables of the captured `trades` columns stand in,
/// with as many rows as the query the door sent asked for.
#[cfg(feature = "http")]
fn excel_service(label: &str) -> Service {
    let root = catalog_root(label);
    let field = StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::DateTime64 {
            unit: yggdryl::TimeUnit::Microsecond,
            timezone: yggdryl::Timezone::UTC,
        }
        .required_field("at"),
        DataType::utf8().required_field("symbol"),
        DataType::utf8().required_field("venue"),
        DataType::utf8().required_field("side"),
        DataType::Decimal128 {
            precision: 18,
            scale: 4,
        }
        .required_field("price"),
        DataType::Int32.required_field("qty"),
        DataType::Float64.nullable_field("notional"),
        DataType::Boolean.required_field("live"),
    ])
    .map(DataType::from)
    .expect("a valid root")
    .required_field("row");
    let rows: Vec<Scalar> = (0..100_i64)
        .map(|i| {
            Scalar::from_sequence([
                Scalar::from(i),
                Scalar::from(1_700_000_000_000_000_i64 + i),
                Scalar::from("AAPL"),
                Scalar::from("XNAS"),
                Scalar::from(if i % 2 == 0 { "buy" } else { "sell" }),
                Scalar::from(1_234_567_i128),
                Scalar::from(100_i32),
                Scalar::Null,
                Scalar::from(true),
            ])
        })
        .collect();
    let options = RecordOptions::for_mime_type(&MimeType::ARROW_STREAM)
        .expect("IPC options")
        .with_field(field);
    for path in [
        "trades.arrows",
        "trades_100k.arrows",
        "trades_1m.arrows",
        "types.arrows",
        "reference/venues.arrows",
    ] {
        let mut leaf = Holder::folder(&root)
            .expect("the root holds")
            .child_by_path(path)
            .expect("the child resolves");
        leaf.overwrite_records(rows.clone(), &options)
            .expect("the table is written");
    }
    Service::new(ServiceOptions::new().with_url("http://127.0.0.1:8080/xmla")).with_catalog(
        FolderCatalog::bound("market", Holder::folder(&root).expect("holds")),
    )
}

/// One door's exchanges - number, request wire, response wire - in the order
/// they went over the wire.
#[cfg(feature = "http")]
fn door(name: &str) -> Vec<(String, Vec<u8>, Vec<u8>)> {
    let folder = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/xmla/fixtures/excel")
        .join(name);
    let mut numbers: Vec<String> = std::fs::read_dir(&folder)
        .unwrap_or_else(|error| panic!("{}: {error}", folder.display()))
        .map(|entry| {
            entry
                .expect("an entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .filter_map(|file| file.strip_suffix("-request.http").map(str::to_owned))
        .collect();
    numbers.sort();
    assert!(!numbers.is_empty(), "{name} holds exchanges");
    numbers
        .into_iter()
        .map(|number| {
            let read = |side: &str| {
                let path = folder.join(format!("{number}-{side}.http"));
                std::fs::read(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
            };
            (number.clone(), read("request"), read("response"))
        })
        .collect()
}

/// The session id a message's header names, when it names one.
#[cfg(feature = "http")]
fn session_id(header: &[Fragment]) -> Option<String> {
    header
        .iter()
        .find(|block| block.name().rsplit(':').next() == Some("Session"))
        .and_then(|block| {
            block
                .element()
                .attribute("SessionId")
                .and_then(Scalar::as_str)
                .map(str::to_owned)
        })
}

/// What a client reads off an answer: the fault's code, or the answer's kind
/// with its columns and its number of rows.
#[cfg(feature = "http")]
fn shape(bytes: &[u8]) -> String {
    let envelope = Envelope::from_bytes(bytes)
        .unwrap_or_else(|error| panic!("{error}\n{}", String::from_utf8_lossy(bytes)));
    if let Some(fault) = envelope.fault() {
        let errors = yggdryl::xmla::XmlaError::from_fault(fault);
        return format!(
            "fault {:#06x}",
            errors.first().map_or(0, |error| error.code())
        );
    }
    let response = Response::from_envelope(&envelope, None)
        .unwrap_or_else(|error| panic!("{error}\n{}", String::from_utf8_lossy(bytes)));
    match response.answer() {
        Answer::Empty => "empty".to_owned(),
        Answer::Rowset { rowset, rows } => format!(
            "{} rows of {}",
            rows.len(),
            rowset
                .field()
                .fields()
                .iter()
                .map(Field::name)
                .collect::<Vec<_>>()
                .join(",")
        ),
        other => format!("{other:?}"),
    }
}

/// Replay `doors` in order through one service - one process served them all,
/// so a session one door opened is the one the next closes - each captured
/// session id standing for the live one it became.
#[cfg(feature = "http")]
fn replay(service: &Service, doors: &[&str]) {
    let mut sessions: HashMap<String, String> = HashMap::new();
    for name in doors {
        for (number, wire_request, wire_response) in door(name) {
            let read = WireRequest::from_bytes(&wire_request)
                .unwrap_or_else(|error| panic!("{name}/{number}: {error}"));
            // Past the interim `100 Continue` ADOMD.NET's requests earn.
            let captured = WireResponse::from_bytes(&wire_response)
                .unwrap_or_else(|error| panic!("{name}/{number}: {error}"))
                .bytes()
                .unwrap_or_else(|error| panic!("{name}/{number}: {error}"));
            let mut body = String::from_utf8(read.body().as_bytes().to_vec()).expect("UTF-8");
            for (was, is) in &sessions {
                body = body.replace(was.as_str(), is);
            }
            let answered = service
                .handle(body.as_bytes(), Vec::new())
                .unwrap_or_else(|error| panic!("{name}/{number}: {error}"));
            let opened = Envelope::from_bytes(&answered).ok().and_then(|envelope| {
                let was = Envelope::from_bytes(&captured).ok()?;
                Some((session_id(was.header())?, session_id(envelope.header())?))
            });
            if body.contains("<BeginSession") {
                let (was, is) =
                    opened.unwrap_or_else(|| panic!("{name}/{number}: a session opens"));
                sessions.insert(was, is);
            }
            assert_eq!(
                shape(&answered),
                shape(&captured),
                "{name}/{number}: the replay answers what the capture was answered\n{}",
                String::from_utf8_lossy(&answered)
            );
        }
    }
}

#[cfg(feature = "http")]
#[test]
fn excels_data_connection_wizard_and_its_pivottable_replay_as_captured() {
    let service = excel_service("excel-wizard");
    replay(&service, &["wizard", "pivottable"]);
}

#[cfg(feature = "http")]
#[test]
fn power_querys_analysis_services_database_with_a_query_replays_as_captured() {
    // ADOMD.NET: every request `Expect: 100-continue` and chunked, the body
    // opening with a byte-order mark; the query text passed through as the
    // statement under `Format=Tabular`, and a `<Cancel/>` per pooled
    // connection before it is reused. `refresh` is the same query refreshed
    // from the sheet against a restarted provider: it opens by closing the
    // session the pool still held, which the provider never opened.
    let service = excel_service("excel-pq-query");
    replay(&service, &["pq-query", "refresh"]);
}

#[cfg(feature = "http")]
#[test]
fn power_querys_navigator_and_a_dax_text_are_refused_as_captured() {
    // Without a query, Power Query runs a DMV query - `select ... from
    // $system.mdschema_cubes where [CUBE_SOURCE] = 1` - and a DAX text is
    // `EVALUATE 'trades'`; both are refused by the grammar at the byte it
    // stopped at, and the client shows the refusal. The doors are kept so a
    // provider that comes to answer them is measured against what they send.
    let service = excel_service("excel-pq-refused");
    replay(&service, &["pq-navigator", "pq-dax"]);
}

#[cfg(feature = "http")]
#[test]
fn an_odc_with_a_command_text_loads_a_table_through_msolap_as_captured() {
    // `Provider=MSOLAP;...;Initial Catalog=market;` with `CommandType Query`:
    // MSOLAP opens a session, reads the catalog and six properties, runs the
    // command text under `Format=Tabular` and closes the session; a refresh
    // is the same five requests again.
    let service = excel_service("excel-odc");
    replay(&service, &["odc-query"]);
}
